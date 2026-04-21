import type { EventContext } from './types.js';
import { getSessionId } from './session.js';

const UTM_KEYS = ['utm_source', 'utm_medium', 'utm_campaign', 'utm_term', 'utm_content'] as const;
const UTM_STORAGE_KEY = '__ns_utms';

type UtmMap = Partial<Pick<EventContext, 'utmSource' | 'utmMedium' | 'utmCampaign' | 'utmTerm' | 'utmContent'>>;

function parseUtms(search: string): UtmMap {
  const p = new URLSearchParams(search);
  const result: UtmMap = {};
  const fieldMap: Record<string, keyof UtmMap> = {
    utm_source: 'utmSource',
    utm_medium: 'utmMedium',
    utm_campaign: 'utmCampaign',
    utm_term: 'utmTerm',
    utm_content: 'utmContent',
  };
  for (const key of UTM_KEYS) {
    const val = p.get(key);
    if (val) result[fieldMap[key]] = val;
  }
  return result;
}

function getStoredUtms(): UtmMap {
  try {
    const raw = sessionStorage.getItem(UTM_STORAGE_KEY);
    return raw ? (JSON.parse(raw) as UtmMap) : {};
  } catch {
    return {};
  }
}

function resolveUtms(): UtmMap {
  if (typeof window === 'undefined') return {};
  const fromUrl = parseUtms(window.location.search);
  if (Object.keys(fromUrl).length > 0) {
    try {
      sessionStorage.setItem(UTM_STORAGE_KEY, JSON.stringify(fromUrl));
    } catch {
      // sessionStorage blocked — still return the current-page UTMs
    }
    return fromUrl;
  }
  return getStoredUtms();
}

export function buildContext(appVersion: string): EventContext {
  const utms = resolveUtms();
  const ctx: EventContext = {
    viewport: getViewport(),
    url: getCurrentUrl(),
    sessionId: getSessionId(),
    appVersion,
    timestamp: Date.now(),
    ...utms,
  };

  if (typeof document !== 'undefined') {
    if (document.referrer) ctx.referrer = document.referrer;
    if (document.title) ctx.pageTitle = document.title;
  }

  return ctx;
}

function getViewport(): string {
  if (typeof window === 'undefined') return '0x0';
  return `${window.innerWidth}x${window.innerHeight}`;
}

function getCurrentUrl(): string {
  if (typeof window === 'undefined') return '/';
  return window.location.href;
}
