use async_tungstenite::tokio::accept_async;
use tokio::net::TcpListener;
use futures_util::{SinkExt, StreamExt};
use async_tungstenite::tungstenite::Message;
use tokio::sync::mpsc;
use tokio::task;

pub struct ServerSocket {
    address: String,
    tx: mpsc::Sender<String>,
}

impl ServerSocket {
    pub fn new(address: &str) -> Self {
        let (tx, rx) = mpsc::channel(100);
        let server = Self {
            address: address.to_string(),
            tx,
        };
        server.start_polling(rx);
        server
    }

    fn start_polling(&self, mut rx: mpsc::Receiver<String>) {
        let tx = self.tx.clone();
        task::spawn(async move {
            loop {
                // Simulate API call/polling
                let data = "Collected data from API".to_string();
                tx.send(data).await.unwrap();
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            }
        });
    }

    pub async fn run(&self) {
        let listener = TcpListener::bind(&self.address).await.unwrap();
        println!("Listening on: {}", self.address);

        while let Ok((stream, _)) = listener.accept().await {
            let tx = self.tx.clone();
            tokio::spawn(Self::handle_connection(stream, tx));
        }
    }

    async fn handle_connection(stream: tokio::net::TcpStream, mut tx: mpsc::Sender<String>) {
        let ws_stream = accept_async(stream).await.unwrap();
        let (mut write, mut read) = ws_stream.split();
        let write_clone = write.clone();

        let (data_tx, mut data_rx) = mpsc::channel(100);
        tokio::spawn(async move {
            while let Some(data) = data_rx.recv().await {
                &write.send(Message::Text(data)).await.unwrap();
            }
        });

        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Frame(_)) => {
                    // Handle frame message
                    println!("Received frame message");
                }
                Ok(Message::Text(text)) => {
                    // Handle text message
                    println!("Received text message: {}", text);
                    let response = format!("Echo: {}", text);
                    // Sending a response to the received text message
                    &write.send(Message::Text(response)).await.unwrap();
                }
                Ok(Message::Binary(bin)) => {
                    // Handle binary message
                    println!("Received binary message: {:?}", bin);
                    write.send(Message::Binary(bin)).await.unwrap();
                }
                Ok(Message::Ping(ping)) => {
                    // Handle ping message
                    println!("Received ping: {:?}", ping);
                    write.send(Message::Pong(ping)).await.unwrap();
                }
                Ok(Message::Pong(_)) => {
                    // Handle pong message
                    println!("Received pong");
                }
                Ok(Message::Close(reason)) => {
                    // Handle close message
                    println!("Received close: {:?}", reason);
                    write.send(Message::Close(reason)).await.unwrap();
                    break;
                }
                Err(e) => {
                    // Handle error
                    println!("Error receiving message: {}", e);
                    break;
                }
            }
        }
    }
}

pub async fn main_() {
    let server = ServerSocket::new("127.0.0.1:8080");
    server.run().await;
}