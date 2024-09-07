use anyhow::Result;
use clap::Parser;
use futures_util::{SinkExt, StreamExt, TryFutureExt};
use log::{debug, trace, warn, LevelFilter};
use log::{error, info};
use log4rs::append::console::ConsoleAppender;
use log4rs::config::{Appender, Config, Root};
use maplit::hashmap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use utils::{download_file, execute_shell, upload_file};
mod config;
mod utils;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

#[derive(Debug, Deserialize, Serialize)]
struct EventMessage {
    id: u64,
    code: i32,
    event: String,
    data: Option<HashMap<String, serde_json::Value>>,
}

struct FileDownloadUploadTask {
    id: u64,
    url: String,
    path: String,
}

struct ExecuteTask {
    id: u64,
    cmd: String,
}

enum Event {
    Download(FileDownloadUploadTask),
    Upload(FileDownloadUploadTask),
    Execute(ExecuteTask),
    Raw(EventMessage),
}

fn json_str(map: &HashMap<String, Value>, key: &str) -> Option<String> {
    map.get(key).map_or(None, |v| {
        if let Value::String(v2) = v {
            Some(v2.clone())
        } else {
            None
        }
    })
}

// fn json_int(map: &HashMap<String, Value>, key: &str) -> Option<i64> {
//     map.get(key).map_or(None, |v| {
//         if let Value::Number(v2) = v {
//             v2.as_i64()
//         } else {
//             None
//         }
//     })
// }

fn json_bool(map: &HashMap<String, Value>, key: &str) -> Option<bool> {
    map.get(key).map_or(None, |v| {
        if let Value::Bool(v2) = v {
            Some(*v2)
        } else {
            None
        }
    })
}

impl From<EventMessage> for Event {
    fn from(msg: EventMessage) -> Self {
        match msg.event.as_str() {
            "download" => {
                if let Some(data) = msg.data.as_ref() {
                    if let Some(url) = json_str(data, "url") {
                        if let Some(path) = json_str(data, "path") {
                            return Event::Download(FileDownloadUploadTask {
                                id: msg.id,
                                url,
                                path,
                            });
                        }
                    }
                }
                Event::Raw(msg)
            }
            "upload" => {
                if let Some(data) = msg.data.as_ref() {
                    if let Some(url) = json_str(data, "url") {
                        if let Some(path) = json_str(data, "path") {
                            return Event::Upload(FileDownloadUploadTask {
                                id: msg.id,
                                url,
                                path,
                            });
                        }
                    }
                }
                Event::Raw(msg)
            }
            "execute" => {
                if let Some(data) = msg.data.as_ref() {
                    if let Some(cmd) = json_str(data, "cmd") {
                        return Event::Execute(ExecuteTask { id: msg.id, cmd });
                    }
                }
                Event::Raw(msg)
            }
            _ => Event::Raw(msg),
        }
    }
}

async fn handle_message(
    event: Event,
    tx: &mut futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
    client: &reqwest::Client,
) -> Result<()> {
    match event {
        Event::Download(task) => {
            info!("Task download begin: id = {}", task.id);
            let response = EventMessage {
                id: task.id,
                event: "task_completed".to_string(),
                code: if download_file(client, task.url.as_str(), task.path.as_str())
                    .await
                    .is_ok()
                {
                    0
                } else {
                    1
                },
                data: None,
            };
            tx.send(Message::Text(json!(response).to_string())).await?;
            info!("Task download completed: id = {}", task.id);
        }
        Event::Upload(task) => {
            info!("Task upload begin: id = {}", task.id);
            let response = EventMessage {
                id: task.id,
                event: "task_completed".to_string(),
                code: if upload_file(client, task.url.as_str(), task.path.as_str())
                    .await
                    .is_ok()
                {
                    0
                } else {
                    1
                },
                data: None,
            };
            tx.send(Message::Text(json!(response).to_string())).await?;
            info!("Task upload completed: id = {}", task.id);
        }
        Event::Execute(task) => {
            info!("Task execute begin: id = {}", task.id);
            let response = EventMessage {
                id: task.id,
                event: "task_completed".to_string(),
                code: if let Ok(sc) = execute_shell(&task.cmd).await {
                    sc
                } else {
                    -1
                },
                data: None,
            };
            tx.send(Message::Text(json!(response).to_string())).await?;
            info!("Task execute completed: id = {}", task.id);
        }
        Event::Raw(msg) => {
            warn!("Received unknown event type, ignore");
            let response = EventMessage {
                id: msg.id,
                event: "task_completed".to_string(),
                code: 0x80000000u32 as i32,
                data: Some(hashmap! {
                    "error".to_string() => Value::String("Unknown event type".to_string())
                }),
            };
            tx.send(Message::Text(json!(response).to_string())).await?;
        }
    }
    Ok(())
}

