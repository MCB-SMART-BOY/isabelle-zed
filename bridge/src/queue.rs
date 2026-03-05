use crate::protocol::{Message, MessageType};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum QueueError {
    #[error("failed to parse document.push payload: {0}")]
    InvalidPushPayload(String),
}

#[derive(Debug, Clone)]
struct Pending {
    message: Message,
    queued_at: Instant,
}

#[derive(Debug)]
pub struct DebounceQueue {
    debounce: Duration,
    pending_by_uri: HashMap<String, Pending>,
}

impl DebounceQueue {
    pub fn new(debounce_ms: u64) -> Self {
        Self {
            debounce: Duration::from_millis(debounce_ms),
            pending_by_uri: HashMap::new(),
        }
    }

    pub fn enqueue(&mut self, message: Message) -> Result<(), QueueError> {
        if message.msg_type != MessageType::DocumentPush {
            return Ok(());
        }

        let payload = message
            .push_payload()
            .map_err(|err| QueueError::InvalidPushPayload(err.to_string()))?;

        let queued_at = Instant::now();
        match self.pending_by_uri.get_mut(&payload.uri) {
            Some(existing) if existing.message.version > message.version => {}
            Some(existing) => {
                existing.message = message;
                existing.queued_at = queued_at;
            }
            None => {
                self.pending_by_uri
                    .insert(payload.uri, Pending { message, queued_at });
            }
        }

        Ok(())
    }

    pub fn drain_ready(&mut self, now: Instant) -> Vec<Message> {
        let mut ready = Vec::new();
        self.pending_by_uri.retain(|_, pending| {
            if now.duration_since(pending.queued_at) >= self.debounce {
                ready.push(pending.message.clone());
                false
            } else {
                true
            }
        });
        ready
    }

    pub fn drain_all(&mut self) -> Vec<Message> {
        self.pending_by_uri
            .drain()
            .map(|(_, pending)| pending.message)
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.pending_by_uri.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::DebounceQueue;
    use crate::protocol::{Message, MessageType};
    use std::thread;
    use std::time::{Duration, Instant};

    fn push(uri: &str, version: i64, text: &str) -> Message {
        Message {
            id: format!("msg-{version:04}"),
            msg_type: MessageType::DocumentPush,
            session: "s1".to_string(),
            version,
            payload: serde_json::json!({
                "uri": uri,
                "text": text,
            }),
        }
    }

    #[test]
    fn debounce_keeps_latest_push_per_uri() {
        let mut queue = DebounceQueue::new(300);
        queue.enqueue(push("file:///a.thy", 1, "old")).unwrap();
        queue.enqueue(push("file:///a.thy", 2, "new")).unwrap();

        let ready = queue.drain_ready(Instant::now() + Duration::from_millis(350));
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].version, 2);
    }

    #[test]
    fn debounce_isolated_per_document_uri() {
        let mut queue = DebounceQueue::new(300);
        queue.enqueue(push("file:///a.thy", 1, "a")).unwrap();
        queue.enqueue(push("file:///b.thy", 1, "b")).unwrap();

        let ready = queue.drain_ready(Instant::now() + Duration::from_millis(350));
        assert_eq!(ready.len(), 2);
    }

    #[test]
    fn non_push_messages_are_ignored() {
        let mut queue = DebounceQueue::new(300);
        queue
            .enqueue(Message {
                id: "msg-0001".to_string(),
                msg_type: MessageType::DocumentCheck,
                session: "s1".to_string(),
                version: 1,
                payload: serde_json::json!({
                    "uri": "file:///a.thy",
                    "version": 1,
                }),
            })
            .unwrap();

        assert!(queue.is_empty());
    }

    #[test]
    fn stale_push_does_not_delay_newer_pending_message() {
        let mut queue = DebounceQueue::new(60);
        queue.enqueue(push("file:///a.thy", 2, "new")).unwrap();

        thread::sleep(Duration::from_millis(40));
        queue.enqueue(push("file:///a.thy", 1, "old")).unwrap();
        thread::sleep(Duration::from_millis(30));

        let ready = queue.drain_ready(Instant::now());
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].version, 2);
    }

    #[test]
    fn drain_all_returns_pending_messages_immediately() {
        let mut queue = DebounceQueue::new(10_000);
        queue.enqueue(push("file:///a.thy", 1, "a")).unwrap();
        queue.enqueue(push("file:///b.thy", 1, "b")).unwrap();

        let drained = queue.drain_all();
        assert_eq!(drained.len(), 2);
        assert!(queue.is_empty());
    }
}
