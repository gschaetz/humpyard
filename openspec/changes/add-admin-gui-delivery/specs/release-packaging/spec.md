## ADDED Requirements

### Requirement: Release builds embed the admin GUI
Release binaries and the container image SHALL include the built admin GUI, so `[admin.ui] enabled = true` and `humpyard ui` work without anything else installed. A build made without the built GUI files (a plain `cargo build`, or a build with the `ui` feature off) SHALL still succeed, and the GUI path SHALL then serve a placeholder page that says how to build the GUI.

#### Scenario: A release build
- **WHEN** a release binary runs with `[admin.ui] enabled = true`
- **THEN** `/admin/ui/` serves the GUI, not the placeholder

#### Scenario: A development build
- **WHEN** the project is built with `cargo build` and no GUI files
- **THEN** the build succeeds and `/admin/ui/` serves the placeholder page when enabled
