use std::{sync::{Arc, RwLock}, collections::HashMap};

use axum::{
    extract::{
        ws::{Message, WebSocket},
        WebSocketUpgrade,
    },
    response::Response,
    routing::get,
    Router,
};
use futures::{
    stream::{SplitSink, SplitStream},
    SinkExt, StreamExt,
};
use tokio::sync::mpsc;

// 定义了一个公开的 Result<T> 类型别名，其中错误类型是一个动态分配的 trait 对象，可以用来表示可能的错误情况
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[tokio::main]
async fn main() -> Result<()> {
    // build our application with a single route
    let app = Router::new().route("/", get(handler));

    // run it with hyper on localhost:3000
    axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
        .serve(app.into_make_service())
        .await
        .unwrap();
    Ok(())
}

pub struct AppState {
    pub clients: Arc<RwLock<HashMap<u16,SplitSink<WebSocket, Message>>>>,
}

async fn handler(ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(handle_socket)
}

async fn handle_socket(socket: WebSocket) {
    let (sender, receiver) = socket.split();

    let (tx, rx) = mpsc::channel::<Message>(1000);

    tokio::spawn(write(sender, rx));
    tokio::spawn(read(receiver, tx));
}

async fn read(mut receiver: SplitStream<WebSocket>, tx: mpsc::Sender<Message>) {
    while let Some(Ok(message)) = receiver.next().await {
        match message {
            Message::Text(text) => {
                println!("received: {text}");
                // mspc的发送端可以多次克隆共享
                // 实现服务端在多场景下主动推送
                tx.send(Message::Text(text)).await.unwrap();
            }
            Message::Close(_) => {
                println!("connection closed");
                break;
            }
            _ => println!("received something else"),
        }
    }
}

async fn write(mut sender: SplitSink<WebSocket, Message>, mut rx: mpsc::Receiver<Message>) {
    while let Some(message) = rx.recv().await {
        sender.send(message).await.unwrap();
    }
}
