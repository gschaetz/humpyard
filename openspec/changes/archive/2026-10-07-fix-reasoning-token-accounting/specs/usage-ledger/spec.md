# Spec Delta

## ADDED Requirements

### Requirement: Reasoning tokens are part of output
The gateway SHALL treat reasoning tokens reported by a provider as part of that call's output tokens, record them for information, and SHALL NOT add them again to cost or to token budgets.

#### Scenario: Reasoning model call
- **WHEN** a provider reports 100 completion tokens of which 90 are reasoning tokens, and 10 prompt tokens
- **THEN** the call is costed on 10 input and 100 output tokens, counts 110 tokens against token budgets, and the ledger entry records 90 reasoning tokens as detail

#### Scenario: Totals after a restart
- **WHEN** spend is rebuilt from the ledger after a restart
- **THEN** reasoning tokens are not added to the rebuilt token totals
