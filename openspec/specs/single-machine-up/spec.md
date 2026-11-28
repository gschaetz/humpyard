# single-machine-up Specification

## Purpose

Lets one person on one machine go from an installed binary to a running gateway with its GUI open, and keeps the few provider keys they choose to save in one small, write-only, local file.

## Requirements

### Requirement: up starts the gateway and the GUI together
`humpyard up` SHALL start the gateway as a child process and serve the admin GUI itself, both listening on loopback only, and SHALL open the GUI in the default browser already signed in. The process that handles model requests SHALL serve no page. When the child exits, `up` SHALL say why and exit non-zero, and when `up` is stopped it SHALL stop the child.

#### Scenario: One command to a signed-in page
- **WHEN** `humpyard up` is run with a usable configuration
- **THEN** the gateway and the GUI are listening on loopback, the browser opens the GUI with no credential to paste, and the terminal prints the addresses

#### Scenario: The gateway serves no page
- **WHEN** a request for the GUI's page is made to the gateway's own address
- **THEN** the gateway does not serve it

#### Scenario: A taken address is refused
- **WHEN** the gateway's or the GUI's address is already in use
- **THEN** `up` starts nothing, names the address and the flag that changes it, and exits non-zero

#### Scenario: Stopping up stops the gateway
- **WHEN** `up` receives an interrupt
- **THEN** the gateway child is stopped before `up` exits

### Requirement: up writes a first configuration when there is none
When no configuration exists in the state directory, `up` SHALL run the `init` flow for the providers whose keys it finds (from the environment or the key file), check their models live, write a loopback-only configuration, and prove it with one request before serving. It SHALL NOT overwrite an existing configuration, and SHALL NOT start with a configuration it could not prove.

#### Scenario: First run
- **WHEN** `up` is run with no configuration and one provider key in the environment
- **THEN** a configuration is written to the state directory and the gateway starts with it

#### Scenario: No key anywhere
- **WHEN** `up` is run with no configuration and no provider key in the environment or the key file
- **THEN** `up` serves the GUI with a setup state that asks for a key, and the gateway is not started until one is saved

#### Scenario: An existing configuration is kept
- **WHEN** a configuration already exists in the state directory
- **THEN** `up` uses it unchanged and writes none

### Requirement: The run's admin credential is made by up and never stored
`up` SHALL create a new admin credential for each run, give the gateway only its hash through a generated file in a `0700` run directory, give the GUI the credential itself, and remove the run directory on exit. The credential SHALL NOT be printed, logged or written outside that run directory's hash. A configuration that already defines an `[admin]` section SHALL be refused with a message saying what to do, not edited.

#### Scenario: A fresh credential each run
- **WHEN** `up` is started twice
- **THEN** the second run's credential differs from the first and the first no longer works

#### Scenario: The credential is not in any output
- **WHEN** `up` runs and exits
- **THEN** its output and the gateway's logs contain neither the credential nor a hash of it, and the run directory is gone

#### Scenario: Existing admin section
- **WHEN** the user's configuration has an `[admin]` section
- **THEN** `up` refuses to start, names the section, and explains how to run the gateway and GUI separately

### Requirement: Saved keys live in one small private file
When the user chooses to save a provider key, `up` SHALL write it to one file in the state directory, created with mode `0600` inside a `0700` directory and replaced atomically, and SHALL write it nowhere else. Nothing SHALL be saved unless the user chose to save it.

#### Scenario: The file is private
- **WHEN** a key is saved
- **THEN** the file's mode is `0600`, its directory's mode is `0700`, and no other file holds the key

#### Scenario: Nothing is saved by default
- **WHEN** a key is entered without choosing to save it
- **THEN** it is used for this run only and no file is written

### Requirement: Only up reads the key file, and the environment wins
Only `humpyard up` SHALL read the key file, and it SHALL pass its values to the gateway child as the environment variables the configuration names in `api_key_env`. `serve`, `ui` and every other command SHALL NOT read it. A variable set in the real environment SHALL win over the file.

#### Scenario: The environment wins
- **WHEN** a provider's variable is set in the environment and the file also holds a key for it
- **THEN** the gateway child receives the environment's value, and the GUI says the key comes from the environment

#### Scenario: serve does not read the file
- **WHEN** `humpyard serve` is run against a configuration naming a variable that only the key file holds
- **THEN** it fails on the missing variable as before

### Requirement: Keys entered in the page are write-only
The local surface SHALL accept a provider key to add, replace or remove, SHALL check an added key live (the provider lists models and answers one small request) before saving it, and SHALL NOT return, echo, log or display a key at any time after it is sent. A status call SHALL say only, for each configured provider, whether a key is present and whether it comes from the environment, the file, or was entered for this run only.

#### Scenario: A key is never returned
- **WHEN** a key is saved and the status is read
- **THEN** the status says the provider has a key from the file and no response, log line or page text contains the key

#### Scenario: A bad key is not saved
- **WHEN** a key the provider rejects is entered
- **THEN** it is not saved, the provider's refusal is shown in words, and the gateway is not restarted

#### Scenario: Replace and remove
- **WHEN** a key is replaced or removed
- **THEN** the file reflects it, the gateway child restarts with the new environment, and requests resume

### Requirement: The local surface is loopback-only and as strict as the GUI
The routes `up` adds for the page (key status and change, connect settings, test request) SHALL answer only on a loopback address to a request that carries the run's session, whose `Host` is loopback and that is not cross-site, with the same restrictive headers as the GUI. If `up` is told to listen on a non-loopback address, key entry SHALL be disabled and the surface SHALL say so.

#### Scenario: Another site cannot save a key
- **WHEN** a page from another origin posts a key to the local surface
- **THEN** the request is refused and nothing is written

#### Scenario: No session, no change
- **WHEN** a request without the run's session reaches a key-changing route
- **THEN** it is refused and nothing is written

#### Scenario: A rebound host name is refused
- **WHEN** a request arrives with a `Host` that is not loopback
- **THEN** it is refused

### Requirement: A test request proves the path end to end
The local surface SHALL offer one request that sends a small fixed prompt to the gateway on its loopback address and returns which target answered, the latency and the outcome, with no key and no prompt content beyond the fixed one.

#### Scenario: A working gateway
- **WHEN** the test request is made and a provider answers
- **THEN** the result names the target that answered and its latency

#### Scenario: A failing gateway
- **WHEN** no provider can answer
- **THEN** the result says which step failed in words and exposes no key
