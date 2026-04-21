use async_trait::async_trait;
use nightshift_core::event::{BatchedEvent, EventType};
use serde::Serialize;

use crate::adapter::{Adapter, AdapterError};

const TRACK_URL: &str = "https://api.mixpanel.com/track";
const ENGAGE_URL: &str = "https://api.mixpanel.com/engage";

pub struct MixpanelAdapter {
    token: String,
    client: reqwest::Client,
}

impl MixpanelAdapter {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            client: reqwest::Client::new(),
        }
    }
}

#[derive(Serialize)]
struct MixpanelEvent {
    event: String,
    properties: serde_json::Value,
}

#[derive(Serialize)]
struct MixpanelProfile {
    #[serde(rename = "$token")]
    token: String,
    #[serde(rename = "$distinct_id")]
    distinct_id: String,
    #[serde(rename = "$set")]
    set: serde_json::Value,
}

#[async_trait]
impl Adapter for MixpanelAdapter {
    fn name(&self) -> &'static str {
        "mixpanel"
    }

    async fn send(&self, event: &BatchedEvent) -> Result<(), AdapterError> {
        match event.event_type {
            EventType::Track | EventType::Error => self.send_track(event).await,
            EventType::Identify => self.send_engage(event).await,
        }
    }
}

impl MixpanelAdapter {
    async fn send_track(&self, event: &BatchedEvent) -> Result<(), AdapterError> {
        let event_name = match event.event_type {
            EventType::Error => "$error".to_string(),
            _ => event.event.clone().unwrap_or_else(|| "unknown".to_string()),
        };

        // Merge properties with required Mixpanel fields
        let mut props = match &event.properties {
            Some(serde_json::Value::Object(m)) => m.clone(),
            _ => serde_json::Map::new(),
        };

        // Required Mixpanel fields
        props.insert("token".into(), serde_json::Value::String(self.token.clone()));
        props.insert(
            "distinct_id".into(),
            serde_json::Value::String(event.context.session_id.clone()),
        );
        props.insert(
            "time".into(),
            serde_json::Value::Number(
                serde_json::Number::from(event.context.timestamp / 1000),
            ),
        );
        props.insert("$browser_size".into(), serde_json::Value::String(event.context.viewport.clone()));
        props.insert("$current_url".into(), serde_json::Value::String(event.context.url.clone()));
        props.insert("app_version".into(), serde_json::Value::String(event.context.app_version.clone()));

        if let EventType::Error = event.event_type {
            if let Some(err) = &event.error {
                props.insert("$error_message".into(), serde_json::Value::String(err.message.clone()));
                props.insert("$error_type".into(), serde_json::Value::String(err.name.clone()));
            }
        }

        let payload = vec![MixpanelEvent {
            event: event_name,
            properties: serde_json::Value::Object(props),
        }];

        let body = serde_json::to_string(&payload).map_err(AdapterError::Serialization)?;

        let resp = self
            .client
            .post(TRACK_URL)
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

    async fn send_engage(&self, event: &BatchedEvent) -> Result<(), AdapterError> {
        let distinct_id = event
            .user_id
            .clone()
            .unwrap_or_else(|| event.context.session_id.clone());

        let traits = event
            .traits
            .clone()
            .unwrap_or(serde_json::Value::Object(Default::default()));

        let profile = MixpanelProfile {
            token: self.token.clone(),
            distinct_id,
            set: traits,
        };

        let body = serde_json::to_string(&vec![profile]).map_err(AdapterError::Serialization)?;

        let resp = self
            .client
            .post(ENGAGE_URL)
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
    use nightshift_core::event::{BatchedEvent, EventContext, EventType};

    fn make_event(t: EventType) -> BatchedEvent {
        BatchedEvent {
            event_type: t,
            event: Some("TestEvent".into()),
            user_id: None,
            properties: None,
            traits: None,
            error: None,
            context: EventContext {
                viewport: "390x844".into(),
                url: "/test".into(),
                session_id: "anon_mp1".into(),
                app_version: "v1".into(),
                timestamp: 1_000_000,
                ..Default::default()
            },
        }
    }

    #[test]
    fn accepts_all_event_types() {
        let adapter = MixpanelAdapter::new("token123");
        assert!(adapter.accepts(&make_event(EventType::Track)));
        assert!(adapter.accepts(&make_event(EventType::Identify)));
        assert!(adapter.accepts(&make_event(EventType::Error)));
    }
}
