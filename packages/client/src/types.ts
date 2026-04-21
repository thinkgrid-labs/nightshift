export type EventType = 'track' | 'identify' | 'error';

export interface EventContext {
  viewport: string;
  url: string;
  sessionId: string;
  appVersion: string;
  timestamp: number;
  referrer?: string;
  pageTitle?: string;
  utmSource?: string;
  utmMedium?: string;
  utmCampaign?: string;
  utmTerm?: string;
  utmContent?: string;
}

export interface SerializedError {
  message: string;
  name: string;
  stack?: string;
}

export interface BatchedEvent<TProperties = Record<string, unknown>> {
  type: EventType;
  event?: string;
  userId?: string;
  properties?: TProperties;
  traits?: Record<string, unknown>;
  error?: SerializedError;
  context: EventContext;
}

export interface IngestBatch {
  batch: BatchedEvent[];
}

export interface NightshiftConfig<
  TEvents extends Record<string, Record<string, unknown>> = Record<string, Record<string, unknown>>,
> {
  endpoint: string;
  appVersion?: string;
  /** Flush interval in ms. Default: 5000 */
  flushInterval?: number;
  /** Max queue size before forced flush. Default: 20 */
  maxBatchSize?: number;
  /** 0.0–1.0 sample rate. Default: 1.0 */
  sampleRate?: number;
  debug?: boolean;
  /** Auto-fire a Page_Viewed event on init and on SPA navigation. Default: false */
  autoPageview?: boolean;
}

export interface BatchQueueOptions {
  flushInterval: number;
  maxSize: number;
  onFlush: (batch: BatchedEvent[]) => Promise<void>;
}
