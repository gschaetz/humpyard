# Tasks

## 1. Matching

- [ ] 1.1 `src/select.rs` (core): config types, glob matching, facts, ordered first-match with a permit predicate; unit and property tests
- [ ] 1.2 `[[select]]` parsing and validation in config (routes exist, unknown conditions, credential headers, duplicate names, unreachable rules)

## 2. Server

- [ ] 2.1 Build facts (lower-cased headers minus credentials, key, agent metadata, stream); select in the authorize stage for the request endpoints and token counting; `x-humpyard-route` and `x-humpyard-rule`; ledger records the served route
- [ ] 2.2 End-to-end tests: header/profile/tag/key/agent selection, first match, no match, narrow-never-widen, 404/403 unchanged, response headers, ledger route

## 3. Docs

- [ ] 3.1 ADR 0011; docs/routing.md (selectors, standard headers), clients.md, example config, architecture status, invariants note; verify claims
