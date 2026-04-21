use async_trait::async_trait;
use nightshift_core::event::{BatchedEvent, EventType};
use serde::Serialize;

use crate::adapter::{Adapter, AdapterError};

// Facebook Conversions API (server-side pixel)
// Docs: https://developers.facebook.com/docs/marketing-api/conversions-api
const FB_CAPI_BASE: &str = "https://graph.facebook.com/v18.0";

pub struct FacebookAdapter {
    pixel_id: String,
    access_token: String,
    client: reqwest::Client,
}

impl FacebookAdapter {
    pub fn new(pixel_id: impl Into<String>, access_token: impl Into<String>) -> Self {
        Self {
            pixel_id: pixel_id.into(),
            access_token: access_token.into(),
            client: reqwest::Client::new(),
        }
    }

    fn endpoint(&self) -> String {
        format!("{}/{}/events", FB_CAPI_BASE, self.pixel_id)
    }
}

// Facebook only handles track and error events; identify has no server-side equivalent.
#[async_trait]
impl Adapter for FacebookAdapter {
    fn name(&self) -> &'static str {
        "facebook"
    }

    fn accepts(&self, event: &BatchedEvent) -> bool {
        !matches!(event.event_type, EventType::Identify)
    }

    async fn send(&self, event: &BatchedEvent) -> Result<(), AdapterError> {
        let event_name = match event.event_type {
            EventType::Track => event.event.clone().unwrap_or_else(|| "CustomEvent".to_string()),
            EventType::Error => "Error".to_string(),
            EventType::Identify => unreachable!("accepts() filters these out"),
        };

        // event_time must be Unix seconds (not ms)
        let event_time = event.context.timestamp / 1000;

        let mut custom_data = serde_json::Map::new();
        if let Some(props) = &event.properties {
            if let serde_json::Value::Object(m) = props {
                custom_data.extend(m.clone());
            }
        }
        if event.event_type == EventType::Error {
            if let Some(err) = &event.error {
                custom_data.insert("error_message".into(), err.message.clone().into());
                custom_data.insert("error_name".into(), err.name.clone().into());
            }
        }

        let fb_event = FbEvent {
            event_name,
            event_time,
            action_source: "website",
            event_source_url: Some(event.context.url.clone()),
            user_data: FbUserData {
                client_ip_address: None, // already stripped by PII sanitizer
                client_user_agent: event.context.user_agent.clone(),
                fbp: None,
                fbc: None,
            },
            custom_data: if custom_data.is_empty() {
                None
            } else {
                Some(serde_json::Value::Object(custom_data))
            },
        };

        let payload = FbPayload { data: vec![fb_event] };
        let body = serde_json::to_string(&payload).map_err(AdapterError::Serialization)?;

        let resp = self
            .client
            .post(self.endpoint())
            .query(&[("access_token", &self.access_token)])
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

#[derive(Serialize)]
struct FbPayload {
    data: Vec<FbEvent>,
}

#[derive(Serialize)]
struct FbEvent {
    event_name: String,
    event_time: i64,
    action_source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    event_source_url: Option<String>,
    user_data: FbUserData,
    #[serde(skip_serializing_if = "Option::is_none")]
    custom_data: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct FbUserData {
    // IP is intentionally not forwarded (stripped by PII sanitizer upstream)
    #[serde(skip_serializing_if = "Option::is_none")]
    client_ip_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_user_agent: Option<String>,
    // Browser-side cookie values — clients may pass these as properties
    #[serde(skip_serializing_if = "Option::is_none")]
    fbp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fbc: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use nightshift_core::event::{BatchedEvent, EventContext, EventType};

    fn make(t: EventType) -> BatchedEvent {
        BatchedEvent {
            event_type: t,
            event: Some("AddToCart".into()),
            user_id: None,
            properties: None,
            traits: None,
            error: None,
            context: EventContext {
                viewport: "1x1".into(),
                url: "https://example.com/cart".into(),
                session_id: "anon_fb1".into(),
                app_version: "v1".into(),
                timestamp: 1_704_067_200_000,
                ..Default::default()
            },
        }
    }

    #[test]
    fn accepts_track_and_error_only() {
        let adapter = FacebookAdapter::new("123456789", "EAAtoken");
        assert!(adapter.accepts(&make(EventType::Track)));
        assert!(adapter.accepts(&make(EventType::Error)));
        assert!(!adapter.accepts(&make(EventType::Identify)));
    }

    #[test]
    fn event_time_is_unix_seconds() {
        // 1_704_067_200_000 ms → 1_704_067_200 s
        let event = make(EventType::Track);
        assert_eq!(event.context.timestamp / 1000, 1_704_067_200);
    }
}
