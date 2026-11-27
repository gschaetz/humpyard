# Spec Delta

## MODIFIED Requirements

### Requirement: The GUI talks only to the admin API of its own origin and keeps its credential for the session
The GUI SHALL send requests only to `/admin/v1` on the origin it was loaded from, and, when `humpyard ui` holds the admin credential and asks for its access token, one request to `/ui/session` on that origin with the token in the body (never in a URL). When it is served by `humpyard up`, it MAY also send requests to `/local` on that origin, and only for the first-run screens; it finds out whether it is served by `up` with one probe of `/local/connect`. When the user supplies a credential, the GUI SHALL keep it for the browser session only (not in `localStorage`, a cookie or any storage that outlives the session), and SHALL hold no configuration or data of its own beyond what it displays.

#### Scenario: The access token
- **WHEN** the service answers a data request with `ui_token_required`
- **THEN** the GUI asks for the token, sends it once in a request body to `/ui/session`, does not store it, and asks the data request again

#### Scenario: Credential lifetime
- **WHEN** the browser session ends
- **THEN** nothing the GUI stored lets a new session act as the user

#### Scenario: Served by humpyard ui
- **WHEN** the GUI is served by `humpyard ui`
- **THEN** its one probe of `/local/connect` is answered as not found, it sends no other request to `/local`, and it shows none of the first-run screens

## ADDED Requirements

### Requirement: A first-run home helps a new user connect a tool
When served by `humpyard up`, the GUI SHALL open on a home screen with a *Connect a tool* panel offering copy-paste settings for each client the documentation covers, filled with the gateway's live loopback address and no secret, and the route names the configuration offers. The text SHALL be rendered as text and the copy action SHALL copy exactly what is shown.

#### Scenario: Settings for Claude Code
- **WHEN** the user opens the panel for Claude Code
- **THEN** it shows the base-URL and model settings with the live address and a placeholder in place of a key

#### Scenario: Copy is exact
- **WHEN** the user presses copy
- **THEN** the clipboard holds the text shown, and nothing else

### Requirement: An empty gateway says what to do and can prove itself
When the gateway has answered no request, the GUI SHALL show an empty state that says so in words, names the next step and offers *Send a test request*, and SHALL show the result of the test (the target that answered and its latency, or the failing step) beside it. The query view and the savings strip SHALL appear, with the strip first, as soon as there is at least one answered call.

#### Scenario: Before any traffic
- **WHEN** the gateway has answered no request
- **THEN** the empty state and the test button are shown instead of an empty chart

#### Scenario: After the first call
- **WHEN** one request has been answered
- **THEN** the query view is shown, with the savings strip above the chart when a comparison is possible

### Requirement: Provider keys are entered in a write-only form
The GUI SHALL offer, for each configured provider, a form to add, replace or remove a key, SHALL offer *use for this run only* and *save on this machine* as separate choices with the second not preselected, SHALL clear the field as soon as the key is sent, and SHALL NOT display, store or log a key. It SHALL show only whether a provider has a key and where it comes from. The form SHALL be absent when key entry is disabled.

#### Scenario: Not saved by default
- **WHEN** the form is opened
- **THEN** *use for this run only* is chosen and *save on this machine* is not

#### Scenario: The field is cleared
- **WHEN** a key is sent
- **THEN** the field is empty at once, whatever the answer, and no page text shows the key

#### Scenario: Key entry disabled
- **WHEN** the local surface says key entry is disabled
- **THEN** no key form is shown and the page says why
