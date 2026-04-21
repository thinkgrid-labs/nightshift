pub mod config;
pub mod routes;

use std::sync::Arc;

use axum::{Router, routing::get, routing::post};
use nightshift_adapters::{
    adapter::AdapterRouter,
    amplitude::AmplitudeAdapter,
    facebook::FacebookAdapter,
    ga4::Ga4Adapter,
    mixpanel::MixpanelAdapter,
    posthog::PostHogAdapter,
    segment::SegmentAdapter,
    sentry::SentryAdapter,
    tiktok::TikTokAdapter,
    webhook::WebhookAdapter,
};
use nightshift_core::dedup::InMemoryDedupCache;
use tokio::sync::Mutex;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::{info, warn};

use crate::config::EdgeConfig;
use crate::routes::{AppState, health_handler, ingest_handler};

pub fn build_app(config: EdgeConfig) -> Router {
    let mut adapters: Vec<Box<dyn nightshift_adapters::adapter::Adapter>> = Vec::new();

    if let (Some(mid), Some(secret)) = (&config.ga4_measurement_id, &config.ga4_api_secret) {
        let mut adapter = Ga4Adapter::new(mid, secret);
        if config.debug {
            adapter = adapter.with_debug();
        }
        adapters.push(Box::new(adapter));
        info!("GA4 adapter enabled");
    }

    if let Some(url) = &config.webhook_url {
        let mut adapter = WebhookAdapter::new(url);
        if let Some(secret) = &config.webhook_secret {
            adapter = adapter.with_secret("X-Webhook-Secret", secret);
        }
        adapters.push(Box::new(adapter));
        info!("Webhook adapter enabled");
    }

    if let Some(dsn) = &config.sentry_dsn {
        match SentryAdapter::new(dsn) {
            Ok(mut adapter) => {
                if let Some(r) = &config.sentry_release {
                    adapter = adapter.with_release(r);
                }
                if let Some(e) = &config.sentry_environment {
                    adapter = adapter.with_environment(e);
                }
                adapters.push(Box::new(adapter));
                info!("Sentry adapter enabled");
            }
            Err(e) => warn!(error = %e, "Sentry adapter disabled — invalid DSN"),
        }
    }

    if let Some(token) = &config.mixpanel_token {
        adapters.push(Box::new(MixpanelAdapter::new(token)));
        info!("Mixpanel adapter enabled");
    }

    if let Some(key) = &config.posthog_api_key {
        let mut adapter = PostHogAdapter::new(key);
        if let Some(endpoint) = &config.posthog_endpoint {
            adapter = adapter.with_endpoint(endpoint);
        }
        adapters.push(Box::new(adapter));
        info!("PostHog adapter enabled");
    }

    if let Some(key) = &config.amplitude_api_key {
        adapters.push(Box::new(AmplitudeAdapter::new(key)));
        info!("Amplitude adapter enabled");
    }

    if let Some(key) = &config.segment_write_key {
        adapters.push(Box::new(SegmentAdapter::new(key)));
        info!("Segment adapter enabled");
    }

    if let (Some(pixel_id), Some(token)) = (&config.facebook_pixel_id, &config.facebook_access_token) {
        adapters.push(Box::new(FacebookAdapter::new(pixel_id, token)));
        info!("Facebook Conversions API adapter enabled");
    }

    if let (Some(pixel_code), Some(token)) = (&config.tiktok_pixel_code, &config.tiktok_access_token) {
        adapters.push(Box::new(TikTokAdapter::new(pixel_code, token)));
        info!("TikTok Events API adapter enabled");
    }

    let state = Arc::new(AppState {
        router: Arc::new(AdapterRouter::new(adapters)),
        dedup: Arc::new(Mutex::new(
            Box::new(InMemoryDedupCache::new(config.dedup_ttl_secs))
                as Box<dyn nightshift_core::dedup::DedupCache>,
        )),
    });

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .route("/ingest", post(ingest_handler))
        .route("/health", get(health_handler))
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state)
}
