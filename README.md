# Nightshift

> **Early Development Notice**
> This project is a work in progress and has not been tested in production. APIs, configuration, and adapter behaviour may change without notice. Use for exploration and feedback — not for production workloads yet.

**The zero-overhead telemetry gateway for modern web applications.**

> Your frontend works the day shift. Nightshift handles the rest.

---

## The Problem

Every analytics SDK you install is a tax on your users.

| SDK | Size | Blocks Main Thread? |
|---|---|---|
| Google Tag Manager | ~100kb | Yes |
| Mixpanel | ~70kb | Yes |
| Sentry Browser | ~50kb | Yes |
| FullStory | ~30kb | Yes |
| **@nightshift/client** | **< 2kb** | **Never** |

These scripts download, parse, and execute on the same thread React uses to render your UI. They directly inflate your **LCP**, **TBT**, and **INP** — the Core Web Vitals that determine your Google search ranking.

And then they leak your users' data to a dozen third-party domains.

---

## How Nightshift Works

```
Browser                    Edge                    Vendors
───────                    ────                    ───────
@nightshift/client         nightshift-edge         GA4
  <2kb gzipped             (Rust / WASM)           Mixpanel
  sendBeacon() ──────────► /ingest ──────────────► PostHog
  zero blocking            ↓                       Sentry
                           PII sanitize            Webhook
                           Dedup
                           Enrich (IP, UA, geo)
                           Fan-out (parallel)
```

**The client** is a typed TypeScript SDK that batches events and fires them via `navigator.sendBeacon()`. No main-thread work. No external script tags. No vendor SDK loaded in the browser.

**The edge worker** runs on a first-party subdomain (e.g. `telemetry.yourdomain.com`). It holds all your API keys, strips PII before fan-out, and translates your generic events into vendor-specific API calls — all in Rust, at the network edge.

---

## Features

- **< 2kb gzipped** TypeScript SDK — strict generic types, zero runtime dependencies
- **`navigator.sendBeacon()`** transport — zero main-thread blocking
- **Smart batching** — flushes on 5s timer, 20-event queue, or tab close
- **PII sanitization** — emails and API tokens stripped before fan-out
- **Dedup** — sliding window prevents double-fires from beacon retries
- **Ad-blocker bypass** — runs on your own subdomain, not a third-party domain
- **GDPR/CCPA ready** — IP addresses never reach vendors
- **Offline resilience** — IndexedDB queue for mobile users losing connectivity
- **Multi-platform** — deploys to Cloudflare Workers, Vercel Edge, or standalone Axum server

### Supported Adapters

| Adapter | Events | Status |
|---|---|---|
| Google Analytics 4 | track, error | ✅ |
| Sentry | error (Envelope API) | ✅ |
| Mixpanel | track, identify, error | ✅ |
| PostHog | track, identify, error | ✅ |
| Webhook | all (generic JSON POST) | ✅ |
| Amplitude | track, identify, error | ✅ |
| Segment | track, identify, error | ✅ |
| Facebook Conversions API | track, error | ✅ |
| TikTok Events API | track, error | ✅ |
| FullStory | identify, custom events¹ | Planned |

