use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum EventType {
    Track,
    Identify,
    Error,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventContext {
    pub viewport: String,
    pub url: String,
    pub session_id: String,
    pub app_version: String,
    pub timestamp: i64,
    /// Enriched server-side from CF-Connecting-IP / X-Forwarded-For
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<String>,
    /// Enriched server-side from User-Agent header
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    /// ISO 3166-1 alpha-2 country from geo lookup
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub referrer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub utm_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub utm_medium: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub utm_campaign: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub utm_term: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub utm_content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedError {
    pub message: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchedEvent {
    #[serde(rename = "type")]
    pub event_type: EventType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub traits: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<SerializedError>,
    pub context: EventContext,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestBatch {
    pub batch: Vec<BatchedEvent>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../../packages/schema/src/fixtures/valid_batch.json");

    #[test]
    fn roundtrip_fixture() {
        let batch: IngestBatch = serde_json::from_str(FIXTURE).expect("fixture must deserialize");
        let re_serialized = serde_json::to_string(&batch).expect("must serialize");
        let batch2: IngestBatch =
            serde_json::from_str(&re_serialized).expect("re-deserialization must succeed");
        assert_eq!(batch.batch.len(), batch2.batch.len());
        assert_eq!(batch.batch[0].event, batch2.batch[0].event);
    }

    #[test]
    fn event_types_deserialize_correctly() {
        let json = r#"{"type":"track","context":{"viewport":"390x844","url":"/","sessionId":"anon_x","appVersion":"v1","timestamp":1000}}"#;
        let event: BatchedEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.event_type, EventType::Track);
    }

    #[test]
    fn optional_fields_round_trip_absent() {
        let json = r#"{"type":"error","error":{"message":"oops","name":"Error"},"context":{"viewport":"1x1","url":"/","sessionId":"anon_y","appVersion":"dev","timestamp":0}}"#;
        let event: BatchedEvent = serde_json::from_str(json).unwrap();
        let out = serde_json::to_string(&event).unwrap();
        assert!(!out.contains("\"ip\""), "ip should be absent");
        assert!(!out.contains("\"stack\""), "stack should be absent");
    }
}
