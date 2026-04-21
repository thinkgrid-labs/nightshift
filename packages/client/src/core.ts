import { BatchQueue } from './batch.js';
import { buildContext } from './context.js';
import { OfflineQueue } from './storage.js';
import { createTransport } from './transport.js';
import type { BatchedEvent, IngestBatch, NightshiftConfig, SerializedError } from './types.js';

export class NightshiftClient<
  TEvents extends Record<string, Record<string, unknown>> = Record<string, Record<string, unknown>>,
> {
  private readonly config: Required<NightshiftConfig<TEvents>>;
  private readonly queue: BatchQueue;
  private readonly transport = createTransport();
  private readonly offline = new OfflineQueue();
  private initialized = false;

  private _pageviewCleanup: (() => void) | null = null;

  constructor(config: NightshiftConfig<TEvents>) {
    this.config = {
      appVersion: 'unknown',
      flushInterval: 5000,
      maxBatchSize: 20,
      sampleRate: 1.0,
      debug: false,
      autoPageview: false,
      ...config,
    };

    this.queue = new BatchQueue({
      flushInterval: this.config.flushInterval,
      maxSize: this.config.maxBatchSize,
      onFlush: (batch) => this.sendBatch(batch),
    });

    void this.offline.init().then(async () => {
      const pending = await this.offline.drain();
      if (pending.length > 0) {
        await this.sendBatch(pending);
      }
    });

    this.initialized = true;

    if (this.config.autoPageview) {
      this._pageviewCleanup = setupAutoPageview(() => this.track('Page_Viewed' as keyof TEvents & string));
    }
  }

  track<K extends keyof TEvents>(event: K, properties?: TEvents[K]): void {
    if (!this.shouldSample()) return;
    const ev: BatchedEvent = {
      type: 'track',
      event: String(event),
      context: buildContext(this.config.appVersion),
    };
    if (properties !== undefined) ev.properties = properties as Record<string, unknown>;
    this.push(ev);
  }

  identify(userId: string, traits?: Record<string, unknown>): void {
    const ev: BatchedEvent = {
      type: 'identify',
      userId,
      context: buildContext(this.config.appVersion),
    };
    if (traits !== undefined) ev.traits = traits;
    this.push(ev);
  }

  error(err: Error, extra?: Record<string, unknown>): void {
    const serialized: SerializedError = { message: err.message, name: err.name };
    if (err.stack !== undefined) serialized.stack = err.stack;
    const ev: BatchedEvent = {
      type: 'error',
      error: serialized,
      context: buildContext(this.config.appVersion),
    };
    if (extra !== undefined) ev.properties = extra;
    this.push(ev);
  }

  async flush(): Promise<void> {
    return this.queue.flush();
  }

  destroy(): void {
    this.queue.destroy();
    this._pageviewCleanup?.();
    this._pageviewCleanup = null;
    this.initialized = false;
  }

  private push(event: BatchedEvent): void {
    if (!this.initialized) return;
    this.queue.push(event);
  }

  private async sendBatch(batch: BatchedEvent[]): Promise<void> {
    const payload: IngestBatch = { batch };
    try {
      await this.transport.send(this.config.endpoint, payload);
    } catch {
      void this.offline.push(batch);
    }
  }

  private shouldSample(): boolean {
    return this.config.sampleRate >= 1.0 || Math.random() < this.config.sampleRate;
  }
}

function setupAutoPageview(fire: () => void): () => void {
  if (typeof window === 'undefined') return () => {};

  fire();

  const onPopState = () => fire();
  window.addEventListener('popstate', onPopState);

  // Wrap history.pushState / replaceState to catch SPA navigation
  const originalPush = history.pushState.bind(history);
  const originalReplace = history.replaceState.bind(history);

  history.pushState = (...args) => {
    originalPush(...args);
    fire();
  };
  history.replaceState = (...args) => {
    originalReplace(...args);
    fire();
  };

  return () => {
    window.removeEventListener('popstate', onPopState);
    history.pushState = originalPush;
    history.replaceState = originalReplace;
  };
}

// Singleton facade
let _instance: NightshiftClient<Record<string, Record<string, unknown>>> | null = null;

function assertInstance(): NightshiftClient<Record<string, Record<string, unknown>>> {
  if (!_instance) throw new Error('[Nightshift] Call Nightshift.init() before using the SDK.');
  return _instance;
}

export const Nightshift = {
  init<TEvents extends Record<string, Record<string, unknown>>>(
    config: NightshiftConfig<TEvents>
  ): NightshiftClient<TEvents> {
    if (_instance) _instance.destroy();
    const client = new NightshiftClient<TEvents>(config);
    _instance = client as unknown as NightshiftClient<Record<string, Record<string, unknown>>>;
    return client;
  },

  track(event: string, properties?: Record<string, unknown>): void {
    assertInstance().track(event, properties);
  },

  identify(userId: string, traits?: Record<string, unknown>): void {
    assertInstance().identify(userId, traits);
  },

  error(err: Error, extra?: Record<string, unknown>): void {
    assertInstance().error(err, extra);
  },

  async flush(): Promise<void> {
    return assertInstance().flush();
  },

  destroy(): void {
    if (_instance) {
      _instance.destroy();
      _instance = null;
    }
  },
};
