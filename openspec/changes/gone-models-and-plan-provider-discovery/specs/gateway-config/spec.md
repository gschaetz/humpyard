## MODIFIED Requirements

### Requirement: Discovery opt-in and settings
A provider MAY set `discover = { strengths = [...], confirm = true, plan = true, class = "..." }` (requires `price_source`), and the configuration MAY have `[discovery]` with `file`, `class` (default `public`), `max_added_per_run` (default 10), `max_removed_fraction` (default 0.5) and `exclude` (a list of `provider/model` ids). `plan` (default false) marks a provider on a flat-fee plan; `class` overrides `[discovery] class` for that provider's generated targets. Setting `plan` or `class` SHALL NOT change the behavior of providers that do not set them.

#### Scenario: Opting in without a source
- **WHEN** a provider sets `discover` but not `price_source`
- **THEN** the configuration is rejected naming the provider

#### Scenario: A plan provider with its own class
- **WHEN** a provider sets `discover = { strengths = ["general"], plan = true, class = "plan" }`
- **THEN** its generated targets carry the class `plan` and other providers' generated targets keep `[discovery] class`

#### Scenario: A class on a provider that is not a plan
- **WHEN** a provider sets `class` without `plan`
- **THEN** its generated targets carry that class and are still priced at zero

### Requirement: The generated file is merged, strictly, and fails safe
When `[discovery] file` names a file, `serve` SHALL parse it at load and on every reload and merge its targets, accepting it only if it contains nothing but `[targets.auto-*]` on providers that exist and have `discover` set, no more than 500 targets, no (provider, model) pair that a hand-written target already uses, and every endpoint priced at zero, except that an endpoint of a provider with `discover.plan = true` MAY carry a price whose numbers are finite, non-negative and within the bounds applied to hand-written prices. A file that does not meet this, or is missing or empty, SHALL be ignored with a log line and a gauge, and the gateway SHALL serve with the main configuration alone. `serve` SHALL NOT fetch any data to do this.

#### Scenario: A valid generated file
- **WHEN** the file holds `auto-groq-x` with the strength `general`
- **THEN** a pool of `general` includes it after the next reload

#### Scenario: An invalid generated file
- **WHEN** the file contains a table other than a target, or a priced endpoint on a provider that is not a plan provider
- **THEN** it is ignored in full and the gateway serves with the main configuration

#### Scenario: A priced endpoint on a plan provider
- **WHEN** the file holds a target on a plan provider with a finite non-negative price
- **THEN** the file is accepted and the target is priced at that rate

#### Scenario: A nonsensical price on a plan provider
- **WHEN** a plan provider's generated endpoint has a negative, infinite or non-numeric price
- **THEN** the file is ignored in full

#### Scenario: Reload sees the file
- **WHEN** only the generated file changes
- **THEN** the gateway reloads
