use async_trait::async_trait;
use nightshift_core::event::{BatchedEvent, EventType};
use serde::Serialize;

use crate::adapter::{Adapter, AdapterError};

const SEGMENT_URL: &str = "https://api.segment.io/v1/batch";

pub struct SegmentAdapter {
    write_key: String,
    client: reqwest::Client,
}

impl SegmentAdapter {
    pub fn new(write_key: impl Into<String>) -> Self {
        Self {
            write_key: write_key.into(),
            client: reqwest::Client::new(),
        }
    }
}

#[derive(Serialize)]
struct SegmentBatch {
    batch: Vec<SegmentCall>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum SegmentCall {
    Track {
        #[serde(rename = "userId")]
        user_id: String,
        event: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        properties: Option<serde_json::Value>,
        context: SegmentContext,
        timestamp: String,
    },
    Identify {
        #[serde(rename = "userId")]
        user_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        traits: Option<serde_json::Value>,
        context: SegmentContext,
        timestamp: String,
    },
}

#[derive(Serialize)]
struct SegmentContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    page: Option<SegmentPage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    referrer: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    campaign: Option<serde_json::Value>,
    library: SegmentLibrary,
}

#[derive(Serialize)]
struct SegmentPage {
    url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    referrer: Option<String>,
}

#[derive(Serialize)]
struct SegmentLibrary {
    name: &'static str,
    version: &'static str,
}

fn unix_ms_to_iso(ts_ms: i64) -> String {
    let secs = ts_ms / 1000;
    let ms = ts_ms % 1000;
    let nanos = (ms * 1_000_000) as u32;
    use std::time::{Duration, UNIX_EPOCH};
    let t = UNIX_EPOCH + Duration::new(secs as u64, nanos);
    let dur = t.duration_since(UNIX_EPOCH).unwrap_or_default();
    let total_secs = dur.as_secs();
    let subsec_ms = dur.subsec_millis();
    let s = total_secs % 60;
    let m = (total_secs / 60) % 60;
    let h = (total_secs / 3600) % 24;
    let days = total_secs / 86400;
    let (year, month, day) = days_to_ymd(days);
    format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}.{subsec_ms:03}Z")
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

fn build_context(event: &BatchedEvent) -> SegmentContext {
    let campaign = if event.context.utm_source.is_some()
        || event.context.utm_medium.is_some()
        || event.context.utm_campaign.is_some()
    {
        let mut m = serde_json::Map::new();
        if let Some(v) = &event.context.utm_source {
            m.insert("source".into(), serde_json::Value::String(v.clone()));
        }
        if let Some(v) = &event.context.utm_medium {
            m.insert("medium".into(), serde_json::Value::String(v.clone()));
        }
        if let Some(v) = &event.context.utm_campaign {
            m.insert("name".into(), serde_json::Value::String(v.clone()));
        }
        if let Some(v) = &event.context.utm_term {
            m.insert("term".into(), serde_json::Value::String(v.clone()));
        }
        if let Some(v) = &event.context.utm_content {
            m.insert("content".into(), serde_json::Value::String(v.clone()));
        }
        Some(serde_json::Value::Object(m))
    } else {
        None
    };

    SegmentContext {
        page: Some(SegmentPage {
            url: event.context.url.clone(),
            title: event.context.page_title.clone(),
            referrer: event.context.referrer.clone(),
        }),
        referrer: event.context.referrer.as_ref().map(|r| {
            serde_json::json!({ "url": r })
        }),
        campaign,
        library: SegmentLibrary { name: "nightshift", version: "0.1.0" },
    }
}

#[async_trait]
impl Adapter for SegmentAdapter {
    fn name(&self) -> &'static str {
        "segment"
    }

    async fn send(&self, event: &BatchedEvent) -> Result<(), AdapterError> {
        let user_id = event
            .user_id
            .clone()
            .unwrap_or_else(|| event.context.session_id.clone());
        let timestamp = unix_ms_to_iso(event.context.timestamp);
        let ctx = build_context(event);

        let call = match event.event_type {
            EventType::Track => SegmentCall::Track {
                user_id,
                event: event.event.clone().unwrap_or_else(|| "unknown".to_string()),
                properties: event.properties.clone(),
                context: ctx,
                timestamp,
            },
            EventType::Identify => SegmentCall::Identify {
                user_id,
                traits: event.traits.clone(),
                context: ctx,
                timestamp,
            },
            EventType::Error => {
                // Map error events to a Track call
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
                SegmentCall::Track {
                    user_id,
                    event: "Error".to_string(),
                    properties: Some(props),
                    context: ctx,
                    timestamp,
                }
            }
        };

        let payload = SegmentBatch { batch: vec![call] };
        let body = serde_json::to_string(&payload).map_err(AdapterError::Serialization)?;

        let resp = self
            .client
            .post(SEGMENT_URL)
            .basic_auth(&self.write_key, Option::<&str>::None)
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
                session_id: "anon_seg1".into(),
                app_version: "v1".into(),
                timestamp: 0,
                ..Default::default()
            },
        }
    }

    #[test]
    fn segment_accepts_all_types() {
        let adapter = SegmentAdapter::new("seg_write_key");
        assert!(adapter.accepts(&make(EventType::Track)));
        assert!(adapter.accepts(&make(EventType::Identify)));
        assert!(adapter.accepts(&make(EventType::Error)));
    }

    #[test]
    fn utm_fields_map_to_campaign_context() {
        let mut ev = make(EventType::Track);
        ev.context.utm_source = Some("google".into());
        ev.context.utm_medium = Some("cpc".into());
        ev.context.utm_campaign = Some("spring_sale".into());
        let ctx = build_context(&ev);
        let campaign = ctx.campaign.unwrap();
        assert_eq!(campaign["source"], "google");
        assert_eq!(campaign["medium"], "cpc");
        assert_eq!(campaign["name"], "spring_sale");
    }

    #[test]
    fn iso_timestamp_roundtrip() {
        assert_eq!(unix_ms_to_iso(1_704_067_200_000), "2024-01-01T00:00:00.000Z");
    }
}
