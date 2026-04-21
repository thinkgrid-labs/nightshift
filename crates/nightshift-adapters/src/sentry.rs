use async_trait::async_trait;
use nightshift_core::event::{BatchedEvent, EventType};
use serde::Serialize;

use crate::adapter::{Adapter, AdapterError};

/// Sends error events to Sentry using the Envelope API.
/// https://develop.sentry.dev/sdk/envelopes/
pub struct SentryAdapter {
    dsn: SentryDsn,
    client: reqwest::Client,
    release: Option<String>,
    environment: Option<String>,
}

struct SentryDsn {
    public_key: String,
    host: String,
    project_id: String,
}

impl SentryDsn {
    /// Parses a Sentry DSN of the form: https://<key>@<host>/<project_id>
    fn parse(dsn: &str) -> Result<Self, AdapterError> {
        let url = url::Url::parse(dsn)
            .map_err(|e| AdapterError::Config(format!("invalid Sentry DSN: {e}")))?;

        let public_key = url.username().to_owned();
        if public_key.is_empty() {
            return Err(AdapterError::Config("Sentry DSN missing public key".into()));
        }

        let host = url
            .host_str()
            .ok_or_else(|| AdapterError::Config("Sentry DSN missing host".into()))?
            .to_owned();

        let project_id = url
            .path()
            .trim_start_matches('/')
            .to_owned();
        if project_id.is_empty() {
            return Err(AdapterError::Config("Sentry DSN missing project ID".into()));
        }

        Ok(Self { public_key, host, project_id })
    }

    fn envelope_url(&self) -> String {
        format!("https://{}/api/{}/envelope/", self.host, self.project_id)
    }

    fn auth_header(&self) -> String {
        format!(
            "Sentry sentry_version=7, sentry_client=nightshift/0.1.0, sentry_key={}",
            self.public_key
        )
    }
}

impl SentryAdapter {
    pub fn new(dsn: &str) -> Result<Self, AdapterError> {
        Ok(Self {
            dsn: SentryDsn::parse(dsn)?,
            client: reqwest::Client::new(),
            release: None,
            environment: None,
        })
    }

    pub fn with_release(mut self, release: impl Into<String>) -> Self {
        self.release = Some(release.into());
        self
    }

    pub fn with_environment(mut self, env: impl Into<String>) -> Self {
        self.environment = Some(env.into());
        self
    }
}

/// Minimal Sentry event — only what we can reconstruct from a BatchedEvent.
#[derive(Serialize)]
struct SentryEvent {
    event_id: String,
    timestamp: f64,
    platform: &'static str,
    level: &'static str,
    exception: SentryExceptions,
    tags: SentryTags,
    #[serde(skip_serializing_if = "Option::is_none")]
    release: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    environment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<SentryUser>,
}

#[derive(Serialize)]
struct SentryExceptions {
    values: Vec<SentryException>,
}

#[derive(Serialize)]
struct SentryException {
    r#type: String,
    value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    stacktrace: Option<SentryStacktrace>,
}

#[derive(Serialize)]
struct SentryStacktrace {
    /// Raw stack trace string — Sentry can demangle if source maps are uploaded.
    /// We send as a single synthetic frame with the raw stack.
    frames: Vec<SentryFrame>,
}

#[derive(Serialize)]
struct SentryFrame {
    #[serde(skip_serializing_if = "Option::is_none")]
    filename: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    function: Option<String>,
    raw_function: String,
}

#[derive(Serialize)]
struct SentryTags {
    viewport: String,
    app_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    country: Option<String>,
}

#[derive(Serialize)]
struct SentryUser {
    id: String,
}

fn make_event_id() -> String {
    // Sentry requires a 32-char hex UUID without dashes
    uuid::Uuid::new_v4().simple().to_string()
}

