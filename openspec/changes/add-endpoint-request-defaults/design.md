# Design

## Context

See proposal.md for why. What the code does today, read from `src/pool.rs` and the `switchyard-llm-client` 0.3.0 and
`switchyard-translation` 0.3.0 crates it uses:

- `endpoint_client` builds one HTTP backend per endpoint and sets its `extra_body` to an empty map and its
  `reasoning_effort` to `None`. The backend is per endpoint, so a setting held there is per endpoint, which is the scope
  this change wants.
- The HTTP client merges `extra_body` into the final outbound body **after** translation, and only for keys the body
  does not already have (`merge_extra_body`: `or_insert_with`). That is fill-don't-force, in the library, already tested
  there. The backend's separate `reasoning_effort` is the opposite, a forced override applied after the merge. This change
  uses the first and leaves the second alone.
- A request that arrives in the same format as its target (OpenAI in, OpenAI out) is sent with its original body, so a
  client's `temperature`, `top_k` or `presence_penalty` reaches the provider. A request that arrives in the Anthropic
  format is re-encoded: the OpenAI encoder writes `temperature`, `top_p` and `reasoning_effort` (from the Anthropic
  `output_config.effort`) and a fixed list of extension fields. It drops `top_k` and does **not** write the Anthropic
  `thinking` object at all.
- `provider.drop_fields` is the existing precedent for shaping a request to one provider: configuration the operator
  writes, checked at load, shown in the effective configuration.

What a Claude Code request can express, which the user asked to have checked: Claude Code speaks the Anthropic format. Its
`thinking: {type, budget_tokens}` is kept as raw data on the request and is not carried to an OpenAI-compatible endpoint.
Its `output_config.effort`, when it sends one, becomes `reasoning_effort`. So today a Claude Code session cannot turn
thinking off or on at a `friday` endpoint through the gateway, whatever it asks, and an endpoint default of
`reasoning_effort = "none"` gives that endpoint the behavior the operator chose, unless the client sends an effort, which
wins. This is stated in the configuration guide so the default is not read as honoring or ignoring a thinking request.

## Goals / Non-Goals

**Goals:**
- State a model's own request settings once, on its endpoint, and have every client get them.
- A client that sets a field keeps its value, in either protocol.
- No new work on the request path beyond a merge the client library already does, and no new dependency.

**Non-Goals:**
- Forcing a value over the client's, defaults on a route or key, or a default for a field that changes cost
  (`max_tokens`). Each is a separate decision with its own risks.
- Defaults for an endpoint that does not speak the OpenAI chat protocol. The roadmap's native Anthropic upstream (item 3)
  will need its own list: an Anthropic body has no effort field and the library rejects the setting for it, `min_p` and the
  two penalties are not Anthropic fields, and its natural defaults are `thinking`, `temperature`, `top_p` and `top_k`. Its
  Anthropic-format requests also keep the client's `thinking` object, so the gap in the threat table does not exist there.
- Mapping the Anthropic `thinking` object to an effort. That is the translation library's job; this change does not wrap
  or patch it.
- Recording on a decision which defaults were filled. The library merges internally and does not report which keys it
  added, and recomputing that in the gateway would duplicate its logic. The effective configuration shows what an endpoint
  would add.

## Decisions

1. **The setting lives on the endpoint.** A thinking switch or a temperature is a property of one model on one provider.
   On a provider it would apply to every model that provider serves (one host can serve several), and on a route it would reach
   whichever model the route falls through to, including one that rejects the field. The endpoint is the unit that already
   carries per-model facts (`price`, `context_window`, `tools`, `vision`, `max_request_tokens`).
   *Alternatives:* provider-level (too coarse), route-level (wrong model after a failover), key-level (the client is not the
   one who knows the model).
2. **Fill, never force.** Use the backend's `extra_body`, which does not replace a field the request has. A client that
   asks for something specific gets it; the operator's default covers the clients that ask for nothing.
   *Alternative:* the backend's forced `reasoning_effort`. Rejected for v1: forcing hides what a client asked for, and
   nothing here needs it. If a need appears it is a named `request_overrides`, a separate decision.
