use futures_util::{SinkExt, StreamExt, Future};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
//use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use async_tungstenite::tokio::accept_async_with_config;
use async_tungstenite::tungstenite::protocol::Message;
use async_tungstenite::tungstenite::error::Error;
use tungstenite::protocol::WebSocketConfig;
use tokio::net::lookup_host;
use serde_json::{to_value, Value};
use serde::{Serialize, Deserialize};
use crate::auth_config::BatchConfig;
use crate::cache::SharedLockedCache;
use crate::request_parser::parser::CallParser;
use crate::request_parser::params::*;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use std::pin::Pin;


use crate::stock::StockPolling;
//use crate::crypto::CryptoPolling;
//use crate::forex::ForexPolling;
//use crate::economic_data::EconomicDataPolling;
//use crate::technical_indicators::TechnicalIndicatorPolling;
//use crate::market::MarketPolling;

const REQUEST_SUCCUESS: u32 = 200;
const REQUEST_FAILED: u32 = 400;
const NOT_ALLOWED: u32 = 500;
const CACHE_SIZE: usize = 1000;

pub struct ServerSocket {
    address: String,
    make: MakeResponse,
    state: Arc<PollState>,
}

impl ServerSocket {
    pub fn new(address: &str) -> Self {
        Self {
            address: address.to_string(),
            make: MakeResponse::new(),
            state: Arc::new(PollState {
                cache: Arc::new(Mutex::new(SharedLockedCache::new(CACHE_SIZE))),
                batch_config: Arc::new(BatchConfig::default()),
            }),
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

        println!("Setting address: {}", self.address);
        let listener = TcpListener::bind(&addr).await
            .map_err(|e| println!("Error: {}", e.to_string()))
            .unwrap();
        println!("WebSocket server listening on: {}", self.address);

        while let Ok((stream, addr)) = listener.accept().await {
            println!("New connection from: {}", addr);
            tokio::spawn(Self::handle_connection(stream, self.make.clone(), self.state.clone()));
        }

        Ok(())
    }

    async fn handle_connection(stream: TcpStream, make: MakeResponse, state: Arc<PollState>) {
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
                    println!("**Received**: {}", text);
                    match serde_json::from_str::<Value>(&text) {
                        Ok(json) => {
                            let state = Arc::clone(&state);
                            println!("**Parsed JSON**: {:?}", json);
                            println!("Making Response...");
                            let response = make.make(state, &text).await;
                            println!("Sending response...");
                            if let Err(_) = tx.send(format!("{}", &response)).await {
                                break;
                            }
                            println!("Response sent");
                        }
                        Err(e) => {
                            println!("Failed to parse JSON: {}", e);
                            if let Err(_) = tx.send("Invalid JSON".to_string()).await {
                                break;
                            }
                        }
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

pub struct PollState {
    cache: Arc<Mutex<SharedLockedCache>>,
    batch_config: Arc<BatchConfig>,
}
impl Default for PollState{
    fn default() -> Self {
        Self {
            cache: Arc::new(Mutex::new(SharedLockedCache::new(CACHE_SIZE))),
            batch_config: Arc::new(BatchConfig::default()),
        }
    }
}
struct Collection;
impl Collection {
    async fn stock_polling_unpinned(state: Arc<PollState>, args: Arc<Value>) -> String{
        let stock_polling = StockPolling::new(state.cache.clone(), state.batch_config.clone());
        stock_polling
            .poll(&args)
            .await
            .map(|_| "Stock polling successful".to_string())
            .unwrap_or_else(|e| format!("Stock polling failed: {}", e))
        
    }

    fn stock_polling_func(state: Arc<PollState>, args: Arc<Value>) -> Pin<Box<dyn Future<Output = String> + Send + 'static>> {
        Box::pin(async move {
            Collection::stock_polling_unpinned(state, args).await
        })
    }
    
}


type Func = fn(Arc<PollState>, Arc<Value>) -> Pin<Box<dyn Future<Output = String> + Send + 'static>>;

#[derive(Clone)]
pub struct MakeResponse{
    fn_map: HashMap<String, Box<Func>>,
}
impl MakeResponse {
    pub fn new() -> Self {
        Self {
            fn_map: HashMap::new(),
        }
    }

    fn register_function(&mut self, where_: String, func: Func) {
        self.fn_map.insert(where_, Box::new(func));
    }

    pub fn build(&mut self){
        self.register_function("stock_polling".to_string(), Collection::stock_polling_func);
    }

    pub async fn unsafe_make(&self, state: Arc<PollState>, s: &str) -> String {
        let call_request: CallRequest = CallParser::key_lookup_parse_json(s).unwrap();
        if call_request.target.to_str() == "task" {
           match call_request.args {
               Args::TaskArgs(task_args) => {
                   let function = task_args.function;
                   let args = task_args.params;
                   if args.is_none() {
                       return "Error: The argument hashmap is empty.".to_string();
                   }
                   let where_ = task_args.look_for.where_;
                   match function {
                       TaskFunction::AggregatedPolling => {
                            let func = self.map_func(&where_).unwrap();
                            let state  = Arc::clone(&state);
                            let args= Arc::new(to_value(args.unwrap().clone()).unwrap());
                            let message = self.exec_func(&func, state, args)
                                .await
                                .map_err(|e| self.return_error(e.to_string()));
                            let status = REQUEST_SUCCUESS;
                            let response = ServerResponse::new(status, Some(message.unwrap().clone()));
                            return response.to_json();
                       }
                       _ => {}
                   }
               }
               _ => {}
           }
        }
        ServerResponse::new(REQUEST_SUCCUESS, Some(s.to_string())).to_json()
    }

    pub async fn make(&self, state: Arc<PollState>, s: &str) -> String {
        let call_request = match CallParser::key_lookup_parse_json(s) {
            Ok(req) => req,
            Err(err) => return ServerResponse::new(REQUEST_FAILED, Some(err)).to_json(),
        };
        println!("**Call request**: {:#?}", call_request);
    
        if call_request.target.to_str() == "task" {
            if let Args::TaskArgs(task_args) = call_request.args {
                if let TaskFunction::AggregatedPolling = task_args.function {
                    return self.handle_task(state, task_args).await;
                }
            }
        }
    
        ServerResponse::new(NOT_ALLOWED, Some(s.to_string())).to_json()
    }
    async fn handle_task(&self, state: Arc<PollState>, task_args: TaskArgs) -> String {
        let where_ = task_args.look_for.where_;
        if let Some(args) = task_args.params {
            if let Some(func) = self.map_func(&where_) {
                let args = Arc::new(to_value(args).unwrap());
                let result = func(state, args).await;
                return ServerResponse::new(REQUEST_SUCCUESS, Some(result)).to_json();
            }
        }
    
        ServerResponse::new(REQUEST_FAILED, Some("Function not found or invalid arguments".to_string())).to_json()
    }
    
    fn map_func(&self, where_: &String) -> Option<Box<Func>> {
        if let Some(func) = self.fn_map.get(where_) {
            Some(func.clone())
        } else {
            None
        }
    }

    async fn exec_func(&self, func: &Func, state: Arc<PollState>, args: Arc<Value>) -> Result<String, Error> {
        let result = func(state, args).await;
        Ok(result)
    }

    fn return_error(&self, msg: String) -> ServerResponse {
        let status = REQUEST_FAILED;
        ServerResponse::new(status, Some(msg.to_string()))

    }
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerResponse {
    pub status: u32,
    pub message: Option<String>,
}
impl ServerResponse {
    pub fn new(status: u32, message: Option<String>) -> Self {
        Self {
            status,
            message,
        }
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap()
    }
    
}





pub async fn main_() -> Result<(), Error> {
    let server = ServerSocket::new("0.0.0.0:8080");
    server.run().await
}