use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
//use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use async_tungstenite::tokio::accept_async_with_config;
use async_tungstenite::tungstenite::protocol::Message;
use async_tungstenite::tungstenite::error::Error;
use tungstenite::protocol::WebSocketConfig;
use tokio::net::lookup_host;


pub struct ServerSocket {
    address: String,
}

impl ServerSocket {
    pub fn new(address: &str) -> Self {
        Self {
            address: address.to_string(),
        }
    }

    pub async fn run(&self) -> Result<(), Error> {
        println!("Resolving address: {}", self.address);
        let mut addrs = lookup_host(&self.address).await
            .map_err(|e| println!("Error resolving address: {}", e.to_string()))
            .unwrap();

        let addr = addrs.next().ok_or_else(|| {
            println!("No address found for: {}", self.address);
            Error::Url(tungstenite::error::UrlError::NoHostName)
        })?;

        println!("Resolved to IP address: {}", addr);

        // Extract IP and Port separately
        let ip_only = addr.ip();
        let port = addr.port();
        let bind_addr = format!("{}:{}", ip_only, port);

        println!("Binding to: {}", bind_addr);

        println!("Setting address: {}", self.address);
        let listener = TcpListener::bind(&bind_addr).await
            .map_err(|e| println!("Error: {}", e.to_string()))
            .unwrap();
        println!("WebSocket server listening on: {}", self.address);

        while let Ok((stream, addr)) = listener.accept().await {
            println!("New connection from: {}", addr);
            tokio::spawn(Self::handle_connection(stream));
        }

        Ok(())
    }

    async fn handle_connection(stream: TcpStream) {
        let config = Some(WebSocketConfig::default());


        let ws_stream = match accept_async_with_config(stream, config).await {
            Ok(ws_stream) => ws_stream,
            Err(e) => {
                println!("Error during handshake: {}", e);
                return;
            }
        };

        let (mut write, mut read) = ws_stream.split();
        let (tx, mut rx) = mpsc::channel::<String>(100);

        // Spawn task to handle outgoing messages
        let write_task = tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if write.send(Message::Text(msg)).await.is_err() {
                    break;
                }
            }
        });

        // Handle incoming messages
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    println!("Received: {}", text);
                    if let Err(_) = tx.send(format!("Echo: {}", text)).await {
                        break;
                    }
                }
                Ok(Message::Close(_)) => break,
                Err(e) => {
                    println!("Error receiving message: {}", e);
                    break;
                }
                _ => {}
            }
        }

        write_task.abort();
    }
}

pub async fn main_() -> Result<(), Error> {
    let server = ServerSocket::new("127.0.0.1:8080");
    server.run().await
}