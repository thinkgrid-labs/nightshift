use axum::body::Body;
use axum::http::{Request, StatusCode};
use httpmock::prelude::*;
use nightshift_server::build_app;
use nightshift_server::config::EdgeConfig;
use serde_json::json;
use tower::ServiceExt;

fn config_with_webhook(url: &str) -> EdgeConfig {
    EdgeConfig {
        port: 0,
        ga4_measurement_id: None,
        ga4_api_secret: None,
        webhook_url: Some(url.to_string()),
        webhook_secret: None,
        sentry_dsn: None,
        sentry_release: None,
        sentry_environment: None,
        mixpanel_token: None,
        posthog_api_key: None,
        posthog_endpoint: None,
        amplitude_api_key: None,
        segment_write_key: None,
        facebook_pixel_id: None,
        facebook_access_token: None,
        tiktok_pixel_code: None,
        tiktok_access_token: None,
        dedup_ttl_secs: 30,
        debug: false,
    }
}

fn batch_body(events: serde_json::Value) -> Body {
    Body::from(serde_json::to_vec(&json!({ "batch": events })).unwrap())
}

#[tokio::test]
async fn ingest_returns_204() {
    let mock_server = MockServer::start();
    let _mock = mock_server.mock(|when, then| {
        when.method(POST).path("/");
        then.status(200);
    });

    let app = build_app(config_with_webhook(&mock_server.url("/")));
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/ingest")
                .header("content-type", "application/json")
                .body(batch_body(json!([{
                    "type": "track",
                    "event": "Test",
                    "context": {
                        "viewport": "390x844",
                        "url": "/",
                        "sessionId": "anon_test1",
                        "appVersion": "v1",
                        "timestamp": 1000000
                    }
                }])))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn empty_batch_returns_400() {
    let app = build_app(config_with_webhook("http://unused"));
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/ingest")
                .header("content-type", "application/json")
                .body(batch_body(json!([])))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn pii_stripped_before_webhook() {
    // PII redaction is unit-tested in nightshift-core/src/pii.rs.
    // Here we verify the sanitized event is still forwarded to the webhook.
    let mock_server = MockServer::start();
    let mock = mock_server.mock(|when, then| {
        when.method(POST).path("/");
        then.status(200);
    });

    let app = build_app(config_with_webhook(&mock_server.url("/")));

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/ingest")
                .header("content-type", "application/json")
                .body(batch_body(json!([{
                    "type": "track",
                    "event": "Signup",
                    "properties": { "email": "user@example.com", "plan": "pro" },
                    "context": {
                        "viewport": "1440x900",
                        "url": "/signup",
                        "sessionId": "anon_pii1",
                        "appVersion": "v1",
                        "timestamp": 2000000
                    }
                }])))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Give the spawned task time to hit the webhook
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    mock.assert_hits_async(1).await;
}

#[tokio::test]
async fn health_returns_200() {
    let app = build_app(config_with_webhook("http://unused"));
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn dedup_drops_duplicate_events() {
    let mock_server = MockServer::start();
    let mock = mock_server.mock(|when, then| {
        when.method(POST).path("/");
        then.status(200);
    });

    let app = build_app(config_with_webhook(&mock_server.url("/")));

    // Same sessionId + event + timestamp rounded to the same second → duplicate
    let body = json!([{
        "type": "track",
        "event": "ButtonClick",
        "context": {
            "viewport": "390x844",
            "url": "/",
            "sessionId": "anon_dup1",
            "appVersion": "v1",
            "timestamp": 3000000
        }
    }]);

    for _ in 0..2 {
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/ingest")
                    .header("content-type", "application/json")
                    .body(batch_body(body.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
    }

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    // Webhook called exactly once — duplicate was dropped
    mock.assert_hits_async(1).await;
}
