# Design

## Context

Routing and the policy seam are in place (`routing.rs`, `policy.rs`), dispatch runs on
Switchyard's `run` over per-target clients (`pool.rs`), and nothing identifies callers or counts
usage. Facts from Switchyard 0.3.0 that shape this design:

- `Usage` is normalized: non-cached `input_tokens`, cache read/creation tokens, `output_tokens`,
  `reasoning_tokens`. Streams carry `LlmResponseChunk::Usage`.
- `run`'s observer reports `usage` only for **buffered** calls. A streamed answer needs a tap on the
  stream (Switchyard's own server wraps the stream and records at the terminal event).
- Judge calls and the answer call both go through our target clients; `run` returns the answer.

## Goals / Non-Goals

**Goals:** accurate per-key spend including judge calls; budgets that survive restarts; no
ledger I/O on the request path; an identity layer that a database-managed key store can replace
later without touching the ledger or budget logic.

**Non-Goals:** managed key CRUD, teams, estimating tokens when a provider omits usage, alerts.

## Decisions

- **Identity behind a trait.** `KeyStore` (async, `lookup(hash) -> Option<KeyRecord>`;
  `KeyRecord { id, allowed_routes, limits, over_budget }`) with a config-backed implementation now.
  Everything downstream (budget tracker, ledger, policy, key info) uses the stable key **id**, never
  the hash. **Moving to database-managed keys later is then a new change that adds:** a SQLite `keys`
  table using the same hash scheme, a `DbKeyStore` with a short-TTL cache, an admin API protected by
  an admin key, and `import-keys` to copy config keys into the table (config keys can stay as a
  read-only seed). No ledger migration is needed because ledger rows reference key ids.
- **Keys are hashed.** `keygen` makes 32 random bytes, prints `sk-conductor-<base32>` once and the
  `sha256:` hash for config. High-entropy random keys make an unsalted SHA-256 adequate and keep
  lookup a hash-map hit. Keys and hashes never reach logs.
- **Auth runs before routing**, accepting `Authorization: Bearer` and `x-api-key` (Anthropic
  clients use the latter). With no keys configured the gateway is open and unbudgeted.
- **Prices live on endpoints** (`price = { input, output, cached_input }`, USD per million tokens).
  USD per million tokens equals micro-USD per token, so cost is computed in integer **micro-USD**
  and stored as `INTEGER`, avoiding float drift in sums. Budgets with USD limits require every
  endpoint to declare a price (zero allowed) so nothing is silently free.
- **Per-request contextual clients.** `run` takes a `ClientRouter` per call. For each request we
  wrap the route's target clients in a thin `Metered` client bound to a `CallContext` (key id,
  route, session id, ledger sender, a pending-entries list). This avoids task-local or header
  tricks to carry identity into judge calls. The inner target client now also reports which
  endpoint served (provider, model, price).
- **Recording flow.** Each served call creates a pending entry with a call id; the id travels back
  on the response in an internal header that is stripped before the client sees it.
  - Buffered responses: entry completed immediately from `Usage`.
  - Streams: the IR stream is wrapped; the entry completes at the terminal event, or on drop (client
    disconnect) with usage seen so far and a missing-usage marker if none arrived.
  - After `run` returns, the entry whose call id is on the returned response is the **answer**; the
    rest are **judge** calls. If `run` fails, completed entries are still recorded as judge calls:
    tokens were spent.
- **Ledger writes are asynchronous.** Completed entries go to a bounded `mpsc`; one writer task
  batches them into SQLite transactions (up to 100 rows or 200 ms). A full channel or failed write
  is logged and counted and never fails the request. The same completion step updates the
  in-memory budget counters immediately, so enforcement does not depend on the writer.
- **Budget tracker** keeps per-key counters for the current UTC day and month (micro-USD and
  tokens), hydrated at startup with `SUM ... GROUP BY key_id` over the ledger, and rolls periods
  lazily on access. The clock is injectable for tests.
- **Enforcement order:** authenticate → allowlist (403) → tracker check. Exhausted → 402 unless
  `over_budget = "free_only"`. Restricted/free-only are applied through a `BudgetPolicy` that
  implements `RoutingPolicy`, so tier substitution and the 503 path come for free.
  `PolicyContext` gains the key id and budget state. A target's price for policy purposes is the
  **maximum** across its endpoints (conservative): a target with an expensive failover endpoint
  counts as expensive; "free" means every endpoint is priced zero.
- **Key info** (`GET /v1/key/info`) reads the tracker; it never queries SQLite.

## Risks / Trade-offs

- **In-flight overshoot**: spend is known only at completion, so concurrent requests can pass a
  limit → documented bound; new requests are refused once observed. Reservation of an estimated
  max cost is a possible later refinement.
- **Providers that omit usage in streams** record zero cost with a marker → warn in logs, surface a
  counter; token estimation is out of scope. Verify `stream_options.include_usage` behavior against
  real providers in the smoke tests.
- **Unsalted SHA-256** is fine for random keys but not for human-chosen ones → `keygen` is the only
  documented way to mint keys; document it.
- **SQLite single writer** is plenty for this write rate; WAL mode and one writer task avoid lock
  contention. Postgres for team deployments is a later change.
- **Clock/timezone**: UTC periods only; documented.

## Migration Plan

Opt-in: with no keys the behavior is unchanged. Configs with `[keys.*]` must also declare
`[ledger]`. Existing ledgers are created on first start with `user_version`-guarded migrations.

## Open Questions

- Default for `restricted_max_output_price` when unset (proposal: unset means no price-based
  restriction, only exhausted handling); confirm during implementation of 5.x.
