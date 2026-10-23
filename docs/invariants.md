# Architecture invariants

Rules that must stay true as the code changes. Each names what enforces it, so a violation fails a
build instead of relying on review. When a rule changes, change it here, in the enforcing test and
in an ADR (`docs/adr/`) in the same change.

## Structure

1. **Engine boundary.** Switchyard crates (`switchyard_libsy`, `_protocol`, `_translation`,
   `_llm_client`, `_runner`) are used only by the engine and edge modules (`error`, `pricing`,
   `pool`, `routing`, `metering`, `server`). Core modules (`clock`, `config`, `policy`, `auth`,
   `ledger`, `budget`) use gateway-owned types. Reason: Switchyard is pre-1.0; its churn must stay
   in a few files. *Enforced by* `tests/architecture.rs`.
2. **Layering.** Core modules may reference only the modules listed for them in the rules table
   (for example `budget` may use `clock`, `config`, `ledger`, `policy`); engine modules may not
   reference the HTTP edge (`server`). *Enforced by* `tests/architecture.rs`; a new module fails
   the test until it is given a layer.
3. **SQL stays in the ledger.** Only `src/ledger.rs` uses `sqlx`. *Enforced by* `tests/architecture.rs`.
4. **Only `main` prints.** Everything else logs through `tracing`. *Enforced by* `tests/architecture.rs`.
4b. **Quota never affects a request.** Provider usage is polled off the request path with a timeout;
    a slow, failing or malformed source only makes the snapshot stale, and stale or missing quota is
    reported as unknown, never guessed. The provider key goes only to that provider's own origin over
    https and is never logged or exposed (ADR 0015). *Enforced by* `tests/quota.rs`.
4a. **Serving never fetches prices.** Only `main` (the `humpyard prices` command) may use the
    `prices` module that reads price sources; prices reach the gateway through the config file only
    (ADR 0014). *Enforced by* `tests/architecture.rs`.

## Supply chain and builds

11. **Dependencies stay permissive, advisory-free and from crates.io.** Licences outside the
    allow-list in `deny.toml`, known vulnerabilities, yanked crates and non-crates.io sources fail
    the build. *Enforced by* the `deny` CI job (`cargo deny check`).
12. **The declared minimum Rust version builds** (`rust-version` in `Cargo.toml`), the docs build
    without warnings, and line coverage does not fall below the floor (currently 92%; baseline
    96.15% on 2026-10-07). The floor only ratchets up. *Enforced by* the `msrv`, `docs` and
    `coverage` CI jobs.

13. **Every commit is signed off (DCO).** Each non-merge commit in a pull request carries a
    `Signed-off-by` line, as `CONTRIBUTING.md` requires. *Enforced by* the `dco` CI job.

## Request path

5. **Order of checks.** A request is authenticated, checked against the key's route allowlist, and
   checked against its budget *before* any upstream call is made. *Enforced by* `tests/auth.rs`
   (401/403 make no upstream call) and `tests/budget.rs` (402 makes no upstream call).
6. **Routing never sees ineligible targets.** The policy narrows targets before any algorithm
   runs. *Enforced by* `tests/policy.rs`.
7. **Ledger writes never block a response, and are flushed before a graceful exit.** Entries go
   through a bounded queue with `try_send`; a full queue or failing database is counted, not
   surfaced. On SIGTERM/SIGINT `server::run` drains, cancels what outlives the grace period, and
   flushes the queue before returning; a request cancelled mid-flight still records the calls it
   paid for. *Enforced by* the ledger tests (`a_full_queue_drops_and_counts_instead_of_waiting`,
   `a_broken_database_never_blocks_or_fails_recording`, the `flush_*` tests), `tests/shutdown.rs`
   (including `run_waits_for_a_stalled_ledger_writer_before_returning`), `tests/shutdown_process.rs`
   (real signals against the binary) and `tests/usage.rs`
   (`a_request_cancelled_mid_flight_still_records_the_judge_call_it_paid_for`).
