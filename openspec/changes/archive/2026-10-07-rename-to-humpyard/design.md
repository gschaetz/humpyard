# Design

## Context

The name appears in the repository, the crate (`switchyard-conductor`, library
`switchyard_conductor`), docs, examples, `NOTICE`, tests, and in four identifiers visible to
clients or operators: two response headers, the generated key prefix, and `owned_by` in the model
list. See proposal.md - Why.

## Decisions

- **Rename the public identifiers too.** Pre-alpha with no known users makes this the cheapest
  moment; later it would be a breaking change with a migration. The key prefix is cosmetic for
  authentication (lookup is by hash), so existing keys keep working.
- **Leave archived changes and ADRs untouched.** They are the historical record of past decisions;
  rewriting them would falsify history. ADR 0008 mentions `x-conductor-target`, and accepted ADRs
  are immutable, so it keeps the old header name (noted in `AGENTS.md`). Living specs and docs are
  updated.
- **No compatibility shims** (no dual headers): nothing depends on them yet, and shims would outlive
  their purpose.
- **Keep the generic example values** `x-app = "conductor"` in tests and specs: they stand for an
  arbitrary application header value, not the project name.
- **Local directory and remote.** The git remote is updated to the new URL; the local checkout's
  directory name is the developer's choice and is not part of the repository.
