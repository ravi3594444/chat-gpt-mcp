use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::{mpsc, oneshot, Mutex, Semaphore};

type Pending = HashMap<String, oneshot::Sender<Result<Value, String>>>;

#[derive(Clone)]
pub struct Bridge {
    connection: Arc<Mutex<Option<Connection>>>,
    pending: Arc<Mutex<Pending>>,
    serial: Arc<Semaphore>,
}

struct Connection {
    id: String,
    name: String,
    tx: mpsc::Sender<Message>,
    device_token: String,
    auth: crate::oauth::Auth,
}

impl Bridge {
    pub fn new() -> Self {
        Self {
            connection: Default::default(),
            pending: Default::default(),
            serial: Arc::new(Semaphore::new(1)),
        }
    }

    pub async fn status(&self) -> Value {
        let connection = self.connection.lock().await;
        match connection
            .as_ref()
            .filter(|c| c.auth.device_name(&c.device_token).is_some())
        {
            Some(c) => {
                json!({"connected":true,"browser":c.name,"instruction":"Only tabs explicitly enabled in the extension are accessible."})
            }
            None => {
                json!({"connected":false,"instruction":"Connect Browser Link in Chrome, then enable the tab you want to use."})
            }
        }
    }

    pub async fn call(&self, mut args: Value) -> Result<Value, String> {
        let _permit = self
            .serial
            .acquire()
            .await
            .map_err(|_| "Bridge is shutting down")?;
        let tx = self
            .connection
            .lock()
            .await
            .as_ref()
            .filter(|c| c.auth.device_name(&c.device_token).is_some())
            .map(|c| c.tx.clone())
            .ok_or("Browser is offline or its pairing was revoked. Connect the extension first.")?;
        let id = crate::oauth::random_token();
        args["id"] = json!(id);
        args["expires_at"] = json!(crate::oauth::now() * 1000 + 25_000);
        let (response_tx, response_rx) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), response_tx);
        if tx
            .send(Message::Text(args.to_string().into()))
            .await
            .is_err()
        {
            self.pending.lock().await.remove(&id);
            return Err("Browser disconnected before the command was delivered".into());
        }
        let response = tokio::time::timeout(Duration::from_secs(28), response_rx).await;
        self.pending.lock().await.remove(&id);
        match response {
            Ok(Ok(result)) => result,
            _ => Err("Browser did not return a result. The action may have happened; take a fresh snapshot before deciding whether to retry.".into()),
        }
    }

    pub async fn socket(self, mut socket: WebSocket, auth: crate::oauth::Auth) {
        let first = tokio::time::timeout(Duration::from_secs(5), socket.recv()).await;
        let Some(Ok(Message::Text(first))) = first.ok().flatten() else {
            return;
        };
        let Ok(hello) = serde_json::from_str::<Value>(&first) else {
            return;
        };
        let token = hello["token"].as_str().unwrap_or_default();
        let Some(name) = auth.device_name(token) else {
            let _ = socket
                .send(Message::Text(
                    json!({"type":"error","error":"Pair the extension again"})
                        .to_string()
                        .into(),
                ))
                .await;
            return;
        };
        let id = crate::oauth::random_token();
        let (tx, mut rx) = mpsc::channel::<Message>(16);
        {
            let mut connection = self.connection.lock().await;
            if connection
                .as_ref()
                .is_some_and(|c| c.auth.device_name(&c.device_token).is_some())
            {
                let _ = socket.send(Message::Text(json!({"type":"error","error":"Another browser is already connected. Disconnect it first."}).to_string().into())).await;
                return;
            }
            if let Some(old) = connection.take() {
                let _ = old.tx.try_send(Message::Close(None));
            }
            *connection = Some(Connection {
                id: id.clone(),
                name,
                tx,
                device_token: token.to_string(),
                auth: auth.clone(),
            });
        }
        let _ = socket
            .send(Message::Text(
                json!({"type":"connected"}).to_string().into(),
            ))
            .await;
        let (mut writer, mut reader) = socket.split();
        loop {
            tokio::select! {
                message = rx.recv() => match message {
                    Some(message) => if writer.send(message).await.is_err() { break; },
                    None => break,
                },
                message = reader.next() => match message {
                    Some(Ok(Message::Text(text))) => {
                        if auth.device_name(token).is_none() { break; }
                        let Ok(value) = serde_json::from_str::<Value>(&text) else { continue; };
                        if value["type"] == "ping" {
                            if writer.send(Message::Text(json!({"type":"pong"}).to_string().into())).await.is_err() { break; }
                            continue;
                        }
                        if let Some(command_id) = value["id"].as_str() {
                            if let Some(tx) = self.pending.lock().await.remove(command_id) {
                                let result = if let Some(error) = value["error"].as_str() {
                                    Err(error.chars().take(500).collect())
                                } else { Ok(value["result"].clone()) };
                                let _ = tx.send(result);
                            }
                        }
                    },
                    Some(Ok(Message::Ping(data))) => { if writer.send(Message::Pong(data)).await.is_err() { break; } },
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    _ => {},
                }
            }
        }
        let mut connection = self.connection.lock().await;
        if connection.as_ref().is_some_and(|c| c.id == id) {
            *connection = None;
            for (_, tx) in self.pending.lock().await.drain() {
                let _ = tx.send(Err(
                    "Browser disconnected. Verify the page state before retrying an action.".into(),
                ));
            }
        }
    }
}
