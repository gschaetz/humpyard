# Design

## Context

Keys are `[keys.<id>]` tables (`RawKey` in `src/config/schema.rs`), turned into a `KeyConfig` by `validate::key`. Everything downstream reads the resolved `KeyConfig`: `auth.rs` builds the key store from it, `budget.rs` reads its limits, `server.rs` checks `allowed_routes` and builds the limiters (`limiter.rs`) from the two rate settings, and `config_view.rs` prints it. A reload rebuilds the `Snapshot` from a freshly loaded `Config`. `over_budget` is a plain field with a default (`block`), so an unset value cannot be told from an explicit `block`. The word "profile" is already used by the request header `x-humpyard-profile` and the selector rule `when = { profile = ... }`, which are unrelated. See proposal.md for the motivation and ADR 0025 for the model.

## Goals / Non-Goals

**Goals:**
- Resolve a key against its profile once, at load, so no request-path code changes and a reload needs no new mechanism.
- Make "a key can only narrow its profile" a load-time rule, so a configuration that breaks it never serves.
- Keep the resolved shape (`KeyConfig`) the single input to the rest of the gateway, which is also what database keys (5.4) will produce.

**Non-Goals:**
- Class restriction on keys, a maximum lifetime and `max_active_keys`: they have nothing to apply to until keys have a lifecycle (5.4).
- A limit shared by the keys of a profile. Each key keeps its own counters.
- Renaming the header `x-humpyard-profile`.

## Decisions

**1. Profiles resolve at load into `KeyConfig`.** `validate::key` takes the key's raw table and its profile (if any) and returns the same `KeyConfig` as today plus the profile's name. Alternative: keep the profile on the key and look it up per request. Rejected: it spreads the merge over the request path, and the limiter, budget tracker and route check would each need to know about profiles.

**2. Inherit what is unset, refuse what is wider.** For each of the nine settings: key unset takes the profile's value; key set is checked against the profile's and refused if wider. Numbers are compared with "at most"; `allowed_routes` by subset; `over_budget` by strictness (`block` is stricter than `free_only`). A setting the profile leaves unset bounds nothing. The error names the key, the profile and the setting, as the limit errors name the key and the setting. Alternative: a key's value replaces the profile's. Rejected: a profile would stop being a ceiling, which database keys need.

**3. Inheritance counts as setting.** A key that omits a limit its profile sets gets the profile's, so omission can never widen. This is why `over_budget` on `RawKey` becomes optional: omitted inherits, and the final default stays `block`.

**4. A profile holds only the nine narrowable settings.** `read_config` and `manage_overrides` are privileges, not limits, and stay on the key; `deny_unknown_fields` on the profile table makes a profile that lists them an error. A profile cannot give a key anything.

**5. One validator for key and profile values.** A profile's numbers and routes go through the same checks as a key's (positive money, `key_limit`, existing routes), so a value that is invalid on a key is invalid on a profile, with the profile named. The key's checks run after the profile's, on the resolved value.

**6. Names.** Profile ids follow the same rules as key ids. The docs say plainly that a *key profile* is unrelated to the `x-humpyard-profile` header, which selects a route.

**7. The view.** `GET /v1/config` gains `profile` on each key and a `profiles` object with the settings as defined. The key's values stay the resolved ones, so what is printed is what is enforced.

## Threat model

Assets: the authority of a key (routes, spend, rate). Actors: the operator editing the file (including by mistake); a future UI producing patches (5.1); a future database key (5.4), which relies on the rule built here.

| # | Threat | Category | Mitigation | Proof |
|---|---|---|---|---|
| 1 | A key is given more than its profile allows, by mistake or by a generated patch | Elevation | Wider values are refused at load, naming key, profile and setting | key-profiles spec; config tests per setting |
| 2 | A profile grants a privilege (`read_config`, `manage_overrides`) | Elevation | A profile has no such settings; an unknown setting is refused | config test |
| 3 | A typo in `profile =` leaves a key with no ceiling | Elevation | An unknown profile is refused | config test |
| 4 | Omitting a setting is read as "unlimited" | Elevation | An unset setting inherits the profile's | config test; an integration test of an inherited limit |
| 5 | A reload lowers a profile under a key's own value and the gateway serves a half-valid state | Tampering | The reload fails as a whole; the previous configuration keeps serving | reload test |
| 6 | A view shows values that are not enforced | Information | The view prints the resolved `KeyConfig` | `tests/effective_config.rs` |
| 7 | Many keys of one profile multiply its allowance | Elevation | **Gap:** each key has its own counters; `max_active_keys` (5.4) bounds how many database keys exist, and a shared profile limit is out of scope | residual, documented |
| 8 | Confusion with the `x-humpyard-profile` header | Spoofing | The docs separate them; a key profile is only ever named in the file | docs |

## Risks / Trade-offs

- **Two things called "profile"** → documentation and the section name "key profiles"; the config key `profile` sits on `[keys.*]`, where it cannot be read as the header.
- **A profile change can make a reload fail** because a key now exceeds it → by design (threat 5); the failure names the key, profile and setting.
- **Resolving at load hides where a value came from** → the view shows the profile name beside the resolved value; per-value provenance (`file` vs `profile`) is left to a later change if wanted.
