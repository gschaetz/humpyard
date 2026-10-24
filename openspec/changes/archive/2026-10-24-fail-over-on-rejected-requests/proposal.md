# Proposal

## Why

A provider that rejects a request's shape answers HTTP 400 or 422, and today that ends the request on every route
(ADR 0023: "a 400 or 422 describes the request"). In a free pool that is the wrong call. Codex through the
`deep-thinker-public` pool (2026-10-23) was broken four separate times by a member that rejected a field the others
accept: Google AI (`store`, `prompt_cache_key`, then a missing thought signature on the next turn), Groq
(`prompt_cache_key`), NVIDIA (`prompt_cache_key`) and Codestral (`store`). Each time the pool had healthy members
behind the strict one, and each time the whole route failed until an operator found the field by replaying the
request and added a `drop_fields` entry. The next strict provider, or a provider that tightens its schema, does the
same.

The answer describes the request *relative to one provider*. When the same request is fine on other members, the
member is the mismatch, not the request. When every member rejects it, the request is the problem.

## What Changes

- A new fallback class `rejected`: an upstream HTTP 400 or 422 that is not already `unavailable` (a model-gone 400).
- On `fastest` routes and pool routes it moves the request to the next endpoint of the target and then the next
  target, by default, like `unavailable`. Other routes keep ending the request unless they list `rejected` in
  `fallback_on`; an explicit `fallback_on` list is taken as written.
- It does not count against the endpoint's health and is not a speed sample (the endpoint is fine, and other clients
  may send requests it accepts), like a 413.
- It is bounded: once three different providers have rejected the same request, it stops and the client gets the last
  rejection, because a request wrong on three providers is wrong.
- The guarded tail (ADR 0024) is unchanged: it is reached only by the classes its `on` list names (`overflow` by
  default), so a request every pool member rejects never reaches a paid tail.

## Capabilities

### New Capabilities

### Modified Capabilities
- `upstream-proxy`: a rejected request moves a pool on, within a bound.

## Impact

`src/config/schema.rs` (the class and its default), `src/error.rs` (classification), `src/metering.rs` (carry it and
stop at the bound), `src/pool.rs` (within-target move, no health count), tests, docs/routing.md,
docs/configuration.md, docs/clients.md, a new ADR 0026 that refines ADR 0023.
