# Tasks

## 1. Config and pool

- [x] 1.1 Add `headers` to the provider config with validation (valid HTTP names/values, no authentication headers); unit tests cover the three gateway-config scenarios
- [x] 1.2 Send configured headers on every call for that provider; integration tests assert the header arrives and is not sent to a different provider after failover
- [x] 1.3 Document `headers` in README and docs/routing.md and show it in examples/config.toml; verify `check-config` accepts the example
