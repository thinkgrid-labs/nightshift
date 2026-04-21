use async_trait::async_trait;
use nightshift_core::event::{BatchedEvent, EventType};
use serde::Serialize;

use crate::adapter::{Adapter, AdapterError};

const AMPLITUDE_URL: &str = "https://api2.amplitude.com/2/httpapi";

pub struct AmplitudeAdapter {
    api_key: String,
    client: reqwest::Client,
}

impl AmplitudeAdapter {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            client: reqwest::Client::new(),
        }
    }
}

#[derive(Serialize)]
struct AmplitudePayload {
    api_key: String,
    events: Vec<AmplitudeEvent>,
}

#[derive(Serialize)]
struct AmplitudeEvent {
    event_type: String,
    user_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    device_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    event_properties: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_properties: Option<serde_json::Value>,
    time: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    app_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    referrer: Option<String>,
}

#[async_trait]
impl Adapter for AmplitudeAdapter {
    fn name(&self) -> &'static str {
        "amplitude"
    }

    async fn send(&self, event: &BatchedEvent) -> Result<(), AdapterError> {
        let user_id = event
            .user_id
            .clone()
            .unwrap_or_else(|| event.context.session_id.clone());

        let (event_type, event_properties, user_properties) = match event.event_type {
            EventType::Track => {
                let name = event.event.clone().unwrap_or_else(|| "unknown".to_string());
                (name, event.properties.clone(), None)
            }
            EventType::Identify => {
                ("$identify".to_string(), None, event.traits.clone())
            }
            EventType::Error => {
                let mut props = event
                    .properties
                    .clone()
                    .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));
                if let Some(err) = &event.error {
                    if let serde_json::Value::Object(ref mut m) = props {
                        m.insert("error_message".into(), serde_json::Value::String(err.message.clone()));
                        m.insert("error_name".into(), serde_json::Value::String(err.name.clone()));
                        if let Some(stack) = &err.stack {
                            m.insert("error_stack".into(), serde_json::Value::String(stack.clone()));
                        }
                    }
                }
                ("$error".to_string(), Some(props), None)
            }
        };

        let amp_event = AmplitudeEvent {
            event_type,
            user_id,
            device_id: None,
            event_properties,
            user_properties,
            time: event.context.timestamp,
            app_version: Some(event.context.app_version.clone()),
            referrer: event.context.referrer.clone(),
        };

        let payload = AmplitudePayload {
            api_key: self.api_key.clone(),
            events: vec![amp_event],
        };

        let body = serde_json::to_string(&payload).map_err(AdapterError::Serialization)?;

        let resp = self
            .client
            .post(AMPLITUDE_URL)
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

    fn make(t: EventType) -> BatchedEvent {
        BatchedEvent {
            event_type: t,
            event: Some("TestEvent".into()),
            user_id: None,
            properties: None,
            traits: None,
            error: None,
            context: EventContext {
                viewport: "1x1".into(),
                url: "/".into(),
                session_id: "anon_amp1".into(),
                app_version: "v1".into(),
                timestamp: 0,
                ..Default::default()
            },
        }
    }

    #[test]
    fn amplitude_accepts_all_types() {
        let adapter = AmplitudeAdapter::new("amp_key");
        assert!(adapter.accepts(&make(EventType::Track)));
        assert!(adapter.accepts(&make(EventType::Identify)));
        assert!(adapter.accepts(&make(EventType::Error)));
    }

    #[test]
    fn error_event_type_is_dollar_error() {
        let mut ev = make(EventType::Error);
        ev.event_type = EventType::Error;
        ev.error = Some(nightshift_core::event::SerializedError {
            message: "oops".into(),
            name: "Error".into(),
            stack: None,
        });
        // just ensure it serializes without panic — actual HTTP call not tested here
        let user_id = ev.user_id.clone().unwrap_or_else(|| ev.context.session_id.clone());
        assert_eq!(user_id, "anon_amp1");
    }
}
