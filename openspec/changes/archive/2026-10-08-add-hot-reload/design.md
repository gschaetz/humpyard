# Design

## Decisions

- **A snapshot, swapped whole.** Everything derived from the config lives in one immutable
  `Snapshot` behind `RwLock<Arc<Snapshot>>`. A request clones the `Arc` once at its start and uses
  it throughout, so a reload can never change the rules mid-request, and streams keep the
  targets they started on alive. Process-wide machinery stays outside it: the ledger writer, the
  budget counters, the clock, the shutdown signal and the embedder's policy.
- **Build first, swap last.** Reading, validating (including env-var keys), building pools,
  routes and selectors all happen before the swap; any failure leaves the old snapshot in place.
  Reloads are serialized; signals that arrive during one are coalesced into one more reload.
- **Carry-over by identity.** An endpoint's breaker is reused when (target, provider, model,
  base URL) and the health settings are equal; a route instance is reused when its spec is equal,
  keeping per-session algorithm state. Anything else starts fresh, which is the honest outcome:
  the thing it described changed.
- **Restart-only settings are refused loudly**, not half-applied: `listen`, the ledger path,
  `shutdown_grace_secs`, and the keyless/keyed switch (the budget tracker is only created for keyed
  gateways).
- **SIGHUP first, optional polling second, no inotify.** SIGHUP works the same under Docker and
  launchd and needs no dependency or credential. A distroless container has no `kill` and
  Kubernetes has no signal API, so `reload_poll_secs` re-reads the file on a timer and reloads
  when a content hash changes. Hashing the content (not trusting timestamps or inotify events)
  is robust to ConfigMap symlink swaps and partial writes; a file that fails to load is attempted
  once per change.
- **The budget threshold is adjustable** on the live tracker (`restricted_at`), while limits come
  from the snapshot's keys on each call, so edited budgets apply immediately.
- **Secrets**: provider keys are read from the process environment on each load. A running
  container's environment cannot change, so rotating a key needs a restart; documented.

## Risks / Trade-offs

- Two requests straddling a reload may see different configs. That is inherent and acceptable;
  each request is internally consistent.
- Reusing route instances means a reload that does not touch a route keeps its latches, which is
  what operators expect.
