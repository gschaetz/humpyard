# Spec Delta

## ADDED Requirements

### Requirement: Admin statistics can price the same calls at a baseline endpoint
`GET /admin/v1/stats` SHALL accept `baseline=<provider>/<model>` and, for each group, report the cost the group's answered calls with known usage would have had at that endpoint's prices, the cost they actually had, the difference and its ratio of the baseline cost, and how many calls were priced. The baseline cost SHALL charge fresh input and cache-creation tokens at the input price, cache reads at the cached price (the input price when none is set) and output tokens at the output price.

#### Scenario: Free traffic against a paid baseline
- **WHEN** an admin asks for statistics with a baseline priced at 2 US dollars per million input tokens and the window holds 1,000,000 input tokens answered by free targets
- **THEN** the group reports a baseline cost of 2 US dollars, an actual cost of 0 and a saving of 2 US dollars, a ratio of 1

#### Scenario: Partly paid traffic
- **WHEN** some of a group's calls were paid and some free
- **THEN** the actual cost is the sum of what each cost, the saving is the baseline cost minus it, and the ratio is that saving over the baseline cost

#### Scenario: No priced calls
- **WHEN** a group has no answered calls with known usage
- **THEN** its baseline and actual costs are 0 and its ratio is null

#### Scenario: Groupings
- **WHEN** the grouping is route, target, endpoint, actor or owner
- **THEN** each call is priced once, in the group of the call that answered it, so the groups add up to the summary

### Requirement: The baseline figure states what it is
When a baseline is asked for, the answer SHALL name the baseline endpoint and its input, cached and output prices, SHALL carry a summary that adds up the groups, and SHALL state that the figures are an estimate from the tokens the calls used at the prices in the configuration, not a comparison of the quality of the answers. It SHALL report the number of answered calls left out because the provider reported no usage.

#### Scenario: The answer names its basis
- **WHEN** an admin asks for statistics with a baseline
- **THEN** the answer names the endpoint, shows its three prices and carries the statement and the summary

#### Scenario: Calls without usage
- **WHEN** three answered calls in the window reported no usage
- **THEN** they are in neither cost and the answer reports that three calls were left out

#### Scenario: Without a baseline
- **WHEN** no baseline is asked for
- **THEN** the answer is exactly what it was before this change

### Requirement: The baseline must be a priced endpoint without tiers
A baseline SHALL be an endpoint of a target in the loaded configuration that has a price, and its price SHALL have no tiers. Anything else SHALL be refused with a 400 that says whether the endpoint is unknown, has no price or has a tiered price, and nothing SHALL be read.

#### Scenario: An unknown endpoint
- **WHEN** an admin asks for a baseline that no target uses
- **THEN** the answer is a 400 naming the baseline as unknown

#### Scenario: An endpoint with no price
- **WHEN** the baseline is a configured endpoint with no price
- **THEN** the answer is a 400 saying it has no price

#### Scenario: A tiered price
- **WHEN** the baseline's price has tiers
- **THEN** the answer is a 400 saying tiered baselines are not supported, because the cost of a call then depends on its prompt size

### Requirement: The client statistics do not accept a baseline
`GET /v1/stats` SHALL refuse the `baseline` parameter with a 400, because it would show a client key the prices of endpoints it may not use.

#### Scenario: A client key asks for a baseline
- **WHEN** a client key calls `GET /v1/stats?baseline=p/m`
- **THEN** it is answered 400 as for any unknown parameter and nothing is read
