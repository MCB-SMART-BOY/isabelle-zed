use crate::protocol::{DocumentPushPayload, JsonMessage, MessageType};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{debug, warn};

pub struct DebounceQueue {
    pending: Arc<RwLock<HashMap<String, (JsonMessage, Instant)>>>,
    debounce_ms: u64,
    tx: mpsc::Sender<JsonMessage>,
    #[allow(dead_code)]
    rx: mpsc::Receiver<JsonMessage>,
}

impl DebounceQueue {
    pub fn new(debounce_ms: u64) -> Self {
        let (tx, rx) = mpsc::channel(100);
        Self {
            pending: Arc::new(RwLock::new(HashMap::new())),
            debounce_ms,
            tx,
            rx,
        }
    }

    pub fn sender(&self) -> mpsc::Sender<JsonMessage> {
        self.tx.clone()
    }

    pub async fn run(&self) {
        let debounce_duration = Duration::from_millis(self.debounce_ms);
        
        loop {
            tokio::time::sleep(Duration::from_millis(50)).await;
            
            let now = Instant::now();
            let mut to_send = Vec::new();
            
            {
                let mut pending = self.pending.write();
                let keys: Vec<String> = pending.keys().cloned().collect();
                
                for key in keys {
                    if let Some((msg, time)) = pending.get(&key)
                        && now.duration_since(*time) >= debounce_duration {
                            to_send.push(msg.clone());
                            pending.remove(&key);
                        }
                }
            }
            
            for msg in to_send {
                debug!("Sending debounced message: {}", msg.id);
                if self.tx.send(msg).await.is_err() {
                    warn!("Receiver dropped");
                    break;
                }
            }
        }
    }

    pub fn enqueue(&self, msg: JsonMessage) {
        if msg.msg_type != MessageType::DocumentPush {
            return;
        }
        
        if let Ok(payload) = serde_json::from_value::<DocumentPushPayload>(msg.payload.clone()) {
            let key = payload.uri;
            debug!("Enqueuing message for {} (debounce {}ms)", key, self.debounce_ms);
            self.pending.write().insert(key, (msg, Instant::now()));
        }
    }
}

pub fn merge_documents(messages: Vec<JsonMessage>) -> Option<JsonMessage> {
    if messages.is_empty() {
        return None;
    }

    let mut latest: Option<(usize, &JsonMessage)> = None;
    
    for (i, msg) in messages.iter().enumerate() {
        if msg.msg_type == MessageType::DocumentPush
            && (latest.is_none() || msg.version > latest.unwrap().1.version) {
                latest = Some((i, msg));
            }
    }
    
    latest.map(|(_, msg)| msg.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::create_document_push;

    #[tokio::test]
    async fn test_debounce_queue() {
        let queue = DebounceQueue::new(100);
        let sender = queue.sender();
        
        let msg1 = create_document_push("file:///test.thy", "text1", "s1", 1);
        let msg2 = create_document_push("file:///test.thy", "text2", "s1", 2);
        
        sender.send(msg1).await.unwrap();
        sender.send(msg2).await.unwrap();
        
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    #[test]
    fn test_merge_documents() {
        let msg1 = create_document_push("file:///test.thy", "text1", "s1", 1);
        let msg2 = create_document_push("file:///test.thy", "text2", "s1", 2);
        
        let merged = merge_documents(vec![msg1.clone(), msg2.clone()]).unwrap();
        assert_eq!(merged.version, Some(2));
    }
}
