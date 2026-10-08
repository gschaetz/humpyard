# Spec Delta

## Purpose

Defines how the gateway process shuts down: draining in-flight work, bounding the wait, and
making sure usage is recorded before it exits.

## ADDED Requirements

### Requirement: Drain on termination signals
On SIGINT or SIGTERM the gateway SHALL stop accepting new connections and SHALL let requests already in flight run to completion.

#### Scenario: In-flight request finishes
- **WHEN** a request is being served and the gateway receives SIGTERM
- **THEN** the request completes normally and the client receives its full response

#### Scenario: New connections refused
- **WHEN** the gateway has begun shutting down
- **THEN** new connection attempts are refused

### Requirement: Bounded grace period
The wait for in-flight requests SHALL end after `shutdown_grace_secs` (default 30), after which the gateway SHALL cancel the remaining requests; streams SHALL end with an error event.

#### Scenario: A stream outlives the grace period
- **WHEN** a streamed response is still running when the grace period ends
- **THEN** the client's stream ends with an error event and the gateway proceeds to exit

### Requirement: A second signal stops waiting
A second termination signal received while draining SHALL end the wait at once and cancel the remaining requests.

#### Scenario: Impatient operator
- **WHEN** the gateway is draining and receives another SIGINT
- **THEN** remaining requests are cancelled immediately rather than after the grace period

### Requirement: Ledger flushed before exit
Before the process exits, the gateway SHALL write every queued usage entry to the ledger, waiting at most 10 seconds, and SHALL exit with status 0 only if it succeeded.

#### Scenario: Clean exit
- **WHEN** the gateway exits after draining
- **THEN** the ledger already contains the entries of all requests that finished or were cancelled

#### Scenario: Ledger cannot be flushed
- **WHEN** the queued entries cannot be written within the wait
- **THEN** the gateway logs that usage entries may be lost and exits with a non-zero status
