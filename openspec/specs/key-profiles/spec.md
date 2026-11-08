# key-profiles Specification

## Purpose
Lets an operator define named bundles of key authority once and apply them to many keys, with the guarantee that a key can only narrow what its profile allows, never widen it.

## Requirements

### Requirement: Profiles are defined in the configuration
The configuration MAY define `[profiles.<id>]` tables. A profile MAY set `allowed_routes`, `over_budget`, `daily_usd`, `monthly_usd`, `daily_tokens`, `monthly_tokens`, `max_request_usd`, `max_requests_per_minute` and `max_concurrent`, each with the meaning and the validation it has on a key. Any other setting in a profile SHALL be refused, naming the profile and the setting.

#### Scenario: A profile with budgets and a limit
- **WHEN** a profile sets `daily_usd = 5` and `max_requests_per_minute = 60`
- **THEN** the configuration loads

#### Scenario: A profile cannot grant a privilege
- **WHEN** a profile sets `read_config = true`
- **THEN** the configuration is rejected naming the profile and `read_config`

#### Scenario: An invalid value is refused as on a key
- **WHEN** a profile sets `max_concurrent = 0`
- **THEN** the configuration is rejected naming the profile and `max_concurrent`

### Requirement: A key inherits the settings of its profile
A key MAY set `profile = "<id>"`. For each profile setting the key leaves unset, the key SHALL have the profile's value. A key without a profile SHALL behave exactly as before.

#### Scenario: Inherited budget and limit
- **WHEN** a profile sets `daily_usd = 5` and `max_concurrent = 4` and a key names it and sets neither
- **THEN** the key is limited to 5 US dollars a day and 4 requests in flight

#### Scenario: No profile
- **WHEN** a key names no profile
- **THEN** its settings are exactly the ones it sets itself

### Requirement: A key's numbers can only narrow its profile's
For `daily_usd`, `monthly_usd`, `daily_tokens`, `monthly_tokens`, `max_request_usd`, `max_requests_per_minute` and `max_concurrent`, a value a key sets itself SHALL be at most the profile's. A setting a profile leaves unset places no bound on the key.

#### Scenario: A lower budget is allowed
- **WHEN** a profile sets `daily_usd = 5` and a key naming it sets `daily_usd = 2`
- **THEN** the key is limited to 2 US dollars a day

#### Scenario: A wider budget is refused
- **WHEN** a profile sets `daily_usd = 5` and a key naming it sets `daily_usd = 10`
- **THEN** the configuration is rejected naming the key, the profile and `daily_usd`

#### Scenario: A limit is not narrowed by omission
- **WHEN** a profile sets `max_requests_per_minute = 60` and a key naming it sets no rate
- **THEN** the key is limited to 60 requests a minute, not unlimited

#### Scenario: A profile with no bound
- **WHEN** a profile sets no `daily_usd` and a key naming it sets `daily_usd = 10`
- **THEN** the key is limited to 10 US dollars a day

### Requirement: A key's routes and over-budget mode can only narrow its profile's
Every route a key lists in `allowed_routes` SHALL be in the profile's list. For `over_budget`, a key MAY set `block` under a profile of `free_only` and SHALL NOT set `free_only` under a profile of `block`.

#### Scenario: A subset of routes
- **WHEN** a profile allows routes `fast` and `deep` and a key naming it lists only `fast`
- **THEN** the key may use `fast` only

#### Scenario: A route outside the profile
- **WHEN** a profile allows routes `fast` and `deep` and a key naming it lists `fast` and `other`
- **THEN** the configuration is rejected naming the key, the profile and `allowed_routes`

#### Scenario: Over-budget mode only gets stricter
- **WHEN** a profile sets `over_budget = "block"` and a key naming it sets `over_budget = "free_only"`
- **THEN** the configuration is rejected naming the key, the profile and `over_budget`

### Requirement: A profile must exist
A key that names a profile that is not defined SHALL be refused at load, naming the key and the profile. A profile that no key names SHALL be allowed.

#### Scenario: Unknown profile
- **WHEN** a key sets `profile = "trial"` and no `[profiles.trial]` exists
- **THEN** the configuration is rejected naming the key and `trial`

### Requirement: Profiles follow a reload
A reload SHALL apply a changed profile to every key that names it, from the next request, with the same refusals as at start. A configuration whose reload is refused SHALL leave the previous profiles and keys in force.

#### Scenario: A profile is raised
- **WHEN** a reload raises a profile's `max_requests_per_minute` and a key inherits it
- **THEN** the key's next request is admitted under the new limit

#### Scenario: A reload that would widen a key past its profile
- **WHEN** a reload lowers a profile's `daily_usd` below a value a key sets itself
- **THEN** the reload is refused and the previous configuration keeps serving
