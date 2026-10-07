# Architecture Decision Records

Durable decisions that outlive any one change. They are created by the `adr` step of the
`spec-driven-adr` OpenSpec workflow (see `openspec/schemas/spec-driven-adr/schema.yaml`) when a
change makes a long-term architectural commitment: a pattern, technology, boundary or contract.
Tactical choices stay in the change's `design.md`.

**Accepted ADRs are immutable.** To change a decision, add a new ADR with
`Status: accepted, supersedes ADR-NNNN` and a `Supersedes:` line; never edit the old file. What is
in force is found by following `Supersedes:` links. Files are named `NNNN-kebab-title.md`,
numbers are monotonic and never reused. Start from [template.md](template.md).

How decisions relate to enforcement: invariants that must hold mechanically are listed in
[../invariants.md](../invariants.md) (once written) and checked by tests and CI.
