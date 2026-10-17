# Spec Delta

## ADDED Requirements

### Requirement: Rename providers and set a quality weight in the migration
`humpyard migrate-modelrelay` SHALL accept `--rename FROM=TO` (repeatable) to name a ModelRelay provider differently in the fragment, applying the new name to the provider table, its `api_key_env` (`<TO>_API_KEY`) and its targets' name prefix, and `--quality-weight W` (0 to 1) to write `quality_weight = W` on every generated `fastest` route. A rename whose `TO` is not lowercase letters, digits and dashes, or repeats another generated provider's name, and a weight outside 0 to 1, SHALL be refused before anything is printed.

#### Scenario: Renaming a provider
- **WHEN** the command is run with `--rename opencode=opencode-zen`
- **THEN** the fragment has `[providers.opencode-zen]` with `api_key_env = "OPENCODE_ZEN_API_KEY"`, its targets are named `opencode-zen-...` and use that provider, and no `opencode` provider appears

#### Scenario: A colliding rename
- **WHEN** two providers are renamed to the same name, or one is renamed to another provider's name
- **THEN** the command fails naming the conflict and prints no fragment

#### Scenario: A quality weight
- **WHEN** the command is run with `--quality-weight 0.5`
- **THEN** every generated `fastest` route has `quality_weight = 0.5` and the fragment loads
