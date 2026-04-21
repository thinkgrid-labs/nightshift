use std::env;

#[derive(Debug, Clone)]
pub struct EdgeConfig {
    pub port: u16,
    // GA4
    pub ga4_measurement_id: Option<String>,
    pub ga4_api_secret: Option<String>,
    // Webhook
    pub webhook_url: Option<String>,
    pub webhook_secret: Option<String>,
    // Sentry
    pub sentry_dsn: Option<String>,
    pub sentry_release: Option<String>,
    pub sentry_environment: Option<String>,
    // Mixpanel
    pub mixpanel_token: Option<String>,
    // PostHog
    pub posthog_api_key: Option<String>,
    pub posthog_endpoint: Option<String>,
    // Amplitude
    pub amplitude_api_key: Option<String>,
    // Segment
    pub segment_write_key: Option<String>,
    // Facebook Conversions API
    pub facebook_pixel_id: Option<String>,
    pub facebook_access_token: Option<String>,
    // TikTok Events API
    pub tiktok_pixel_code: Option<String>,
    pub tiktok_access_token: Option<String>,
    /// Dedup TTL in seconds (default: 30)
    pub dedup_ttl_secs: u64,
    pub debug: bool,
}

impl EdgeConfig {
    pub fn from_env() -> Self {
        Self {
            port: env_u16("PORT", 8080),
            ga4_measurement_id: env::var("GA4_MEASUREMENT_ID").ok(),
            ga4_api_secret: env::var("GA4_API_SECRET").ok(),
            webhook_url: env::var("WEBHOOK_URL").ok(),
            webhook_secret: env::var("WEBHOOK_SECRET").ok(),
            sentry_dsn: env::var("SENTRY_DSN").ok(),
            sentry_release: env::var("SENTRY_RELEASE").ok(),
            sentry_environment: env::var("SENTRY_ENVIRONMENT").ok(),
            mixpanel_token: env::var("MIXPANEL_TOKEN").ok(),
            posthog_api_key: env::var("POSTHOG_API_KEY").ok(),
            posthog_endpoint: env::var("POSTHOG_ENDPOINT").ok(),
            amplitude_api_key: env::var("AMPLITUDE_API_KEY").ok(),
            segment_write_key: env::var("SEGMENT_WRITE_KEY").ok(),
            facebook_pixel_id: env::var("FACEBOOK_PIXEL_ID").ok(),
            facebook_access_token: env::var("FACEBOOK_ACCESS_TOKEN").ok(),
            tiktok_pixel_code: env::var("TIKTOK_PIXEL_CODE").ok(),
            tiktok_access_token: env::var("TIKTOK_ACCESS_TOKEN").ok(),
            dedup_ttl_secs: env_u64("DEDUP_TTL_SECS", 30),
            debug: env::var("DEBUG").is_ok(),
        }
    }
}

fn env_u16(key: &str, default: u16) -> u16 {
    env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn env_u64(key: &str, default: u64) -> u64 {
    env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}
