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

How decisions relate to enforcement: invariants that must hold mechanically are listed in
[../invariants.md](../invariants.md) and checked by tests and CI.
