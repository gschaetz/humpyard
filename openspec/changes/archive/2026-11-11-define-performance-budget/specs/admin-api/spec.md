## ADDED Requirements

### Requirement: Heavy admin reads are limited to one at a time and share results
The admin API SHALL run at most one heavy read at a time (statistics, session views and time-bucketed statistics), SHALL answer another heavy read that arrives meanwhile with 503 `admin_busy` and a `Retry-After` header and SHALL NOT queue it, and SHALL answer an identical read that arrives within ten seconds of an earlier one from that earlier result.

#### Scenario: A second heavy read while one runs
- **WHEN** a statistics read is running and a session view is requested
- **THEN** the second answer is 503 `admin_busy` with `Retry-After`, and the first read is unaffected

#### Scenario: The same read twice
- **WHEN** the same statistics read is made twice within ten seconds
- **THEN** the ledger is read once and both answers are identical

#### Scenario: The same read at the same moment
- **WHEN** five identical statistics reads arrive while the first is still being computed
- **THEN** all five are answered from that one computation, none is answered 503, and the ledger is read once

#### Scenario: A different read is not served another's result
- **WHEN** two statistics reads differ in `window`, `group_by`, a filter or the `key` filter
- **THEN** each is computed for its own parameters

#### Scenario: A read that cannot get a connection
- **WHEN** a heavy or light ledger read cannot get a read connection or lock within the ledger's read limits
- **THEN** the answer is 503 `admin_busy` with `Retry-After`, not a hung request and not a 500

#### Scenario: Light reads are not gated
- **WHEN** a heavy read is running
- **THEN** `GET /admin/v1/health`, `/config`, `/whoami` and the override endpoints are answered normally

### Requirement: Shedding happens after the scope is checked
A heavy admin read SHALL be authenticated and checked for its scope before it can occupy the gate or be answered from a shared result, so a caller without the scope receives the scope error and cannot hold the gate.

#### Scenario: No scope
- **WHEN** a credential without `read` asks for statistics while the gate is free
- **THEN** the answer is 403 `admin_scope_required` and the gate stays free
