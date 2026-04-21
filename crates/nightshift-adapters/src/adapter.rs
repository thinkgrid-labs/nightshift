use async_trait::async_trait;
use futures::future::join_all;
use nightshift_core::event::BatchedEvent;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AdapterError {
    #[error("HTTP {status}: {body}")]
    Http { status: u16, body: String },
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("network error: {0}")]
    Network(String),
    #[error("config error: {0}")]
    Config(String),
}

impl AdapterError {
    pub fn is_retryable(&self) -> bool {
        match self {
            AdapterError::Network(_) => true,
            AdapterError::Http { status, .. } => *status >= 500 || *status == 429,
            _ => false,
        }
    }
}

#[async_trait]
pub trait Adapter: Send + Sync {
    fn name(&self) -> &'static str;

    fn accepts(&self, _event: &BatchedEvent) -> bool {
        true
    }

    async fn send(&self, event: &BatchedEvent) -> Result<(), AdapterError>;
}

async fn send_with_retry(adapter: &dyn Adapter, event: &BatchedEvent) {
    let mut delay_ms = 100u64;
    for attempt in 0..3u8 {
        match adapter.send(event).await {
            Ok(_) => return,
            Err(e) if e.is_retryable() && attempt < 2 => {
                tracing::warn!(
                    adapter = adapter.name(),
                    attempt,
                    error = %e,
                    "transient adapter error, retrying"
                );
                #[cfg(not(target_arch = "wasm32"))]
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                delay_ms *= 2;
            }
            Err(e) => {
                tracing::error!(
                    adapter = adapter.name(),
                    attempt,
                    error = %e,
                    "adapter failed permanently"
                );
                return;
            }
        }
    }
}

pub struct AdapterRouter {
    adapters: Vec<Box<dyn Adapter>>,
}

impl AdapterRouter {
    pub fn new(adapters: Vec<Box<dyn Adapter>>) -> Self {
        Self { adapters }
    }

    pub async fn route(&self, events: Vec<BatchedEvent>) {
        let futs: Vec<_> = events
            .iter()
            .flat_map(|event| {
                self.adapters
                    .iter()
                    .filter(|a| a.accepts(event))
                    .map(move |a| send_with_retry(a.as_ref(), event))
            })
            .collect();

        join_all(futs).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nightshift_core::event::{BatchedEvent, EventContext, EventType};
    use std::sync::{Arc, Mutex};

    struct RecordingAdapter {
        calls: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl Adapter for RecordingAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }
        async fn send(&self, event: &BatchedEvent) -> Result<(), AdapterError> {
            self.calls
                .lock()
                .unwrap()
                .push(event.event.clone().unwrap_or_default());
            Ok(())
        }
    }

    struct FailingAdapter;

    #[async_trait]
    impl Adapter for FailingAdapter {
        fn name(&self) -> &'static str {
            "failing"
        }
        async fn send(&self, _event: &BatchedEvent) -> Result<(), AdapterError> {
            Err(AdapterError::Network("always fails".to_string()))
        }
    }

    struct FlakyAdapter {
        calls: Arc<Mutex<u32>>,
        fail_times: u32,
    }

    #[async_trait]
    impl Adapter for FlakyAdapter {
        fn name(&self) -> &'static str {
            "flaky"
        }
        async fn send(&self, _event: &BatchedEvent) -> Result<(), AdapterError> {
            let mut count = self.calls.lock().unwrap();
            *count += 1;
            if *count <= self.fail_times {
                Err(AdapterError::Http { status: 503, body: "unavailable".into() })
            } else {
                Ok(())
            }
        }
    }

    fn make_event(name: &str) -> BatchedEvent {
        BatchedEvent {
            event_type: EventType::Track,
            event: Some(name.to_string()),
            user_id: None,
            properties: None,
            traits: None,
            error: None,
            context: EventContext {
                viewport: "1x1".to_string(),
                url: "/".to_string(),
                session_id: "anon_test".to_string(),
                app_version: "v1".to_string(),
                timestamp: 0,
                ..Default::default()
            },
        }
    }

    #[tokio::test]
    async fn routes_to_all_adapters() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let router = AdapterRouter::new(vec![
            Box::new(RecordingAdapter { calls: calls.clone() }),
            Box::new(RecordingAdapter { calls: calls.clone() }),
        ]);
        router.route(vec![make_event("Click")]).await;
        assert_eq!(calls.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn failing_adapter_does_not_abort_others() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let router = AdapterRouter::new(vec![
            Box::new(FailingAdapter),
            Box::new(RecordingAdapter { calls: calls.clone() }),
        ]);
        router.route(vec![make_event("Click")]).await;
        assert_eq!(calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn retries_on_transient_5xx() {
        let calls = Arc::new(Mutex::new(0u32));
        let router = AdapterRouter::new(vec![Box::new(FlakyAdapter {
            calls: calls.clone(),
            fail_times: 2,
        })]);
        router.route(vec![make_event("Click")]).await;
        assert_eq!(*calls.lock().unwrap(), 3); // 2 failures + 1 success
    }

    #[tokio::test]
    async fn does_not_retry_4xx() {
        // Use a separate adapter that always returns 400 (non-retryable)
        struct BadRequestAdapter { calls: Arc<Mutex<u32>> }
        #[async_trait]
        impl Adapter for BadRequestAdapter {
            fn name(&self) -> &'static str { "bad-request" }
            async fn send(&self, _: &BatchedEvent) -> Result<(), AdapterError> {
                *self.calls.lock().unwrap() += 1;
                Err(AdapterError::Http { status: 400, body: "bad request".into() })
            }
        }
        let calls2 = Arc::new(Mutex::new(0u32));
        let router2 = AdapterRouter::new(vec![Box::new(BadRequestAdapter { calls: calls2.clone() })]);
        router2.route(vec![make_event("Click")]).await;
        assert_eq!(*calls2.lock().unwrap(), 1); // only tried once
    }

    #[tokio::test]
    async fn accepts_filter_skips_adapter() {
        struct TrackOnlyAdapter {
            calls: Arc<Mutex<Vec<String>>>,
        }

        #[async_trait]
        impl Adapter for TrackOnlyAdapter {
            fn name(&self) -> &'static str {
                "track-only"
            }
            fn accepts(&self, event: &BatchedEvent) -> bool {
                matches!(event.event_type, EventType::Track)
            }
            async fn send(&self, event: &BatchedEvent) -> Result<(), AdapterError> {
                self.calls
                    .lock()
                    .unwrap()
                    .push(event.event.clone().unwrap_or_default());
                Ok(())
            }
        }

        let calls = Arc::new(Mutex::new(Vec::new()));
        let router = AdapterRouter::new(vec![Box::new(TrackOnlyAdapter {
            calls: calls.clone(),
        })]);

        let mut error_event = make_event("oops");
        error_event.event_type = EventType::Error;

        router.route(vec![make_event("Click"), error_event]).await;

        assert_eq!(calls.lock().unwrap().len(), 1);
    }
}
