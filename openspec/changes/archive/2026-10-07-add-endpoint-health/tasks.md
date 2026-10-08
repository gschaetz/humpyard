# Tasks

## 1. Health state machine

- [x] 1.1 `src/health.rs`: breaker per endpoint with injected clock (closed/open/half-open, doubling capped cooldown, single probe, snapshot); unit and property tests (state never skips, cooldown capped, one probe)
- [x] 1.2 `[health]` config with defaults and validation; config tests; example config

## 2. Pool integration

- [x] 2.1 Pool consults and updates health in the endpoint walk using `fails_over`; fail-open ordering when all are cold; logging of transitions
- [x] 2.2 Integration tests with mock providers: dead first endpoint is skipped after the threshold, recovery probe restores it, all-cold still serves, client errors do not count, disabled setting

## 3. Observability

- [x] 3.1 `GET /v1/health` (auth like other reads); tests

## 4. Docs

- [x] 4.1 ADR 0009; docs/routing.md (failover + health), docs/architecture.md (diagram, status), README status line, examples/config.toml, AGENTS.md if conventions changed; fix stale "budget policies are planned" text in docs/routing.md; verify claims against code