¹ FullStory's session replay requires its browser SDK to run in-page (DOM capture can't be proxied). The Nightshift FullStory adapter covers the server-side portion: forwarding `identify` calls to the [FullStory Identity API](https://developer.fullstory.com/server/v2/users/set-user-properties/) and custom events to the [Events API](https://developer.fullstory.com/server/v2/events/create-events/) for cross-device user stitching.

---

## Quick Start

### 1. Add the client SDK

```bash
npm install @nightshift/client
```

```tsx
// app/providers.tsx (Next.js App Router)
'use client';
import { useEffect } from 'react';
import { Nightshift } from '@nightshift/client';

export function NightshiftProvider({ children }: { children: React.ReactNode }) {
  useEffect(() => {
    Nightshift.init({
      endpoint: 'https://telemetry.yourdomain.com/ingest',
      appVersion: process.env.NEXT_PUBLIC_APP_VERSION,
    });
  }, []);
  return <>{children}</>;
}
```

```tsx
// Anywhere in your app
import { Nightshift } from '@nightshift/client';

Nightshift.track('Checkout_Started', { cartValue: 150, currency: 'USD' });
Nightshift.identify('user_123', { plan: 'premium' });
Nightshift.error(new Error('Payment gateway timeout'));
```

**Type-safe event schemas** — define your event map once:

```typescript
type MyEvents = {
  Checkout_Started: { cartValue: number; currency: string };
  Button_Clicked: { buttonName: string };
};

const client = Nightshift.init<MyEvents>({ endpoint: '...' });
client.track('Checkout_Started', { cartValue: 150, currency: 'USD' }); // ✅
client.track('Unknown_Event', {});                                      // ❌ type error
client.track('Checkout_Started', { cartValue: 'oops' });               // ❌ type error
```

### 2. Deploy the edge worker

**Option A: Cloudflare Workers** (recommended)

```bash
cd crates/nightshift-worker
npx wrangler secret put GA4_MEASUREMENT_ID
npx wrangler secret put GA4_API_SECRET
npx wrangler secret put SENTRY_DSN
npx wrangler secret put MIXPANEL_TOKEN
npx wrangler secret put POSTHOG_API_KEY
npx wrangler deploy
```

**Option B: Standalone Axum server** (Docker / VPS)

```bash
cargo build -p nightshift-server --release

GA4_MEASUREMENT_ID=G-XXXXXXXX \
GA4_API_SECRET=your_secret \
SENTRY_DSN=https://key@o123.ingest.sentry.io/456 \
MIXPANEL_TOKEN=your_token \
POSTHOG_API_KEY=phc_your_key \
./target/release/nightshift-server
```

**Option C: Local development**

```bash
cargo run -p nightshift-server
# Server starts on http://localhost:8080

# Point your client at it:
NEXT_PUBLIC_NIGHTSHIFT_ENDPOINT=http://localhost:8080/ingest
```

---

## Environment Variables

| Variable | Description | Required For |
|---|---|---|
| `GA4_MEASUREMENT_ID` | GA4 Measurement ID (G-XXXXXXXX) | GA4 adapter |
| `GA4_API_SECRET` | GA4 Measurement Protocol API Secret | GA4 adapter |
| `SENTRY_DSN` | Sentry DSN | Sentry adapter |
| `SENTRY_RELEASE` | Release version tag | Sentry (optional) |
| `SENTRY_ENVIRONMENT` | e.g. `production` | Sentry (optional) |
| `MIXPANEL_TOKEN` | Mixpanel Project Token | Mixpanel adapter |
| `POSTHOG_API_KEY` | PostHog API Key (phc_...) | PostHog adapter |
| `POSTHOG_ENDPOINT` | Self-hosted PostHog URL | PostHog (optional) |
| `WEBHOOK_URL` | Generic webhook endpoint | Webhook adapter |
| `WEBHOOK_SECRET` | Value for X-Webhook-Secret header | Webhook (optional) |
| `PORT` | HTTP port (default: 8080) | Server only |
| `DEDUP_TTL_SECS` | Dedup window in seconds (default: 30) | All targets |

---

## Architecture

### Repository Structure

```
nightshift/
├── packages/
│   ├── schema/     # @nightshift/schema — canonical JSON Schema + TypeScript types
│   └── client/     # @nightshift/client — <2kb TypeScript SDK
├── crates/
│   ├── nightshift-core/      # Rust types, PII sanitizer, dedup cache
│   ├── nightshift-adapters/  # GA4, Sentry, Mixpanel, PostHog, Webhook adapters
│   ├── nightshift-server/    # Standalone Axum HTTP server
│   ├── nightshift-worker/    # Cloudflare Workers target (worker-rs)
│   └── nightshift-vercel/    # Vercel Edge Functions target (WASM)
└── examples/
    └── nextjs-demo/          # Next.js 15 App Router demo
```

### Event Schema

All events share a single canonical schema defined in `packages/schema/src/event.schema.json`. TypeScript types are generated from this schema. Rust serde structs mirror it exactly, with a roundtrip fixture test enforcing parity.

```json
{
  "batch": [{
    "type": "track | identify | error",
    "event": "Checkout_Started",
    "properties": { "cartValue": 150 },
    "context": {
      "viewport": "390x844",
      "url": "/checkout",
      "sessionId": "anon_abc123",
      "appVersion": "v1.2.0",
      "timestamp": 1700000000000
    }
  }]
}
```

### Security Model

- **Secrets never touch the browser.** All vendor API keys live in edge environment variables.
- **PII is stripped before fan-out.** Emails and token patterns are redacted by regex in `nightshift-core/pii.rs` before any adapter receives the event.
- **IP addresses are stripped.** Client IPs are enriched server-side for geo-lookup and then removed before vendor fan-out.
- **First-party domain.** Deploy on `telemetry.yourdomain.com` — ad-blockers only block known third-party analytics domains.

---

## Development

```bash
# Install dependencies
pnpm install

# Run all Rust tests
cargo test -p nightshift-core -p nightshift-adapters -p nightshift-server

# Run TypeScript tests
pnpm --filter @nightshift/client test

# Build everything
pnpm build

# Start local demo
cargo run -p nightshift-server &
pnpm --filter nextjs-demo dev
# Open http://localhost:3000
```

---

## Contributing

Adapters are the most impactful contribution. See `crates/nightshift-adapters/src/adapter.rs` for the `Adapter` trait — implementing a new vendor is ~80 lines of Rust.

Planned: Plausible, FullStory (server-side identify + custom events only — session replay requires in-browser DOM capture and cannot be proxied).

---

## License

MIT
