## ADDED Requirements

### Requirement: Key limits
A key MAY set `max_requests_per_minute` and `max_concurrent`, each a positive integer. `max_requests_per_minute` SHALL NOT exceed 100000. A zero, negative or non-integer value, or a value above the maximum, SHALL be rejected with a message naming the key and the setting. The effective configuration SHALL show a key's limits.

#### Scenario: A valid limit
- **WHEN** a key sets `max_requests_per_minute = 60` and `max_concurrent = 4`
- **THEN** the configuration loads and the effective configuration shows both

#### Scenario: A zero limit
- **WHEN** a key sets `max_concurrent = 0`
- **THEN** the configuration is rejected naming the key and `max_concurrent`

#### Scenario: Too large a window
- **WHEN** a key sets `max_requests_per_minute = 1000000`
- **THEN** the configuration is rejected naming the key and `max_requests_per_minute`
