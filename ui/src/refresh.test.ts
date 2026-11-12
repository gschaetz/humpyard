import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "./api/client";
import { CEILING_MS, Refresher, type Failure } from "./refresh";

const PERIOD = 60_000;

/** A refresher over fake timers whose reads the test answers by hand. */
function rig(period = PERIOD) {
  let hidden = false;
  const reads: { signal: AbortSignal; ok: (value: string) => void; fail: (error: unknown) => void }[] = [];
  const log = { data: [] as string[], failures: [] as Failure[], refused: 0 };
  const refresher = new Refresher<string>(
    {
      load: (signal) =>
        new Promise<string>((ok, fail) => {
          reads.push({ signal, ok, fail });
        }),
      onData: (data) => log.data.push(data),
      onFailure: (failure) => log.failures.push(failure),
      onRefused: () => {
        log.refused += 1;
      },
    },
    { now: () => Date.now(), hidden: () => hidden },
  );
  refresher.setPeriod(period);
  const answer = async (value = "ok") => {
    reads.at(-1)?.ok(value);
    await vi.advanceTimersByTimeAsync(0);
  };
  const fail = async (error: unknown) => {
    reads.at(-1)?.fail(error);
    await vi.advanceTimersByTimeAsync(0);
  };
  const hide = (value: boolean) => {
    hidden = value;
    refresher.visibilityChanged();
  };
  return { refresher, reads, log, answer, fail, hide };
}

beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
});

describe("the refresh timer", () => {
  it("reads at once, then once every period", async () => {
    const r = rig();
    r.refresher.restart();
    expect(r.reads).toHaveLength(1);
    await r.answer("one");
    await vi.advanceTimersByTimeAsync(PERIOD - 1);
    expect(r.reads).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(r.reads).toHaveLength(2);
    await r.answer("two");
    expect(r.log.data).toEqual(["one", "two"]);
  });

  it("makes no request while the page is hidden, and one when it is visible again if the data is older than the period", async () => {
    const r = rig();
    r.refresher.restart();
    await r.answer();
    r.hide(true);
    await vi.advanceTimersByTimeAsync(10 * 60_000);
    expect(r.reads).toHaveLength(1);
    r.hide(false);
    expect(r.reads).toHaveLength(1); // the wait is zero, the read follows on the next tick
    await vi.advanceTimersByTimeAsync(0);
    expect(r.reads).toHaveLength(2);
  });

  it("does not read again on becoming visible when the data is younger than the period, but waits out the rest", async () => {
    const r = rig();
    r.refresher.restart();
    await r.answer();
    await vi.advanceTimersByTimeAsync(20_000);
    r.hide(true);
    r.hide(false);
    await vi.advanceTimersByTimeAsync(39_000);
    expect(r.reads).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(1_000);
    expect(r.reads).toHaveLength(2);
  });

  it("starts no second request while one is running, and schedules the next after it ends", async () => {
    const r = rig();
    r.refresher.restart();
    await vi.advanceTimersByTimeAsync(5 * PERIOD); // the gateway never answers
    expect(r.reads).toHaveLength(1);
    await r.answer();
    await vi.advanceTimersByTimeAsync(PERIOD - 1);
    expect(r.reads).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(r.reads).toHaveLength(2);
  });

  it("waits the period, then double, up to five minutes after failures, and the period again after a success", async () => {
    const r = rig(PERIOD);
    r.refresher.restart();
    const waits: number[] = [];
    for (let i = 0; i < 5; i++) {
      await r.fail(new ApiError(0, null, "down"));
      const before = r.reads.length;
      let waited = 0;
      while (r.reads.length === before) {
        await vi.advanceTimersByTimeAsync(1_000);
        waited += 1_000;
      }
      waits.push(waited);
    }
    expect(waits).toEqual([PERIOD, 2 * PERIOD, 4 * PERIOD, CEILING_MS, CEILING_MS]);
    expect(r.log.failures).toHaveLength(5);
    await r.answer();
    const before = r.reads.length;
    await vi.advanceTimersByTimeAsync(PERIOD);
    expect(r.reads).toHaveLength(before + 1);
  });

  it("says a busy gateway is busy and does not ask again before its Retry-After", async () => {
    const r = rig(30_000);
    r.refresher.restart();
    await r.fail(new ApiError(503, "admin_busy", "busy", 90_000));
    expect(r.log.failures[0]).toMatchObject({ busy: true });
    await vi.advanceTimersByTimeAsync(89_000);
    expect(r.reads).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(1_000);
    expect(r.reads).toHaveLength(2);
  });

  it("stops on a refused credential and reads nothing more until restarted", async () => {
    for (const status of [401, 403]) {
      const r = rig();
      r.refresher.restart();
      await r.fail(new ApiError(status, "admin_credential_invalid", "refused"));
      expect(r.log.refused).toBe(1);
      expect(r.log.failures).toHaveLength(0);
      await vi.advanceTimersByTimeAsync(60 * 60_000);
      expect(r.reads).toHaveLength(1);
      r.hide(true);
      r.hide(false);
      await vi.advanceTimersByTimeAsync(PERIOD);
      expect(r.reads).toHaveLength(1);
    }
  });

  it("drops the running request on a query change, reads at once, and never lets the late answer replace the new one", async () => {
    const r = rig();
    r.refresher.restart();
    const old = r.reads[0];
    r.refresher.restart();
    expect(old?.signal.aborted).toBe(true);
    expect(r.reads).toHaveLength(2);
    r.reads[1]?.ok("new");
    await vi.advanceTimersByTimeAsync(0);
    old?.ok("old, late");
    await vi.advanceTimersByTimeAsync(0);
    expect(r.log.data).toEqual(["new"]);
    // And a late failure of the old query reports nothing either.
    const third = rig();
    third.refresher.restart();
    const stale = third.reads[0];
    third.refresher.restart();
    stale?.fail(new ApiError(0, null, "down"));
    await vi.advanceTimersByTimeAsync(0);
    expect(third.log.failures).toHaveLength(0);
  });

  it("restarts the period from a query change", async () => {
    const r = rig();
    r.refresher.restart();
    await r.answer();
    await vi.advanceTimersByTimeAsync(40_000);
    r.refresher.restart();
    await r.answer();
    await vi.advanceTimersByTimeAsync(PERIOD - 1);
    expect(r.reads).toHaveLength(2);
    await vi.advanceTimersByTimeAsync(1);
    expect(r.reads).toHaveLength(3);
  });

  it("reads nothing but on a restart when the period is off", async () => {
    const r = rig(0);
    r.refresher.restart();
    await r.answer();
    await vi.advanceTimersByTimeAsync(60 * 60_000);
    r.hide(true);
    r.hide(false);
    await vi.advanceTimersByTimeAsync(60 * 60_000);
    expect(r.reads).toHaveLength(1);
    r.refresher.restart();
    expect(r.reads).toHaveLength(2);
  });

  it("holds a restart that happens in a hidden tab until it is visible", async () => {
    const r = rig();
    r.hide(true);
    r.refresher.restart();
    expect(r.reads).toHaveLength(0);
    r.hide(false);
    await vi.advanceTimersByTimeAsync(0);
    expect(r.reads).toHaveLength(1);
  });

  it("stops reading when stopped", async () => {
    const r = rig();
    r.refresher.restart();
    await r.answer();
    r.refresher.stop();
    await vi.advanceTimersByTimeAsync(10 * PERIOD);
    expect(r.reads).toHaveLength(1);
  });
});
