# Tasks

## 1. Observe (no routing behavior)

- [x] 1.1 Quota snapshot model and the OpenCode Go adapter: strict parsing of recorded fixtures (valid, clamped over 100, malformed window, no usable window, unknown status text); tests without network
- [x] 1.2 Config: `quota_source`, `quota_poll_secs` (default 60, 1 to 3600; the minimum is low so tests can poll quickly, 15 or more is sensible in production), `quota_stale_secs` (default 300); validation; origin and https rules for the poll URL
- [x] 1.3 Poller task in `serve`: off the request path, timeout, no key in logs or snapshots; tests for hung, failing and malformed sources never affecting requests, and for staleness
- [x] 1.4 `GET /v1/quota` with snapshot age and unknown reasons; reload keeps the snapshot of unchanged providers; tests
- [x] 1.5 Docs and ADR 0015 accepted; an architecture note that the poller is the only runtime source fetch

## 2. Measure

- [x] 2.0 Record quota samples in the ledger (schema v3) and expose `GET /v1/quota/history`: a sample is stored when a window's percent, status or reset changes, and at least every 15 minutes; each sample carries the provider's notional cost since the previous one, from the usage rows; kept for `decision_retention_days`; tests (schema upgrade, sampling rule, history query, auth, no ledger)
- [ ] 2.1 Run it against the real account for several days; compare percent deltas with ledger cost; record what the percent is a percent of, the `status` values seen, and the behavior at a cap (decision input for step 3)
  - Findings so far (7 days, 2026-10-17 to 10-24, 17,802 samples from the live gateway); the task stays open until a cap is observed:
    - The percent is an integer and account-level, and it moves for usage outside the gateway too: the monthly window reached 16% while the cost the gateway recorded was cents ($0.12 and $0.06 on 10-18, $0.015 on 10-22).
    - One percentage point stepped for about $0.015 to $0.12 of gateway cost, so percent deltas cannot be matched against ledger cost except across bursts. The cost on each sample counts every key's calls to the provider.
    - Only status `ok` was seen. The highest values were rolling 5, weekly 9 and monthly 16, so behavior at a cap is not yet observed.
    - Resets: the rolling window's `resets_at` slides with the clock (about 5 hours ahead) while idle; the weekly window resets Monday 00:00 UTC; the monthly window reset on 10-23 at 23:37 UTC.
    - For step 3: thresholds in a `when.quota` rule can only be coarse (whole percents), and a rule must treat a missing or stale snapshot as unknown, as designed.
    - Reading the data needs a key that sees the traffic: `decisions` and `stats` are scoped to the calling key, but `/v1/quota/history` is not.

## 3. Use it in rules (after step 2)

- [ ] 3.1 `when.quota` condition, validation, `explain` and decision recording; tests (match, below, unknown, stale, each window, invalid configs)
- [ ] 3.2 Pacing recipe in `docs/routing.md` and the auto-routing example; verify with `explain`

## 4. Later, separate proposals

- [ ] 4.1 Per-model quota if a provider publishes it; adapters for other providers
- [ ] 4.2 Anything that acts on quota without an operator rule (advisory ordering), only after weeks of statistics