14. **Health never denies service.** A target whose endpoints are all cooling down is still tried
    unless another target remains (then it is left out of the plan, never the last one), a request is
    never refused because of health, and client errors never count against an endpoint. *Enforced by*
    `tests/health.rs` and `tests/health_eligibility.rs`
    (`health_never_denies_service_when_everything_is_cold`, `a_single_cold_target_is_still_tried`); see
    [ADR-0009](adr/0009-endpoint-health-lives-in-the-pool.md) and
    [ADR-0018](adr/0018-cold-targets-are-skipped-only-when-an-alternative-remains.md). The one deliberate
    exception is a [runtime override](routing.md#runtime-overrides): an operator who disables the last target of a
    route gets a 503 `override_no_target`, because that is what they asked for and removing the override restores
    service at once. Health itself still never refuses a request. *Enforced by* `tests/overrides_api.rs`
    (`disabling_the_last_target_refuses_and_removing_it_restores_service`).

15. **A reload is all or nothing.** A config that fails to load, validate or build, or that changes
    a setting a live process cannot change, leaves the running configuration untouched, and a
    request uses one configuration from start to finish. *Enforced by* `tests/reload.rs` and
    `tests/reload_process.rs`; see [ADR-0012](adr/0012-config-snapshot-reloaded-on-sighup.md).

16. **Decisions are metadata only, and recording them never hurts a request.** A decision holds
    routing facts, never a prompt or a response; client-supplied text is clipped; the write path
    is the ledger's bounded queue, so a failing database cannot fail or delay a request; a key
    reads only its own decisions. *Enforced by* `tests/decisions.rs` (including
    `prompts_and_responses_never_reach_the_database` and
    `a_broken_decision_table_never_affects_requests`); see
    [ADR-0013](adr/0013-decisions-recorded-per-request.md).

17. **Metric labels never carry client strings, and metrics are off the request's critical path.**
    Label values come only from the loaded configuration or fixed sets, so a client cannot grow
    the series or put content into them; key ids are opt-in, only on four counters and capped;
    `/metrics` has its own credential that opens nothing else; recording is non-blocking and never
    fails a request. *Enforced by* `tests/metrics_registry.rs`
    (`a_clients_strings_never_become_labels`, `distinct_key_labels_are_capped`) and `tests/metrics.rs`;
    see [ADR-0016](adr/0016-metrics-are-one-registry-with-bounded-labels.md).

18. **A strict route never calls a target outside its allowed classes.** Targets of other classes (and
    unclassed ones) are removed before the routing algorithm runs, so no failure, retry or tier
    substitution can reach them; the configuration fails to load if a strict route mentions an
    unclassed target or has no allowed target; health, budgets, selectors and headers can narrow a
    strict route but never add a target back. *Enforced by* `tests/strict_fallback.rs`
    (`a_strict_route_never_calls_a_disallowed_target`,
    `a_cooling_down_allowed_target_is_still_tried_and_nothing_is_added`,
    `strict_routes_are_validated_when_the_config_loads`); see
    [ADR-0017](adr/0017-strict-routes-exclude-disallowed-classes-before-routing.md).

19. **A target is removed for capability only on certainty, and only removed.** A target that cannot serve a
    request (a declared `tools = false` with tools, `vision = false` with images, or a prompt whose
    deliberately low estimate exceeds every declared window) is left out of the plan before the
    algorithm runs; undeclared means capable; a borderline prompt is never removed; capability never
    adds a target; a request no target can serve is refused with 400 `no_capable_target` before any
    upstream call. *Enforced by* `tests/capability_eligibility.rs`
    (`a_borderline_prompt_is_not_removed`, `an_endpoint_that_declares_nothing_is_capable`,
    `a_request_nothing_can_serve_is_refused_before_any_upstream_call`,
    `capability_never_adds_a_target_to_a_strict_route`); see
    [ADR-0019](adr/0019-declared-capabilities-remove-targets-that-cannot-serve-a-request.md).

20. **Ordering by speed ranks only what eligibility left.** A `fastest` route orders the targets that
    remain after class, policy, capability and health removal; it never adds a removed target back however
    fast its figure, and only answered attempts update a figure (a failure is never a speed sample).
    *Enforced by* `tests/latency_ordering.rs`
    (`ordering_never_brings_back_a_target_that_eligibility_removed`,
    `a_failed_attempt_is_not_a_speed_sample_and_the_request_falls_over`); see
    [ADR-0020](adr/0020-latency-ordered-routes-rank-only-what-eligibility-leaves.md).

21. **Strengths decide what a pool contains and nothing else.** A target's strengths are read only when a
    pool route's membership is resolved, at configuration load; they never remove a target from a request,
    and an undeclared context window never qualifies a target for a size-limited pool. *Enforced by*
    `tests/strengths_pools.rs` (`a_strict_route_narrows_a_pool_per_request_and_never_widens_it`,
    `a_minimum_window_counts_only_declared_windows_and_says_who_was_left_out`); see
    [ADR-0021](adr/0021-strengths-build-pools-at-load-time-and-never-remove-a-target.md).

22. **A dead endpoint cannot fail every request of a pool.** A 404, 401 or 403 (or a missing-model 400)
    counts against its endpoint, so it cools and is skipped; on `fastest` and pool routes it also moves the
    request to the next target. A 400 or 422 is the request's error and ends it. *Enforced by*
    `tests/unavailable_endpoints.rs` (`a_dead_first_target_no_longer_fails_a_fastest_route_or_a_pool`,
    `request_errors_still_end_the_request`, `other_routes_keep_their_behavior_unless_they_opt_in`); see
    [ADR-0023](adr/0023-answers-that-describe-the-endpoint-make-it-unavailable.md).

## Data and secrets

8. **No plaintext secrets.** Provider keys come from environment variables; client keys exist only
   as SHA-256 hashes in config; neither keys nor hashes appear in logs. *Enforced by* the config
   tests, `tests/auth.rs` (log checks) and `Debug` redaction tests.
9. **Money is integer micro-USD, and numeric conversions are checked.** Cost is computed and
   stored as whole micro-USD; floating point appears only at the config and display edges, and
   every integer/float conversion on money and token paths goes through `src/num.rs`. *Enforced
   by* the denied `clippy::cast_*` lints in `Cargo.toml` (run by CI), the `num` and `pricing`
   tests, and, for production code, denied `unwrap_used`, `expect_used` and `panic`. Property
   tests are planned (`harden-engineering` group 6).
10. **The ledger refers to key ids, never hashes.** Identity changes (for example database-managed
    keys) must not require a ledger migration. *Enforced by* review and the ledger schema; see
    [ADR-0003](adr/0003-config-first-virtual-keys-behind-keystore.md).
