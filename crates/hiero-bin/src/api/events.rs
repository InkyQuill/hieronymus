use axum::{
    extract::{
        State, WebSocketUpgrade,
        ws::{CloseFrame, Message, WebSocket},
    },
    response::Response,
};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::daemon::AppState;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AdminEvent {
    Refresh,
    ResyncRequired,
}

#[must_use]
pub fn admin_event_channel(
    capacity: usize,
) -> (
    broadcast::Sender<AdminEvent>,
    broadcast::Receiver<AdminEvent>,
) {
    broadcast::channel(capacity)
}

pub async fn next_admin_event(
    receiver: &mut broadcast::Receiver<AdminEvent>,
) -> Option<AdminEvent> {
    match receiver.recv().await {
        Ok(event) => Some(event),
        Err(broadcast::error::RecvError::Lagged(_)) => Some(AdminEvent::ResyncRequired),
        Err(broadcast::error::RecvError::Closed) => None,
    }
}

pub(crate) async fn admin_ws(State(state): State<AppState>, upgrade: WebSocketUpgrade) -> Response {
    let events = state.events.subscribe();
    let shutdown = state.shutdown.subscribe();
    let workers = state.workers.clone();
    upgrade.on_upgrade(move |socket| async move {
        let _ = workers
            .spawn(stream_admin_events(socket, events, shutdown))
            .await;
    })
}

async fn stream_admin_events(
    mut socket: WebSocket,
    mut events: broadcast::Receiver<AdminEvent>,
    mut shutdown: broadcast::Receiver<()>,
) {
    loop {
        tokio::select! {
            event = next_admin_event(&mut events) => {
                let Some(event) = event else {
                    close_socket(&mut socket).await;
                    break;
                };
                let Ok(payload) = serde_json::to_string(&event) else {
                    close_socket(&mut socket).await;
                    break;
                };
                if socket.send(Message::Text(payload.into())).await.is_err() {
                    break;
                }
            }
            _ = shutdown.recv() => {
                close_socket(&mut socket).await;
                break;
            }
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    Some(Ok(_)) => {}
                }
            }
        }
    }
}

async fn close_socket(socket: &mut WebSocket) {
    let _ = socket
        .send(Message::Close(Some(CloseFrame {
            code: 1001,
            reason: "server shutdown".into(),
        })))
        .await;
}
