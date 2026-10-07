# Tasks

## 1. Spike: confirm Switchyard integration points

- [x] 1.1 Write a throwaway test that builds `StageRouter` and `Passthrough` via libsy constructors, runs `run_stream` with a filtered `RuntimeModels`, and verify a filtered-out target is never selected; recorded in design.md, kept as `tests/switchyard_assumptions.rs`
- [x] 1.2 Verify whether `switchyard-llm-client::run` streams the final answer or buffers it, and whether `decide` plus our own dispatch is needed; record the decision in design.md and update tasks 4.x if it changes
- [x] 1.3 Verify session state persists across two `run_stream` calls on one algorithm instance with the same session id (escalation/stage latch); record the result in design.md

## 2. Configuration

- [x] 2.1 Implement `[[providers]]`, `[[targets]]` (ordered endpoints) and `[[routes]]` config with `deny_unknown_fields`, dangling-reference and duplicate-name checks; unit tests cover each spec scenario
- [x] 2.2 Update `check-config`, `examples/config.toml` and README for the new shape and verify the example passes `check-config`

## 3. Provider pool

- [x] 3.0 Move dispatch onto `run` with one implicit passthrough route and `TranslatingLlmClient` backends, replacing the hand-written relay; verify all existing `tests/gateway.rs` tests still pass
- [x] 3.1 Implement the per-target wrapper `RoutedLlmClient` that walks endpoints with per-endpoint model name and key; integration tests with mock providers cover ordering and model-name mapping
- [x] 3.2 Implement failover rules (429/5xx/connect/timeout fail over; other 4xx do not; none after first streamed byte); tests cover every failover scenario in provider-pool
- [x] 3.3 Keep error mapping per spec when failover is exhausted; tests assert last-error semantics for 429, 502, 504

## 4. Routing

- [x] 4.1 Build `passthrough` and `random` routes and direct-target implicit routes at startup; tests assert requests reach the expected target
- [x] 4.2 Build `stage_router` routes; test with failing tool-result history that the capable target is chosen per config
- [x] 4.3 Build `llm_classifier` routes with the judge served through the pool; test that judge calls reach the judge target and are invisible to the client
- [x] 4.4 Wire session ids from `x-switchyard-session-id`; test that state persists across two requests of one session and not across sessions
- [x] 4.5 Apply ordered Switchyard fallbacks across whole targets; test the capable-unavailable scenario
- [x] 4.6 Set `x-conductor-target` and `x-conductor-provider` for streaming and buffered responses and add them to the request log; tests assert both

## 5. Routing policy seam

- [ ] 5.1 Define the policy trait, `PolicyContext`, and the allow-all default; wire eligibility into the per-request runtime model set (remove from every category, rebuild `Any`, substitute emptied tiers); tests cover default, removal and tier-substitution scenarios
- [ ] 5.2 Return 503 in the endpoint's error shape when no target is eligible; test all three protocols
- [ ] 5.3 Update `/v1/models` to list routes and targets; test the listing

## 6. Docs and integration

- [ ] 6.1 Document routes, targets, failover and the policy seam in README and `docs/`, and verify the documented config runs against mock providers
- [ ] 6.2 Update docs/architecture.md (status table and both diagrams) to match the code and verify the diagrams against the module list
- [ ] 6.3 End-to-end test: Claude-Code-style Anthropic client with tool-error history escalates from efficient to capable through the full stack
