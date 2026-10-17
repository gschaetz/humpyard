# Architecture Decision Records

Durable decisions that outlive any one change. They are created by the `adr` step of the
`spec-driven-adr` OpenSpec workflow (see `openspec/schemas/spec-driven-adr/schema.yaml`) when a
change makes a long-term architectural commitment: a pattern, technology, boundary or contract.
Tactical choices stay in the change's `design.md`.

**Accepted ADRs are immutable.** To change a decision, add a new ADR with
`Status: accepted, supersedes ADR-NNNN` and a `Supersedes:` line; never edit the old file. What is
in force is found by following `Supersedes:` links. Files are named `NNNN-kebab-title.md`,
numbers are monotonic and never reused. Start from [template.md](template.md).

## Index

| ADR | Decision |
|---|---|
| [0001](0001-embed-switchyard-in-process.md) | Embed Switchyard in-process |
| [0002](0002-dispatch-on-switchyards-client.md) | Dispatch on Switchyard's client; providers speak OpenAI Chat |
| [0003](0003-config-first-virtual-keys-behind-keystore.md) | Config-first virtual keys behind a `KeyStore` trait |
| [0004](0004-no-plaintext-secrets.md) | No plaintext secrets in config, logs or the ledger |
| [0005](0005-sqlite-ledger-with-async-writer.md) | SQLite usage ledger with an asynchronous writer |
| [0006](0006-money-as-integer-micro-usd.md) | Money is integer micro-USD |
| [0007](0007-one-shot-modelrelay-migration.md) | One-shot modelrelay migration, not a runtime importer |
| [0008](0008-routing-policy-seam.md) | A routing policy seam in front of Switchyard's algorithms |
| [0009](0009-endpoint-health-lives-in-the-pool.md) | Endpoint health lives in the pool and fails open |
| [0010](0010-tag-only-native-releases.md) | Releases are tag-only and built on native runners |
| [0011](0011-selectors-choose-the-route.md) | Selectors choose the route from request facts; clients narrow, never widen |
| [0012](0012-config-snapshot-reloaded-on-sighup.md) | The configuration is an atomically swapped snapshot, reloaded on SIGHUP |
| [0013](0013-decisions-recorded-per-request.md) | Routing decisions are recorded per request in the ledger database |
| [0014](0014-prices-are-proposed-never-fetched-at-runtime.md) | Prices are proposed by a CLI and never fetched at runtime (proposed) |
| [0015](0015-provider-quota-is-polled-advisory-and-fails-open.md) | Provider quota is polled at runtime, advisory, and fails open |
| [0016](0016-metrics-are-one-registry-with-bounded-labels.md) | Metrics are one registry with bounded labels |
| [0017](0017-strict-routes-exclude-disallowed-classes-before-routing.md) | Strict routes exclude disallowed classes before routing |
| [0018](0018-cold-targets-are-skipped-only-when-an-alternative-remains.md) | Cold targets are skipped only when an alternative remains |
| [0019](0019-declared-capabilities-remove-targets-that-cannot-serve-a-request.md) | Declared capabilities remove targets that cannot serve a request |
| [0020](0020-latency-ordered-routes-rank-only-what-eligibility-leaves.md) | Latency-ordered routes rank only what eligibility leaves |
| [0021](0021-strengths-build-pools-at-load-time-and-never-remove-a-target.md) | Strengths build pools at load time and never remove a target |
| [0022](0022-free-pool-membership-may-be-updated-on-a-schedule-without-review.md) | Free-pool membership may be updated on a schedule without review |
| [0023](0023-answers-that-describe-the-endpoint-make-it-unavailable.md) | Answers that describe the endpoint make it unavailable |
| [0024](0024-a-pool-route-can-end-in-a-guarded-tail.md) | A pool route can end in a guarded tail |

How decisions relate to enforcement: invariants that must hold mechanically are listed in
[../invariants.md](../invariants.md) and checked by tests and CI.
