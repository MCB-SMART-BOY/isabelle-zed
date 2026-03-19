use crate::diagnostics::{PublishedDiagnosticTargets, publish_diagnostics_for};
use crate::transport::BridgeTransport;
use bridge::protocol::{DocumentPushPayload, MessageType};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc, oneshot};
use tokio::time::{Duration, Instant, MissedTickBehavior};
use tower_lsp::Client;
use tower_lsp::lsp_types::Url;

#[derive(Clone)]
struct PendingPush {
    uri: Url,
    version: i64,
    text: String,
    queued_at: Instant,
}

pub(crate) enum PushEvent {
    Update {
        uri: Url,
        version: i64,
        text: String,
    },
    Flush {
        uris: Option<Vec<Url>>,
        respond_to: oneshot::Sender<()>,
    },
}

pub(crate) fn spawn_push_worker(
    mut rx: mpsc::UnboundedReceiver<PushEvent>,
    client: Client,
    bridge: BridgeTransport,
    published_diagnostic_targets: PublishedDiagnosticTargets,
    session_running: Arc<RwLock<bool>>,
    debounce_window: Duration,
) {
    tokio::spawn(async move {
        let mut pending_by_uri: HashMap<Url, PendingPush> = HashMap::new();
        let mut flush_tick = tokio::time::interval(Duration::from_millis(50));
        flush_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                event = rx.recv() => {
                    match event {
                        Some(PushEvent::Update { uri, version, text }) => {
                            let queued_at = Instant::now();
                            match pending_by_uri.get_mut(&uri) {
                                Some(existing) if existing.version > version => {}
                                Some(existing) => {
                                    existing.version = version;
                                    existing.text = text;
                                    existing.queued_at = queued_at;
                                }
                                None => {
                                    pending_by_uri.insert(uri.clone(), PendingPush { uri, version, text, queued_at });
                                }
                            }
                        }
                        Some(PushEvent::Flush { uris, respond_to }) => {
                            let targets = match uris {
                                Some(list) => list,
                                None => pending_by_uri.keys().cloned().collect(),
                            };
                            for uri in targets {
                                if let Some(pending) = pending_by_uri.remove(&uri)
                                    && let Err(err) = send_document_push(
                                        &bridge,
                                        &client,
                                        &published_diagnostic_targets,
                                        &session_running,
                                        &pending.uri,
                                        pending.version,
                                        pending.text,
                                    )
                                    .await
                                {
                                    crate::log_error_for(
                                        &client,
                                        format!("failed to push document: {err}"),
                                    )
                                    .await;
                                }
                            }
                            let _ = respond_to.send(());
                        }
                        None => break,
                    }
                }
                _ = flush_tick.tick() => {
                    let now = Instant::now();
                    let ready = pending_by_uri
                        .iter()
                        .filter(|(_, pending)| now.duration_since(pending.queued_at) >= debounce_window)
                        .map(|(uri, _)| uri.clone())
                        .collect::<Vec<_>>();

                    for uri in ready {
                        if let Some(pending) = pending_by_uri.remove(&uri)
                            && let Err(err) = send_document_push(
                                &bridge,
                                &client,
                                &published_diagnostic_targets,
                                &session_running,
                                &pending.uri,
                                pending.version,
                                pending.text,
                            )
                            .await
                        {
                            crate::log_error_for(
                                &client,
                                format!("failed to push document: {err}"),
                            )
                            .await;
                        }
                    }
                }
            }
        }
    });
}

async fn send_document_push(
    bridge: &BridgeTransport,
    client: &Client,
    published_diagnostic_targets: &PublishedDiagnosticTargets,
    session_running: &Arc<RwLock<bool>>,
    uri: &Url,
    version: i64,
    text: String,
) -> Result<(), String> {
    if !*session_running.read().await {
        return Ok(());
    }

    let payload = serde_json::to_value(DocumentPushPayload {
        uri: uri.to_string(),
        text,
    })
    .map_err(|err| err.to_string())?;

    let response = bridge
        .request(MessageType::DocumentPush, version, payload)
        .await
        .map_err(|err| err.to_string())?;

    publish_diagnostics_for(
        client,
        published_diagnostic_targets,
        uri.clone(),
        version,
        response,
    )
    .await
}
