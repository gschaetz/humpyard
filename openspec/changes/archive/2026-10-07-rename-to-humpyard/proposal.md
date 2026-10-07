# Proposal

## Why

The project is being named **humpyard**, after the railroad hump yard where cars are classified
and sent down the right track: a fitting picture for a gateway that sorts each request onto the
right model. The old name embedded another project's name (Switchyard), which is also a dependency
we do not want to be confused with. The domain humpyard.dev is registered.

## What Changes

- The GitHub repository is renamed to `gschaetz/humpyard` (done; GitHub redirects the old URL).
- The crate, library and binary become `humpyard`; every reference in code, tests, docs, examples
  and `NOTICE` follows.
- **BREAKING** public identifiers carry the new name: response headers `x-conductor-target`,
  `x-conductor-provider` become `x-humpyard-target`, `x-humpyard-provider`; minted client keys are
  prefixed `sk-humpyard-` instead of `sk-conductor-`; the model-list `owned_by` value is
  `humpyard`; the example ledger file is `humpyard.db`.
- Archived OpenSpec changes are left untouched as history.

Out of scope: any behavior change, wiring the domain (DNS, site), publishing the crate.

## Capabilities

### New Capabilities

### Modified Capabilities
- `request-routing`: the serving-attribution headers are renamed.

## Impact

- Everything that mentions the old name; `Cargo.toml`/`Cargo.lock`, `tests/*` imports, docs.
- Existing keys keep working (authentication compares hashes, not prefixes); only newly minted
  keys use the new prefix. Clients that read the old headers must switch. No known users yet
  (pre-alpha).
