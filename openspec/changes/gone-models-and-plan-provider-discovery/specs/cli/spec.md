## ADDED Requirements

### Requirement: Discovery on a plan provider
For a provider with `discover.plan = true`, `humpyard models sync` SHALL add a chat model only when the provider's own model list includes it and the dataset gives it a price and a context window, SHALL write the dataset's list price on the generated endpoint and the provider's class on the target, and SHALL apply the same bounds as for free providers (`max_added_per_run`, `max_removed_fraction`, `exclude`). It SHALL remove a generated plan model when the provider stops listing it. `humpyard models check` SHALL report plan models as new or gone like free ones, with the price shown. For a provider that is not a plan provider, `models sync` SHALL keep adding only free models.

#### Scenario: A new plan model
- **WHEN** a plan provider lists a chat model the dataset prices, and no target uses it
- **THEN** `models sync` writes a generated target with the dataset's price and the provider's class

#### Scenario: Only the provider lists it
- **WHEN** a plan provider lists a model that the dataset does not price
- **THEN** it is not added

#### Scenario: A provider that is not a plan
- **WHEN** a provider without `plan` lists a paid chat model
- **THEN** it is only reported by `models check --paid`, never added

#### Scenario: The bound
- **WHEN** a run would add more than `max_added_per_run` plan targets
- **THEN** nothing is written and the command exits 1
