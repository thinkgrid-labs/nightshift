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
use nightshift_core::{
    dedup::{DedupCache, InMemoryDedupCache, dedup_key},
    event::IngestBatch,
    pii::sanitize_event,
};
use std::sync::Mutex;
use worker::*;

/// Returns true if the key is a duplicate. Uses KV for cross-request persistence when available;
/// falls back to the in-memory cache for same-batch duplicates only.
async fn kv_or_mem_dedup(
    kv: &Option<worker::kv::KvStore>,
    mem: &mut InMemoryDedupCache,
    key: &str,
) -> bool {
    if let Some(store) = kv {
        let ns_key = format!("dedup:{key}");
        match store.get(&ns_key).text().await {
            Ok(Some(_)) => return true,
            _ => {
                if let Ok(builder) = store.put(&ns_key, "1") {
                    let _ = builder.expiration_ttl(30).execute().await;
                }
                return false;
            }
        }
    }
    mem.is_duplicate(key)
}

/// Builds the AdapterRouter from Cloudflare Worker environment bindings.
/// Each adapter is enabled only if its required secrets are present.
fn build_router(env: &Env) -> AdapterRouter {
    let mut adapters: Vec<Box<dyn nightshift_adapters::adapter::Adapter>> = Vec::new();

    if let (Ok(mid), Ok(secret)) = (env.var("GA4_MEASUREMENT_ID"), env.var("GA4_API_SECRET")) {
        adapters.push(Box::new(Ga4Adapter::new(mid.to_string(), secret.to_string())));
    }

    if let Ok(url) = env.var("WEBHOOK_URL") {
        let mut adapter = WebhookAdapter::new(url.to_string());
        if let Ok(s) = env.var("WEBHOOK_SECRET") {
            adapter = adapter.with_secret("X-Webhook-Secret", s.to_string());
        }
        adapters.push(Box::new(adapter));
    }

    if let Ok(dsn) = env.var("SENTRY_DSN") {
        if let Ok(mut adapter) = SentryAdapter::new(&dsn.to_string()) {
            if let Ok(r) = env.var("SENTRY_RELEASE") {
                adapter = adapter.with_release(r.to_string());
            }
            if let Ok(e) = env.var("SENTRY_ENVIRONMENT") {
                adapter = adapter.with_environment(e.to_string());
            }
            adapters.push(Box::new(adapter));
        }
    }

    if let Ok(token) = env.var("MIXPANEL_TOKEN") {
        adapters.push(Box::new(MixpanelAdapter::new(token.to_string())));
    }

    if let Ok(key) = env.var("POSTHOG_API_KEY") {
        let mut adapter = PostHogAdapter::new(key.to_string());
        if let Ok(endpoint) = env.var("POSTHOG_ENDPOINT") {
            adapter = adapter.with_endpoint(endpoint.to_string());
        }
        adapters.push(Box::new(adapter));
    }

    if let Ok(key) = env.var("AMPLITUDE_API_KEY") {
        adapters.push(Box::new(AmplitudeAdapter::new(key.to_string())));
    }

    if let Ok(key) = env.var("SEGMENT_WRITE_KEY") {
        adapters.push(Box::new(SegmentAdapter::new(key.to_string())));
    }

    if let (Ok(pixel_id), Ok(token)) = (env.var("FACEBOOK_PIXEL_ID"), env.var("FACEBOOK_ACCESS_TOKEN")) {
        adapters.push(Box::new(FacebookAdapter::new(pixel_id.to_string(), token.to_string())));
    }

    if let (Ok(pixel_code), Ok(token)) = (env.var("TIKTOK_PIXEL_CODE"), env.var("TIKTOK_ACCESS_TOKEN")) {
        adapters.push(Box::new(TikTokAdapter::new(pixel_code.to_string(), token.to_string())));
    }

    AdapterRouter::new(adapters)
}

#[event(fetch)]
async fn main(mut req: Request, env: Env, _ctx: Context) -> Result<Response> {
    // CORS preflight
    if req.method() == Method::Options {
        return Response::empty()
            .map(|r| r.with_headers(cors_headers()));
    }

    if req.path() == "/health" && req.method() == Method::Get {
        return Response::ok("ok");
    }

    if req.path() != "/ingest" || req.method() != Method::Post {
        return Response::error("Not Found", 404);
    }

    let body: IngestBatch = match req.json().await {
        Ok(b) => b,
        Err(_) => return Response::error("Bad Request", 400),
    };

    if body.batch.is_empty() {
        return Response::error("Bad Request", 400);
    }

    // Extract client IP from Cloudflare-injected header
    let ip = req
        .headers()
        .get("CF-Connecting-IP")
        .ok()
        .flatten();
    let user_agent = req
        .headers()
        .get("User-Agent")
        .ok()
        .flatten();
    let country = req
        .headers()
        .get("CF-IPCountry")
        .ok()
        .flatten();

    // Dedup — prefer KV for cross-request dedup; fall back to in-memory for same-batch duplicates.
    // Bind a KV namespace named "DEDUP" in wrangler.toml to enable persistent dedup.
    let kv = env.kv("DEDUP").ok();
    let mut mem_dedup = InMemoryDedupCache::new(5);

    let mut events: Vec<_> = Vec::new();
    for mut event in body.batch.into_iter() {
        event.context.ip = ip.clone();
        event.context.user_agent = user_agent.clone();
        event.context.country = country.clone();

        let key = dedup_key(&event);
        let is_dup = kv_or_mem_dedup(&kv, &mut mem_dedup, &key).await;
        if !is_dup {
            events.push(sanitize_event(event));
        }
    }

    if !events.is_empty() {
        let router = build_router(&env);
        router.route(events).await;
    }

    Response::empty()
        .map(|r| r.with_status(204).with_headers(cors_headers()))
}

fn cors_headers() -> Headers {
    let mut headers = Headers::new();
    let _ = headers.set("Access-Control-Allow-Origin", "*");
    let _ = headers.set("Access-Control-Allow-Methods", "POST, OPTIONS");
    let _ = headers.set("Access-Control-Allow-Headers", "Content-Type");
    headers
}
