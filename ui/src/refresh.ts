import { ApiError } from "./api/client";

/** The longest wait between attempts after failures. */
export const CEILING_MS = 300_000;
/** How long a busy gateway is left alone when it said nothing better. */
const BUSY_DEFAULT_MS = 5_000;

export interface Failure {
  message: string;
  /** The gateway said it is busy (503 `admin_busy`): not a fault of the credential or the connection. */
  busy: boolean;
}

export interface Hooks<T> {
  /** Reads everything the view shows. Its answer is dropped if the query changed meanwhile. */
  load(signal: AbortSignal): Promise<T>;
  onData(data: T, at: number): void;
  onFailure(failure: Failure): void;
  /** The gateway refused the credential (401 or 403): refreshing has stopped. */
  onRefused(): void;
  onRunning?(running: boolean): void;
}

export interface Env {
  now(): number;
  hidden(): boolean;
}

const real: Env = { now: () => Date.now(), hidden: () => document.hidden };

/**
 * The timer behind the query view (spec: the view refreshes itself without hammering the gateway). One timer for the
 * whole view, which obeys four rules:
 *  - visible only: nothing is read while the page is hidden; on becoming visible it reads once if the data is older than
 *    the period;
 *  - no overlap: the next wait starts when the previous request ends, so a slow gateway is never given a second request;
 *  - back off: after a failure the wait is the period, then double, up to five minutes, and back to the period after a
 *    success; a busy gateway is not asked again before its `Retry-After`;
 *  - stop on refusal: a 401 or 403 ends refreshing.
 * A change of query (`restart`) drops the running request, whose late answer can never replace a newer one, reads at once
 * and starts the period again. With the period off nothing is read until `restart`.
 */
export class Refresher<T> {
  private periodMs = 0;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private abort: AbortController | null = null;
  private sequence = 0;
  private running = false;
  private failures = 0;
  private notBefore = 0;
  private lastDataAt: number | null = null;
  private stopped = false;

  constructor(
    private readonly hooks: Hooks<T>,
    private readonly env: Env = real,
  ) {}

  /** Sets the period (0 is off) without reading; the next `restart` or wait uses it. */
  setPeriod(ms: number): void {
    this.periodMs = ms;
  }

  /** The query changed, or the person asked: read now and start the period again. */
  restart(): void {
    this.stopped = false;
    this.abort?.abort();
    this.clear();
    this.sequence += 1;
    this.running = false;
    this.failures = 0;
    this.notBefore = 0;
    if (this.env.hidden()) {
      this.lastDataAt = null; // read as soon as the page is visible again
      return;
    }
    void this.run();
  }

  /** Call when the page becomes visible or hidden. */
  visibilityChanged(): void {
    if (this.stopped) return;
    if (this.env.hidden()) {
      this.clear();
      return;
    }
    if (this.running || this.periodMs === 0) return;
    const age = this.lastDataAt === null ? Infinity : this.env.now() - this.lastDataAt;
    if (age >= this.periodMs) this.wait(Math.max(0, this.notBefore - this.env.now()));
    else this.wait(this.periodMs - age);
  }

  stop(): void {
    this.stopped = true;
    this.abort?.abort();
    this.clear();
    this.sequence += 1;
    this.running = false;
  }

  private clear(): void {
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = null;
  }

  private wait(ms: number): void {
    this.clear();
    if (this.stopped || this.periodMs === 0 || this.env.hidden()) return;
    this.timer = setTimeout(() => {
      this.timer = null;
      void this.run();
    }, ms);
  }

  private async run(): Promise<void> {
    if (this.running) return;
    this.running = true;
    const mine = ++this.sequence;
    const controller = new AbortController();
    this.abort = controller;
    this.hooks.onRunning?.(true);
    let next = this.periodMs;
    try {
      const data = await this.hooks.load(controller.signal);
      if (mine !== this.sequence) return;
      this.failures = 0;
      this.lastDataAt = this.env.now();
      this.hooks.onData(data, this.lastDataAt);
    } catch (failure) {
      if (mine !== this.sequence) return;
      if (failure instanceof ApiError && (failure.status === 401 || failure.status === 403)) {
        this.stopped = true;
        this.hooks.onRefused();
        return;
      }
      this.failures += 1;
      const busy = failure instanceof ApiError && failure.status === 503 && failure.code === "admin_busy";
      next = Math.min(this.periodMs * 2 ** (this.failures - 1), CEILING_MS);
      if (busy) {
        next = Math.max(next, failure.retryAfterMs ?? BUSY_DEFAULT_MS);
        this.notBefore = this.env.now() + next;
      }
      this.hooks.onFailure({
        busy,
        message: busy ? "The gateway is busy with another statistics read." : failure instanceof ApiError ? failure.message : "The statistics could not be read.",
      });
    } finally {
      if (mine === this.sequence) {
        this.running = false;
        this.hooks.onRunning?.(false);
        this.wait(next);
      }
    }
  }
}
