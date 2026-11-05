# Design

## Context

`retired::check` reads each provider's list with `discover::fetch_provider_models_within`, which returns `DiscoverError::Source(String)` for every failure: unreachable host, an error status, a body that is not JSON, an oversized body. The check then turns any error into `Listing::Unusable`, and `Report::exit_code` returns 2 when any provider is unusable.

Observed on 2026-11-04 in the cluster (45 endpoints, exit 2): `codestral` answered 404 for `https://codestral.mistral.ai/v1/models` (it answers 404 without a key too, so the host has no such route), and a local Ollama provider on a laptop (reached at a fixed LAN address, designed to be asleep or away at times) answered 503 from inside the mesh and refused connections from outside. Neither is a problem with the configuration. A job that alerts on exit 2 would be red every week.

In-force ADRs read: 0014 (third-party data informs, never fetched while serving), 0022 (discovery reads a provider's own list; command-only), 0023 (answers that describe the endpoint). No ADR changes.

## Goals / Non-Goals

**Goals:**
- Exit 2 means a provider answered and the answer was unusable, so it is worth looking at.
- A provider with no list, and one that cannot be reached, are said out loud in the report without failing the check.

**Non-Goals:**
- A per-provider opt-out in the configuration.
- Changing what discovery does with a failed list.
- Judging whether an unreachable provider is healthy (that is the health checks' and `humpyard_endpoint_unavailable_total`'s job).

## Decisions

**1. The fetch returns what kind of failure it was.** `fetch_provider_models_within` gets a result error type that says `NoList`, `Unreachable` or `Unusable` along with the message, and `fetch_provider_models` (discovery's entry point) maps all of them to the existing `DiscoverError::Source` so discovery's behavior and messages do not change. Classification: a status of 404, 405 or 501 is `NoList`; a connect error, a timeout (the time limit) and 502, 503 or 504 are `Unreachable`; everything else (401, 403, other 4xx and 5xx, a body that is not a model list, an oversized body, an empty list) is `Unusable`.

**2. The exit code follows only `Unusable` and retired endpoints.** `exit_code` is 2 when any provider is unusable, else 1 when any endpoint is retired, else 0, as before; `NoList` and `Unreachable` appear in the report and change nothing. A provider that is permanently dead would therefore be reported as unreachable every run without failing it; it is visible on the health metrics and the counter instead, and the report line keeps it in front of the operator.

**3. Report shape.** Text: `no list:   codestral answers 404 for /models` and `unreachable: local-laptop: <reason>`. JSON keeps `checked`, `retired`, `unusable` and `skipped` (providers with no key) as they are and adds `no_list` and `unreachable`, each a list of `{provider, reason}`.

## Risks / Trade-offs

- **A 404 that means something else.** A provider whose base URL is wrong would answer 404 for `/models` and be skipped silently. The report still names it, and a wrong base URL also shows up as every request to it failing, which the counter and the health checks surface.
- **A 503 that means the provider is overloaded**, not absent, is reported as unreachable and not failed. Acceptable for a report-only check that cannot tell them apart.
- **Hiding a permanently dead provider** from the exit code; mitigated by the report line and the other signals.

## Open Questions

None.
