# Design

## Context

`TargetClient::call_detailed` walks a target's endpoints in order and fails over per request,
statelessly. `fails_over` already classifies which errors are endpoint-specific.

## Decisions

- **Per endpoint (provider + model), in memory.** Rate limits are usually per account, but 5xx are
  often per model; endpoint granularity needs no config and the cooldown is short. State resets on
  restart, which is acceptable (the first requests re-learn it).
- **Classic breaker with exponential cooldown and a single probe.** closed -> (threshold failures)
  -> open until `now + cooldown` -> half-open: one request admitted, others skip -> success closes
  (cooldown resets), failure reopens with doubled cooldown (capped).
- **Count exactly the `fails_over` errors.** Single source of truth for "endpoint problem";
  client errors (400/401/403/404) say nothing about the endpoint's health.
- **Fail open inside the pool** (see proposal). This is why target-level policy integration is
  deferred: a policy cannot know whether a route has any healthy alternative, so making a cold
  target ineligible could turn slow into 503. A later change can pass route topology to a health
  policy and add a fall-back-to-unfiltered step.
- **Injected `Clock`** so tests control time; no sleeping in tests.
- **A pure `health` module** (core layer: clock + config only); the pool drives it. Probe
  admission is a state transition under one mutex, so concurrent requests cannot all probe.
- **Streams:** success is recorded when the response starts (headers OK); failures after the
  stream starts are not counted (nothing fails over then either).

## Risks / Trade-offs

- A burst of concurrent failures can overshoot the threshold slightly; harmless.
- One probe request can be slow when the endpoint is still dead (it pays the timeout once per
  cooldown); bounded by `max_cooldown_secs`.
- With threshold 3, three genuine 429s in a row also open the breaker: intended, since the
  endpoint is rate-limiting us.