async fn agent_main(config: config::Config) -> Result<()> {
    let api_base_url = format!(
        "{}://{}:{}/{}",
        if config.https { "https" } else { "http" },
        config.addr,
        config.port,
        config.api_base_path
    );
    info!("Use Controller URL: {}", api_base_url);
    let client = reqwest::Client::new();
    loop {
        info!("Trying to connect to controller",);
        let res = client
            .post(&format!("{}/register", api_base_url))
            .json(&serde_json::json!({
                "clientId": utils::get_machine_uuid()?.to_string(),
            }))
            .send()
            .await;
        match res {
            Ok(response) => {
                debug!("Connected to controller: {:?}", response.status());
                let client_conf: EventMessage = response.json().await?;
                info!("Registered to controller: {:?}", client_conf);
                let ws_url = if let Some(data) = client_conf.data {
                    if let Some(ws) = json_str(&data, "ws") {
                        if json_bool(&data, "redirct").is_some_and(|v| v) {
                            Some(ws)
                        } else {
                            Some(format!(
                                "{}://{}:{}/{}",
                                if config.https { "wss" } else { "ws" },
                                config.addr,
                                config.port,
                                ws
                            ))
                        }
                    } else {
                        Some(format!(
                            "{}://{}:{}/{}/ws",
                            if config.https { "wss" } else { "ws" },
                            config.addr,
                            config.port,
                            config.api_base_path
                        ))
                    }
                } else {
                    None
                };
                if ws_url.is_none() {
                    error!("Failed to get websocket URL from controller, retry in 15 seconds...");
                    tokio::time::sleep(tokio::time::Duration::from_secs(15)).await;
                    continue;
                }
                let ws_url = ws_url.unwrap(); // Safe to unwrap here
                info!("Connecting to controller websocket: {}", ws_url);
                let (ws, _) = connect_async(ws_url).await?;
                let (mut tx, mut rx) = ws.split();
                trace!("Websocket connected to controller. Begin to handle message loop");
                while let Some(event) = rx.next().await {
                    match event {
                        Ok(ws_msg) => {
                            debug!("Received message: {:?}", ws_msg);
                            match ws_msg {
                                Message::Text(msg) => {
                                    trace!("Received text message from controller");
                                    let event_msg: EventMessage = serde_json::from_str(&msg)?;
                                    log::info!("Received event: {:?}", event_msg);
                                    _ = handle_message(Event::from(event_msg), &mut tx, &client)
                                        .inspect_err(|err| {
                                            error!("Failed to handle message: {}", err);
                                        });
                                }
                                Message::Binary(_) => {
                                    // Binary message from controller, do nothing
                                    // Controller should NEVER send binary message to agent
                                    debug!("Received binary message from controller");
                                }
                                Message::Ping(msg) => {
                                    tx.send(Message::Pong(msg)).await?;
                                    debug!("Received Ping from controller, Pong sent");
                                }
                                Message::Pong(_) => {
                                    // Pong message from controller, do nothing
                                    // In fact, agent will never send ping message as of now
                                    debug!("Received Pong from controller");
                                }
                                Message::Close(_) => {
                                    // Unexpected close message from controller. Connection closing should be initiated by agent or by specific event from controller
                                    warn!("Websocket connection closed, retry");
                                }
                                Message::Frame(_) => {
                                    // As tungstenite noted, this should not happen, maybe a malformed controller response
                                    warn!("Received a frame message type, which should NOT happen and maybe a bug")
                                }
                            }
                        }
                        Err(err) => {
                            error!("Failed to receive message: {}", err);
                        }
                    }
                }
            }
            Err(err) => {
                error!(
                    "Failed to connect to controller: {}. Retry in 15 seconds...",
                    err
                );
                tokio::time::sleep(tokio::time::Duration::from_secs(15)).await;
            }
        }
    }
}

#[tokio::main]
async fn main() {
    if let Err(err) = log4rs::init_file("log4rs.yml", Default::default()) {
        eprintln!("Failed to initialize log4rs: {}", err);
        if let Ok(config) = Config::builder()
            .appender(
                Appender::builder().build("stdout", Box::new(ConsoleAppender::builder().build())),
            )
            .build(Root::builder().appender("stdout").build(LevelFilter::Debug))
        {
            if log4rs::init_config(config).is_err() {
                eprintln!("Failed to initialize log4rs with default config");
                return;
            }
        } else {
            eprintln!("Failed to construct default log4rs config");
            return;
        }
    }

    let args = config::Args::parse();
    let config = config::Config::from(args);
    info!("MetalX Agent - Launching with config: {:?}", config);
    loop {
        if let Err(err) = agent_main(config.clone()).await {
            error!("Agent failed: {}", err);
            info!("Restart in 60 seconds...");
            tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
        }
    }
}