3. **A short allow-list per protocol, each field typed and ranged.** Every endpoint is OpenAI-compatible today, so there is
   one list; the check takes the endpoint's protocol as an input so a second protocol adds a second list instead of
   widening this one. Inserting a field the target protocol does not know makes every request fail, which the merge cannot
   detect.
   The list: `reasoning_effort`, `temperature`, `top_p`, `top_k`, `min_p`,
   `presence_penalty`, `frequency_penalty`. These are the sampling and thinking controls the evaluation showed matter and
   that providers share. An open map would let a typo become a request field that a provider rejects, found only when
   traffic fails; `drop_fields` already shows that providers do reject unknown fields. `max_tokens` is left out on
   purpose: the gateway estimates and budgets on it, and a default would change cost behavior.
4. **Validate with the target in hand.** A check in `config/validate.rs`, which walks the raw targets and so knows the
   target name and the endpoint's position, so the message can name both and the field (`max_request_tokens` is checked
   while parsing and its message names only the field, which is not enough here). The table is read untyped so that a wrong
   type is reported by field, not as a parser message. Startup, `check-config` and a reload all load through
   `Config::from_toml`, so all three refuse a bad table; a refused reload keeps the previous configuration serving.
5. **Show it where the rest of an endpoint is shown.** The effective configuration serializes the `Endpoint` struct, so a
   field with `skip_serializing_if` for the empty case appears for the endpoints that have it and is absent for the others,
   as `max_request_tokens` is. The admin GUI's generated types follow the OpenAPI document, which gains the field.
6. **Hand it to the backend as JSON values.** The TOML table is converted to `serde_json::Value`s once at load and kept on
   the endpoint; `endpoint_client` passes it as `extra_body`. Nothing is computed per request.

## Threat model

Assets: the client's request body, the operator's provider spend, a provider's availability. Actors: a client (untrusted),
the operator (trusted, writes the file). Trust boundary: the operator's file supplies fields that are written into a
request that came from a client.

| Threat | Mitigation | Proof |
|---|---|---|
| A default replaces what the client asked for | Defaults go through the library's non-replacing merge | Test: a request carrying the field reaches a mock endpoint unchanged |
| A client uses a default to learn or change config | The client sees only the response; the defaults are not echoed | Existing config view needs `read_config`; test that `/v1/models` and responses do not carry defaults |
| A typo or unsupported field makes a provider reject every request | Allow-list, type and range checks at load | Tests for each refusal; `check-config` fails the deploy before it reaches the cluster |
| A default raises cost (`max_tokens`, a long thinking effort) | `max_tokens` is not allowed; the operator chose the effort | Spec scenario "Routing is unchanged"; the allow-list test |
| A default silently changes behavior for a client that expected the provider default | Documented; visible in the effective configuration | Docs task; config view test |
| **Gap:** a client that wants thinking on at an endpoint defaulted to `none` must send an effort field | Not mitigated by code: Claude Code's `thinking` object is not translated (above) | Stated in the configuration guide; the translation library is the place to fix it |

No change to authentication, a listener, secrets or the admin surface beyond one more field in an existing read-only view.

## Risks / Trade-offs

- **[Unverified fields reaching Ollama]** `reasoning_effort`, `temperature` and `top_p` were measured to take effect through
  the gateway on `friday`. `top_k`, `min_p` and `presence_penalty` through Ollama's OpenAI-style endpoint were not, and the
  model already carries `top_k 20`, `presence_penalty 1.5` as its own parameters. → The tasks include a check against
  `friday`; the guide says which fields were measured.
- **[A default hides a model's own default]** An operator who sets `temperature` stops seeing the model's. → Shown in the
  effective configuration; documented.
- **[Library behavior changes in an upgrade]** The fill-not-replace guarantee comes from `merge_extra_body`. → A test in
  this repo asserts it end to end, so an upgrade that changed it fails here.
- **[Older gateways refuse the field]** `deny_unknown_fields` rejects it. → Deploy order is stated in the migration plan.

## Migration Plan

1. Release the gateway with the field. 2. Add `request_defaults` to the `friday` endpoint in the cluster configuration and
let the file poll reload it (no restart). 3. Roll back by removing the field; an endpoint without it behaves as before.
A configuration using the field must not reach a gateway older than the release, which refuses to load it (and a running
older gateway keeps its previous configuration on a refused reload); bump the image pin before or with the config change.

## Performance

No use of the serving runtime, the ledger pool, the writer's queue or the CPU allowance beyond the library's existing merge
of at most seven keys per request. No performance task is needed (ADR 0036).

## Open Questions

- None that change what is built. Whether to later map the Anthropic `thinking` object to an effort, and whether to add
  `request_overrides`, are follow-ups outside this change.