fn parse_stack_frames(stack: &str) -> Vec<SentryFrame> {
    // Each line like "    at functionName (file.js:line:col)"
    stack
        .lines()
        .filter(|l| l.trim_start().starts_with("at "))
        .map(|line| SentryFrame {
            filename: None,
            function: None,
            raw_function: line.trim().to_string(),
        })
        .collect()
}

#[async_trait]
impl Adapter for SentryAdapter {
    fn name(&self) -> &'static str {
        "sentry"
    }

    fn accepts(&self, event: &BatchedEvent) -> bool {
        matches!(event.event_type, EventType::Error)
    }

    async fn send(&self, event: &BatchedEvent) -> Result<(), AdapterError> {
        let err = match &event.error {
            Some(e) => e,
            None => return Ok(()),
        };

        let stacktrace = err.stack.as_deref().map(|stack| SentryStacktrace {
            frames: parse_stack_frames(stack),
        });

        let sentry_event = SentryEvent {
            event_id: make_event_id(),
            // Sentry expects Unix seconds as float
            timestamp: event.context.timestamp as f64 / 1000.0,
            platform: "javascript",
            level: "error",
            exception: SentryExceptions {
                values: vec![SentryException {
                    r#type: err.name.clone(),
                    value: err.message.clone(),
                    stacktrace,
                }],
            },
            tags: SentryTags {
                viewport: event.context.viewport.clone(),
                app_version: event.context.app_version.clone(),
                country: event.context.country.clone(),
            },
            release: self.release.clone().or_else(|| {
                Some(event.context.app_version.clone())
            }),
            environment: self.environment.clone(),
            user: event.context.session_id.as_str().strip_prefix("anon_").map(|_| {
                SentryUser { id: event.context.session_id.clone() }
            }),
        };

        // Sentry Envelope format:
        // <envelope header>\n<item header>\n<item body>
        let envelope_header = serde_json::json!({
            "event_id": &sentry_event.event_id,
            "dsn": format!(
                "https://{}@{}/{}",
                self.dsn.public_key, self.dsn.host, self.dsn.project_id
            )
        });
        let item_header = serde_json::json!({ "type": "event" });
        let item_body = serde_json::to_string(&sentry_event).map_err(AdapterError::Serialization)?;

        let envelope = format!(
            "{}\n{}\n{}",
            serde_json::to_string(&envelope_header).map_err(AdapterError::Serialization)?,
            serde_json::to_string(&item_header).map_err(AdapterError::Serialization)?,
            item_body,
        );

        let resp = self
            .client
            .post(self.dsn.envelope_url())
            .header("Content-Type", "application/x-sentry-envelope")
            .header("X-Sentry-Auth", self.dsn.auth_header())
            .body(envelope)
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
    fn dsn_parse_valid() {
        let dsn = SentryDsn::parse("https://abc123@o123.ingest.sentry.io/456789").unwrap();
        assert_eq!(dsn.public_key, "abc123");
        assert_eq!(dsn.host, "o123.ingest.sentry.io");
        assert_eq!(dsn.project_id, "456789");
    }

    #[test]
    fn dsn_envelope_url() {
        let dsn = SentryDsn::parse("https://abc123@o123.ingest.sentry.io/456789").unwrap();
        assert_eq!(dsn.envelope_url(), "https://o123.ingest.sentry.io/api/456789/envelope/");
    }

    #[test]
    fn dsn_parse_invalid() {
        assert!(SentryDsn::parse("not-a-url").is_err());
        assert!(SentryDsn::parse("https://@sentry.io/123").is_err());
    }

    #[test]
    fn only_accepts_error_events() {
        let adapter = SentryAdapter::new("https://key@sentry.io/1").unwrap();
        use nightshift_core::event::{BatchedEvent, EventContext, EventType};

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

        assert!(!adapter.accepts(&make(EventType::Track)));
        assert!(!adapter.accepts(&make(EventType::Identify)));
        assert!(adapter.accepts(&make(EventType::Error)));
    }
}
