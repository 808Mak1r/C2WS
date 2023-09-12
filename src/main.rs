use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc},
};

use axum::{
    extract::{
        ws::{Message, WebSocket},
        ConnectInfo, State, WebSocketUpgrade,
    },
    response::Response,
    routing::get,
    Router, ServiceExt,
};
use futures::{
    stream::{SplitSink, SplitStream},
    SinkExt, StreamExt,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{
    mpsc,
    RwLock,
};

// 定义了一个公开的 Result<T> 类型别名，其中错误类型是一个动态分配的 trait 对象，可以用来表示可能的错误情况
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[tokio::main]
async fn main() -> Result<()> {
    let (sender_tx, mut sender_rx) = mpsc::channel::<MessagePayLoad>(1000);

    let app_state = AppState {
        clients: Arc::new(RwLock::new(
            HashMap::<u16, SplitSink<WebSocket, Message>>::new(),
        )),
        sender_tx,
    };

    // build our application with a single route
    let app = Router::new()
        .route("/", get(handler))
        .with_state(app_state.clone());

    // sender_task
    // 预留统一发送线程
    let mut sender_task = tokio::spawn(async move {
        while let Some(payload) = sender_rx.recv().await {
            if let Some(to) = payload.to {
                if let Some(sender) = app_state.clients.write().await.get_mut(&to) {
                    sender
                        .send(Message::Text(json!(payload).to_string()))
                        .await
                        .unwrap();
                }
            }
        }
    });

    // sender_task
    let mut server_task = tokio::spawn(async move {
        // run it with hyper on localhost:3000
        axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
            .serve(app.into_make_service_with_connect_info::<SocketAddr>())
            .await
            .unwrap();
    });

    // 监听两个线程，无论那个退出都把另一个退出
    tokio::select! {
        _ = (&mut sender_task) => {
            println!("sender_task finished");
            server_task.abort();
        },
        _ = (&mut server_task) => {
            println!("server_task finished");
            sender_task.abort();
        },
        // 捕获 ctrl_c 终止信号
        _ = tokio::signal::ctrl_c() => {
            println!("ctrl_c");
            sender_task.abort();
            server_task.abort();
        }
    }

    Ok(())
}

#[derive(Clone)]
pub struct AppState {
    pub clients: Arc<RwLock<HashMap<u16, SplitSink<WebSocket, Message>>>>,
    pub sender_tx: mpsc::Sender<MessagePayLoad>,
}

#[derive(Serialize, Deserialize)]
pub struct MessagePayLoad {
    msg_type: MsgType,
    from: Option<u16>,
    to: Option<u16>,
    data: String,
}

#[derive(Serialize, Deserialize)]
pub enum MsgType {
    Message,
    Command,
    Reply
}

async fn handler(
    ws: WebSocketUpgrade,
    State(app_state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, app_state, addr))
}

async fn handle_socket(socket: WebSocket, app_state: AppState, addr: SocketAddr) {
    let (sender, receiver) = socket.split();

    // 暂时使用连接的端口号作为用户 ID
    app_state.clients.write().await.insert(addr.port(), sender);

    // let (tx, rx) = mpsc::channel::<Message>(1000);

    // tokio::spawn(write(sender, rx));
    tokio::spawn(read(receiver, app_state, addr.port()));
}

async fn read(mut receiver: SplitStream<WebSocket>, app_state: AppState, client_port: u16) {
    while let Some(Ok(message)) = receiver.next().await {
        match message {
            Message::Text(text) => {
                println!("received: {text}");
                let mut payload = serde_json::from_str::<MessagePayLoad>(&text).unwrap();
                match payload.msg_type {
                    MsgType::Message => {
                        payload.from = Some(client_port);
                        app_state.sender_tx.send(payload).await.unwrap();
                    }
                    MsgType::Command => {
                        // 获取在线用户
                        if payload.data == "list" {
                            let clients = app_state.clients.read().await.iter().map(|(k,_)|k.clone()).collect::<Vec<u16>>();
                            let reply = MessagePayLoad {
                                msg_type: MsgType::Reply,
                                from: None,
                                to: Some(client_port),
                                data: json!(clients).to_string(),
                            };
                            app_state.sender_tx.send(reply).await.unwrap();
                        }
                    }
                    _ => {}
                }

            }
            Message::Close(_) => {
                println!("connection closed");
                break;
            }
            _ => println!("received something else")
        }
    }
    // 连接断开时删除 state 中的 sender
    app_state.clients.write().await.remove(&client_port);
}

// async fn write(mut sender: SplitSink<WebSocket, Message>, mut rx: mpsc::Receiver<Message>) {
//     while let Some(message) = rx.recv().await {
//         sender.send(message).await.unwrap();
//     }
// }
