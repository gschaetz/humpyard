# Spec Delta

## ADDED Requirements

### Requirement: Keys, ledger and prices in configuration
The configuration SHALL allow `[keys.<id>]` entries (hash, optional route allowlist, budgets, `over_budget`), a `[ledger]` section naming the SQLite file, a `[budget]` section with the restricted fraction and price ceiling, and an optional `price` (input, output, optional cached input, USD per million tokens) on each endpoint.

#### Scenario: Valid budget config
- **WHEN** the file declares a key with `daily_usd`, a ledger path and priced endpoints
- **THEN** it loads

#### Scenario: Budget without ledger
- **WHEN** a key declares a budget but no ledger path is configured
- **THEN** startup fails explaining that budgets need a ledger

#### Scenario: Invalid limits
- **WHEN** a limit is negative or `restricted_at` is outside 0 to 1
- **THEN** startup fails naming the field
