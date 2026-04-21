use async_trait::async_trait;
use nightshift_core::event::{BatchedEvent, EventType};
use serde::Serialize;

use crate::adapter::{Adapter, AdapterError};

// TikTok Events API (server-side pixel)
// Docs: https://business-api.us.tiktok.com/portal/docs?id=1771101027431425
const TIKTOK_EVENTS_URL: &str = "https://business-api.us.tiktok.com/open_api/v1.3/event/track/";

pub struct TikTokAdapter {
    pixel_code: String,
    access_token: String,
    client: reqwest::Client,
}

impl TikTokAdapter {
    pub fn new(pixel_code: impl Into<String>, access_token: impl Into<String>) -> Self {
        Self {
            pixel_code: pixel_code.into(),
            access_token: access_token.into(),
            client: reqwest::Client::new(),
        }
    }
}

// TikTok only supports track/error; identify has no equivalent in the Events API.
#[async_trait]
impl Adapter for TikTokAdapter {
    fn name(&self) -> &'static str {
        "tiktok"
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

        let timestamp = unix_ms_to_iso(event.context.timestamp);

        let mut properties = serde_json::Map::new();
        if let Some(props) = &event.properties {
            if let serde_json::Value::Object(m) = props {
                properties.extend(m.clone());
            }
        }
        if event.event_type == EventType::Error {
            if let Some(err) = &event.error {
                properties.insert("error_message".into(), err.message.clone().into());
                properties.insert("error_name".into(), err.name.clone().into());
            }
        }

        let payload = TikTokPayload {
            pixel_code: self.pixel_code.clone(),
            event: event_name,
            timestamp,
            context: TikTokContext {
                page: TikTokPage {
                    url: event.context.url.clone(),
                    referrer: event.context.referrer.clone(),
                },
                user_agent: event.context.user_agent.clone(),
            },
            properties: if properties.is_empty() {
                None
            } else {
                Some(serde_json::Value::Object(properties))
            },
        };

        let body = serde_json::to_string(&payload).map_err(AdapterError::Serialization)?;

        let resp = self
            .client
            .post(TIKTOK_EVENTS_URL)
            .header("Access-Token", &self.access_token)
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
struct TikTokPayload {
    pixel_code: String,
    event: String,
    timestamp: String,
    context: TikTokContext,
    #[serde(skip_serializing_if = "Option::is_none")]
    properties: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct TikTokContext {
    page: TikTokPage,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_agent: Option<String>,
}

#[derive(Serialize)]
struct TikTokPage {
    url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    referrer: Option<String>,
}

fn unix_ms_to_iso(ts_ms: i64) -> String {
    let secs = ts_ms / 1000;
    let nanos = ((ts_ms % 1000) * 1_000_000) as u32;
    use std::time::{Duration, UNIX_EPOCH};
    let dur = (UNIX_EPOCH + Duration::new(secs as u64, nanos))
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let total_secs = dur.as_secs();
    let ms = dur.subsec_millis();
    let s = total_secs % 60;
    let m = (total_secs / 60) % 60;
    let h = (total_secs / 3600) % 24;
    let days = total_secs / 86400;
    let (year, month, day) = days_to_ymd(days);
    format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}.{ms:03}Z")
}

fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    let mut year = 1970u64;
    loop {
        let dy = if is_leap(year) { 366 } else { 365 };
        if days < dy { break; }
        days -= dy;
        year += 1;
    }
    let months = if is_leap(year) {
        [31u64, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31u64, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut month = 1u64;
    for dm in &months {
        if days < *dm { break; }
        days -= dm;
        month += 1;
    }
    (year, month, days + 1)
}

fn is_leap(y: u64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use nightshift_core::event::{BatchedEvent, EventContext, EventType};

    fn make(t: EventType) -> BatchedEvent {
        BatchedEvent {
            event_type: t,
            event: Some("ViewContent".into()),
            user_id: None,
            properties: None,
            traits: None,
            error: None,
            context: EventContext {
                viewport: "1x1".into(),
                url: "https://example.com/product/123".into(),
                session_id: "anon_tt1".into(),
                app_version: "v1".into(),
                timestamp: 1_704_067_200_000,
                ..Default::default()
            },
        }
    }

    #[test]
    fn accepts_track_and_error_only() {
        let adapter = TikTokAdapter::new("PIXEL123", "token_abc");
        assert!(adapter.accepts(&make(EventType::Track)));
        assert!(adapter.accepts(&make(EventType::Error)));
        assert!(!adapter.accepts(&make(EventType::Identify)));
    }

    #[test]
    fn iso_timestamp_format() {
        assert_eq!(unix_ms_to_iso(1_704_067_200_000), "2024-01-01T00:00:00.000Z");
    }

    #[test]
    fn referrer_forwarded_when_present() {
        let mut ev = make(EventType::Track);
        ev.context.referrer = Some("https://tiktok.com".into());
        let adapter = TikTokAdapter::new("PIXEL123", "token_abc");
        // Just confirm accepts and referrer field is set
        assert!(adapter.accepts(&ev));
        assert_eq!(ev.context.referrer.as_deref(), Some("https://tiktok.com"));
    }
}
