use async_trait::async_trait;
use nightshift_core::event::{BatchedEvent, EventType};
use serde::Serialize;

use crate::adapter::{Adapter, AdapterError};

const POSTHOG_CAPTURE_URL: &str = "https://app.posthog.com/capture/";

pub struct PostHogAdapter {
    api_key: String,
    /// Override for self-hosted PostHog instances
    endpoint: String,
    client: reqwest::Client,
}

impl PostHogAdapter {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            endpoint: POSTHOG_CAPTURE_URL.to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// For self-hosted PostHog: e.g. "https://posthog.yourdomain.com/capture/"
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }
}

#[derive(Serialize)]
struct PostHogBatch {
    api_key: String,
    batch: Vec<PostHogEvent>,
}

#[derive(Serialize)]
struct PostHogEvent {
    event: String,
    distinct_id: String,
    timestamp: String,
    properties: serde_json::Value,
}

fn iso_timestamp(ts_ms: i64) -> String {
    // PostHog expects ISO 8601 — produce UTC from unix ms
    // Simple approach: format as RFC3339 manually from unix ms
    let secs = ts_ms / 1000;
    let nanos = ((ts_ms % 1000) * 1_000_000) as u32;
    use std::time::{Duration, UNIX_EPOCH};
    let t = UNIX_EPOCH + Duration::new(secs as u64, nanos);
    // Format: 2024-01-01T00:00:00.000Z
    let datetime = t.duration_since(UNIX_EPOCH).unwrap_or_default();
    let total_secs = datetime.as_secs();
    let ms = datetime.subsec_millis();
    let s = total_secs % 60;
    let m = (total_secs / 60) % 60;
    let h = (total_secs / 3600) % 24;
    let days = total_secs / 86400;
    // Simple Gregorian calendar calculation
    let (year, month, day) = days_to_ymd(days);
    format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}.{ms:03}Z")
}

fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    // Gregorian calendar from day 0 = 1970-01-01
    let mut year = 1970u64;
    loop {
        let dy = if is_leap(year) { 366 } else { 365 };
        if days < dy {
            break;
        }
        days -= dy;
        year += 1;
    }
    let months = if is_leap(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut month = 1u64;
    for dm in &months {
        if days < *dm {
            break;
        }
        days -= dm;
        month += 1;
    }
    (year, month, days + 1)
}

fn is_leap(y: u64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

#[async_trait]
impl Adapter for PostHogAdapter {
    fn name(&self) -> &'static str {
        "posthog"
    }

    async fn send(&self, event: &BatchedEvent) -> Result<(), AdapterError> {
        let event_name = match event.event_type {
            EventType::Track => event.event.clone().unwrap_or_else(|| "unknown".to_string()),
            EventType::Identify => "$identify".to_string(),
            EventType::Error => "$exception".to_string(),
        };

        let distinct_id = event
            .user_id
            .clone()
            .unwrap_or_else(|| event.context.session_id.clone());

        // Build properties
        let mut props = match event.event_type {
            EventType::Identify => {
                let mut p = serde_json::Map::new();
                if let Some(traits) = &event.traits {
                    p.insert("$set".into(), traits.clone());
                }
                p
            }
            _ => match &event.properties {
                Some(serde_json::Value::Object(m)) => m.clone(),
                _ => serde_json::Map::new(),
            },
        };

        // PostHog reserved property names
        props.insert("$screen_width".into(), {
            let vp: Vec<&str> = event.context.viewport.splitn(2, 'x').collect();
            vp.first()
                .and_then(|s| s.parse::<u64>().ok())
                .map(|n| serde_json::Value::Number(n.into()))
                .unwrap_or(serde_json::Value::Null)
        });
        props.insert("$current_url".into(), serde_json::Value::String(event.context.url.clone()));
        props.insert("$lib".into(), serde_json::Value::String("nightshift".into()));
        props.insert("$lib_version".into(), serde_json::Value::String("0.1.0".into()));
        props.insert("app_version".into(), serde_json::Value::String(event.context.app_version.clone()));

        if let EventType::Error = event.event_type {
            if let Some(err) = &event.error {
                props.insert("$exception_type".into(), serde_json::Value::String(err.name.clone()));
                props.insert("$exception_message".into(), serde_json::Value::String(err.message.clone()));
                if let Some(stack) = &err.stack {
                    props.insert("$exception_stack_trace_raw".into(), serde_json::Value::String(stack.clone()));
                }
            }
        }

        let batch = PostHogBatch {
            api_key: self.api_key.clone(),
            batch: vec![PostHogEvent {
                event: event_name,
                distinct_id,
                timestamp: iso_timestamp(event.context.timestamp),
                properties: serde_json::Value::Object(props),
            }],
        };

        let body = serde_json::to_string(&batch).map_err(AdapterError::Serialization)?;

        let resp = self
            .client
            .post(&self.endpoint)
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await
            .map_err(|e| AdapterError::Network(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(AdapterError::Http { status, body });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_timestamp_known_values() {
        // 2024-01-01T00:00:00.000Z = 1704067200000 ms
        assert_eq!(iso_timestamp(1_704_067_200_000), "2024-01-01T00:00:00.000Z");
        // 2024-01-15T12:30:45.500Z
        assert_eq!(iso_timestamp(1_705_321_845_500), "2024-01-15T12:30:45.500Z");
    }

    #[test]
    fn posthog_accepts_all_types() {
        use nightshift_core::event::{BatchedEvent, EventContext, EventType};
        let adapter = PostHogAdapter::new("phc_test");
        let make = |t: EventType| BatchedEvent {
            event_type: t,
            event: None,
            user_id: None,
            properties: None,
            traits: None,
            error: None,
            context: EventContext {
                viewport: "1x1".into(),
                url: "/".into(),
                session_id: "anon_x".into(),
                app_version: "v1".into(),
                timestamp: 0,
                ..Default::default()
            },
        };
        assert!(adapter.accepts(&make(EventType::Track)));
        assert!(adapter.accepts(&make(EventType::Identify)));
        assert!(adapter.accepts(&make(EventType::Error)));
    }
}
