# Tasks

## 1. Snapshot

- [ ] 1.1 Move everything derived from the config into `Snapshot` behind a lock; handlers take one `Arc` per request
- [ ] 1.2 Reuse unchanged breakers (pool) and route instances (routing); adjustable `restricted_at`

## 2. Reload

- [ ] 2.1 `reload()` (read, validate, build, check restart-only settings, swap) with serialization and recorded outcome; SIGHUP wired in `main`/`run`
- [ ] 2.2 Fingerprint and reload status in `GET /v1/health`
- [ ] 2.3 Tests: rule edit takes effect, in-flight stream unaffected, bad file keeps old, restart-only refused, breaker/route state kept, health report, concurrent reloads

## 3. Docs

- [ ] 3.1 ADR 0012; README; docs/deployment.md (reload under Docker, Kubernetes, launchd); docs/routing.md; architecture; example
