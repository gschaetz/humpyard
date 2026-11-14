# Spec Delta

## ADDED Requirements

### Requirement: Admin statistics can be grouped by actor and by owner
`GET /admin/v1/stats` and `GET /admin/v1/stats/series` SHALL accept `group_by=actor` and `group_by=owner`, counting each group by the same rules as the other groupings. Calls whose decision records no actor (or no owner) SHALL be counted in one group named `(none)`.

#### Scenario: Usage per agent
- **WHEN** an admin asks for `group_by=actor` over a window in which keys of two agents were used
- **THEN** there is one group per agent, with the requests, outcomes and cost of all keys that named it

#### Scenario: Usage per person
- **WHEN** an admin asks for `group_by=owner` and two agents are owned by `gary`
- **THEN** one group `gary` holds the sum of both agents' calls, and the calls of keys of the human `gary` himself

#### Scenario: Calls without an actor
- **WHEN** some calls in the window used a key that names no actor
- **THEN** they appear in the group `(none)` and are not dropped from the totals

#### Scenario: A series by actor
- **WHEN** an admin asks for the series with `group_by=actor`
- **THEN** each bucket is counted by the same function as the totals and the top groups follow the usual limit

### Requirement: The client statistics API does not group by actor
`GET /v1/stats` SHALL NOT accept `group_by=actor` or `group_by=owner`; a client key sees only its own calls, so the grouping could add nothing.

#### Scenario: A client key asks to group by actor
- **WHEN** a client key calls `GET /v1/stats?group_by=actor`
- **THEN** it is answered 400 with the same error as for any unknown `group_by`
