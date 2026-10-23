# Spec Delta

## ADDED Requirements

### Requirement: Decisions and explain show the overrides that applied
Each decision SHALL list the overrides that applied to it (id, kind and subject), and `explain` SHALL show a target removed by an override with the override's id as the reason. The effective configuration view SHALL label a target disabled by an active override with the source `override`.

#### Scenario: An override is visible
- **WHEN** a request is planned while a target is disabled
- **THEN** its decision lists the override and `explain` names it as the reason the target was removed
