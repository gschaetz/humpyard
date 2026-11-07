# Design

## Context

`list_models` (`src/server.rs`) emits `{id, object, created, owned_by}` for `Config::model_names()`,
filtered by `KeyRecord::may_use`. The requested model string is read in `decode` and flows to
`select_route`, which feeds it to selector rules (`Facts.model`) and `authorize`. Routes are a tagged
enum (`RouteSpec`, many variants) and targets are a separate map, so a "route-level" field would have to
be repeated on every variant and could not cover a target. See proposal.md for motivation.

In-force ADRs read: 0003 (config-first keys), 0008 (routing policy seam), 0011 (selectors choose the
route), 0012 (config snapshot reloaded), 0017 to 0019. The fixed request order (decode, authorize, budget,
policy, run, encode; invariants item 5) must not change.

## Goals / Non-Goals

**Goals:**
- A client can request, and `/v1/models` can advertise, a name with `claude` in it for any pool.
- The alias never reaches authorization, selectors, policy, the ledger or the decision log.

**Non-Goals:**
- Aliases for anything but a route or target (no alias chains, no per-key alias sets).
- Anthropic-shaped model objects (`type`, `created_at`); the OpenAI-shaped list stays.
- Auto-generating `claude-` aliases.
- Recording which alias a client used.

## Decisions

**1. A top-level `[aliases.<id>]` table, not fields on each route.** `RouteSpec` has many variants and
targets are a separate map; per-route fields would be repeated across variants and could not alias a
target. One table with `model = "<name>"` covers both, validates in one place, and lets one route have
several aliases. Alternative: `aliases = [...]` on each route (what the request first sketched);
rejected for the variant sprawl and because ownership of the uniqueness rule is clearer in one table.

**2. Resolve once, at decode.** The requested model is mapped to the real name immediately after it is
read from the body and before `select_route`. Nothing downstream needs to know aliases exist, which keeps
the stage order intact (invariants item 5) and means policy, selectors and the ledger need no changes.
Alternative: resolve inside `authorize` and `select_route`; rejected, it would spread alias awareness
over every consumer of the model string.

**3. Allowlists name real names only.** `allowed_routes` is validated against routes and targets as
today, so naming an alias is a load error. `list_models` shows an alias when `may_use(target)` holds.
Alternative: let `allowed_routes` accept aliases; rejected, two names for one grant makes "which keys
can reach this pool" harder to read and audit, and the alias set is a presentation concern.

**4. Selector rules match the resolved name.** `[[select]]` rules with `model = "..."` see the real name
whichever name the client sent, so a rule written for a route keeps working for its aliases.

**5. A `model_aliases` switch, default on.** One top-level boolean turns resolution and listing off as an
operator kill switch (for example if a client mishandles the extra names) without deleting the table. The
table is still validated while off, so flipping it back on at a reload cannot fail. With no aliases
configured the default does nothing. Alternative: no switch (remove the table to disable); rejected, it
loses the labels and hides the intent.

**6. Labels are plain bounded strings.** `display_name` (64) and `description` (256), no control
characters. They are shown to clients only through `/v1/models` and are config-authored, so no escaping
beyond JSON encoding is needed.

## Threat model

Assets: the key allowlist (which pools a key may reach), the integrity of the decision log and ledger
(what is recorded as served). Actors: a client holding a valid key, an unauthenticated caller on an open
gateway, an operator editing config. Trust boundary: the client-supplied `model` string crossing into
authorization and routing.

| Threat | Mitigation | Proof |
|---|---|---|
| A key reaches a pool its allowlist forbids by requesting an alias | Alias resolved before `authorize`; the check runs on the real name | test: key without `coding-paid` is refused for `claude-coding` with the real name in the error |
| `/v1/models` reveals pools a key may not use via their aliases | Aliases filtered with `may_use(target)` | test: hidden alias absent for a restricted key |
| An alias shadows a real name and redirects traffic | Load rejects any collision with a route, target or alias | config tests for each collision kind |
| A chain or cycle of aliases loops resolution | `model` must name a route or target, never an alias | config test: alias pointing at an alias is rejected |
| Oversized or hostile label text reaches clients | Length and control-character limits at load | config test for each limit |
| A reload swaps the alias set mid-request | Resolution reads the request's own config snapshot (ADR 0012) | existing snapshot behavior; reload scenario test |
| Ledger or decision log records an alias, making per-pool statistics split | Only the real name leaves decode | test: ledger row and decision record hold the real name |
| Aliases stay live after an operator wants them off | `model_aliases = false` stops resolution and listing at the next reload | test: alias request is not found and absent from the list when off |

No gaps: no new listener, secret or admin surface.

## Risks / Trade-offs

- Claude Code's discovery filter is its own behavior and can change → the aliases are ordinary names and
  harmless if it does; docs/clients.md records the version it was verified on and states that the
  end-to-end picker check was done by hand.
- Statistics never show which alias a client used → accepted (non-goal); the real pool is what matters.
- One more name per pool in `/v1/models` → clients that list every model see duplicates; a client can
  tell them apart by the labels.
