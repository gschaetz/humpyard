## ADDED Requirements

### Requirement: Release builds embed the admin GUI and fail without it
Release binaries and the container image SHALL be built with the `ui` feature and SHALL include the built admin GUI, so `humpyard ui` works without anything else installed. A build with the `ui` feature on and no built GUI files SHALL fail. A build with the feature off (a plain `cargo build`) SHALL succeed, and `humpyard ui` SHALL then serve a placeholder page that says how to build the GUI.

#### Scenario: A release build
- **WHEN** a release binary runs `humpyard ui`
- **THEN** the GUI's page is served, not the placeholder

#### Scenario: A release build without the GUI files
- **WHEN** a build with the `ui` feature is made and the built GUI files are missing
- **THEN** the build fails and says why

#### Scenario: A development build
- **WHEN** the project is built with `cargo build` and no GUI files
- **THEN** the build succeeds and `humpyard ui` serves the placeholder page
