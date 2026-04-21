use once_cell::sync::Lazy;
use regex::Regex;

use crate::event::BatchedEvent;

static EMAIL_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"[a-zA-Z0-9._%+\-]+@[a-zA-Z0-9.\-]+\.[a-zA-Z]{2,}").unwrap()
});

static TOKEN_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(bearer\s+|token[=:\s]+|api[_\-]?key[=:\s]+)[a-zA-Z0-9\-_.]{20,}").unwrap()
});

/// Sanitizes a single event in-place:
/// - Strips the client IP (never forward to vendors)
/// - Redacts email addresses and API tokens in properties, traits, and error strings
pub fn sanitize_event(mut event: BatchedEvent) -> BatchedEvent {
    event.context.ip = None;

    if let Some(props) = &mut event.properties {
        sanitize_value(props);
    }
    if let Some(traits) = &mut event.traits {
        sanitize_value(traits);
    }
    if let Some(err) = &mut event.error {
        err.message = redact_string(&err.message);
        if let Some(stack) = &err.stack {
            err.stack = Some(redact_string(stack));
        }
    }
    event
}

fn sanitize_value(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(s) => *s = redact_string(s),
        serde_json::Value::Object(map) => {
            for v in map.values_mut() {
                sanitize_value(v);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr.iter_mut() {
                sanitize_value(v);
            }
        }
        _ => {}
    }
}

fn redact_string(s: &str) -> String {
    let s = EMAIL_RE.replace_all(s, "[REDACTED_EMAIL]");
    TOKEN_RE.replace_all(&s, "[REDACTED_TOKEN]").into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{BatchedEvent, EventContext, EventType};

    fn make_event(props: Option<serde_json::Value>) -> BatchedEvent {
        BatchedEvent {
            event_type: EventType::Track,
            event: Some("test".to_string()),
            user_id: None,
            properties: props,
            traits: None,
            error: None,
            context: EventContext {
                viewport: "390x844".to_string(),
                url: "/".to_string(),
                session_id: "anon_test".to_string(),
                app_version: "v1".to_string(),
                timestamp: 0,
                ip: Some("1.2.3.4".to_string()),
                ..Default::default()
            },
        }
    }

    #[test]
    fn strips_ip() {
        let event = make_event(None);
        let out = sanitize_event(event);
        assert!(out.context.ip.is_none());
    }

    #[test]
    fn redacts_email_in_properties() {
        let props = serde_json::json!({ "email": "user@example.com", "name": "Alice" });
        let event = make_event(Some(props));
        let out = sanitize_event(event);
        let props = out.properties.unwrap();
        assert_eq!(props["email"], "[REDACTED_EMAIL]");
        assert_eq!(props["name"], "Alice");
    }

    #[test]
    fn redacts_nested_email() {
        let props = serde_json::json!({ "user": { "contact": "test@foo.io" } });
        let event = make_event(Some(props));
        let out = sanitize_event(event);
        assert_eq!(out.properties.unwrap()["user"]["contact"], "[REDACTED_EMAIL]");
    }

    #[test]
    fn redacts_email_in_array() {
        let props = serde_json::json!({ "emails": ["a@b.com", "plain"] });
        let event = make_event(Some(props));
        let out = sanitize_event(event);
        let arr = out.properties.unwrap();
        assert_eq!(arr["emails"][0], "[REDACTED_EMAIL]");
        assert_eq!(arr["emails"][1], "plain");
    }

    #[test]
    fn redacts_error_message() {
        let mut event = make_event(None);
        event.error = Some(crate::event::SerializedError {
            message: "sent to user@example.com failed".to_string(),
            name: "Error".to_string(),
            stack: Some("at fn (user@test.org:1)".to_string()),
        });
        let out = sanitize_event(event);
        let err = out.error.unwrap();
        assert!(err.message.contains("[REDACTED_EMAIL]"));
        assert!(err.stack.unwrap().contains("[REDACTED_EMAIL]"));
    }
}
