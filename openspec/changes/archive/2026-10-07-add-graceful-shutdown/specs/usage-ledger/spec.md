# Spec Delta

## ADDED Requirements

### Requirement: Cancelled requests still record their spend
Usage already incurred by a request SHALL be recorded even if the request is cancelled before a response is returned, whether by the client disconnecting or by shutdown.

#### Scenario: Client disconnects during a classifier route
- **WHEN** a client disconnects after a route's judge call has returned but before the answer completes
- **THEN** the judge call's usage is recorded and charged to the key

### Requirement: Entries survive a graceful exit
The gateway SHALL write all queued entries before exiting after a termination signal.

#### Scenario: Stop right after a request
- **WHEN** a request finishes and the gateway receives SIGTERM before the ledger writer has run
- **THEN** the entry is in the ledger when the process has exited
