## ADDED Requirements

### Requirement: The admin GUI setting
The configuration MAY have `[admin.ui]` with `enabled` (default false). It SHALL accept no other key. It SHALL be rejected when `enabled` is true and `[admin]` defines no credential. A change to `[admin.ui] enabled` SHALL take effect on configuration reload, without a restart.

#### Scenario: Unknown key
- **WHEN** `[admin.ui]` contains a key other than `enabled`
- **THEN** the configuration is rejected naming the key

#### Scenario: Turned on by a reload
- **WHEN** a running gateway's configuration is reloaded with `[admin.ui] enabled = true`
- **THEN** `/admin/ui/` is served from then on, without a restart

#### Scenario: Turned off by a reload
- **WHEN** a running gateway's configuration is reloaded with the GUI disabled
- **THEN** `/admin/ui/` answers 404 from then on
