# Spec Delta

## ADDED Requirements

### Requirement: The query bar can compare the traffic with a baseline endpoint
The query bar SHALL offer `Compare with`: none, which is the start, or one of the priced endpoints without tiers named in the effective configuration, as `provider/model`, and SHALL read the statistics again when the choice changes.

#### Scenario: Choosing a baseline
- **WHEN** the person chooses an endpoint under Compare with
- **THEN** the statistics are read again with that baseline

#### Scenario: Endpoints without a usable price
- **WHEN** an endpoint has no price, or a tiered price
- **THEN** it is not offered

### Requirement: The baseline choice is kept in the fragment and treated as untrusted
The choice SHALL be kept in the URL fragment as `baseline` and SHALL be read only when it is exactly one of the offered endpoints, and otherwise as none.

#### Scenario: A shared link
- **WHEN** a link holding an offered baseline is opened
- **THEN** the query bar and the figure show that baseline once the person has signed in

#### Scenario: A hostile or unknown value
- **WHEN** the fragment holds `baseline=<script>alert(1)</script>` or an endpoint the configuration does not offer
- **THEN** the view uses no baseline and creates no element from the fragment

### Requirement: Nothing to compare with shows no choice
When the configuration cannot be read, or has no priced endpoint without tiers, the view SHALL NOT show the choice and SHALL work as it does without it.

#### Scenario: Nothing to offer
- **WHEN** the configuration has no priced, untiered endpoint, or it cannot be read
- **THEN** there is no Compare with choice and nothing else changes

### Requirement: The view says what the traffic would have cost
With a baseline chosen, the view SHALL show above the chart, in words, the baseline endpoint and its prices, what the traffic would have cost there, what it cost, and the difference with its share of the baseline cost, and SHALL show nothing of the kind without a baseline. Every string in it SHALL be rendered as text.

#### Scenario: A saving
- **WHEN** the answer's summary says 0.018 US dollars at the baseline and 0.012 actually
- **THEN** the strip says the traffic would have cost about $0.018 at that endpoint, cost about $0.012, and was $0.006 less, 31%

#### Scenario: Nothing priced
- **WHEN** the summary has no priced calls
- **THEN** the strip says there were no calls with known usage in the window and shows no share

#### Scenario: Hostile text
- **WHEN** the endpoint name or the statement holds markup
- **THEN** it is shown as text and creates no element

### Requirement: The figure says what it is
The strip SHALL show the number of calls left out and the statement that the figures are an estimate, and SHALL say the traffic cost more, with the amount, when the baseline would have been cheaper.

#### Scenario: The baseline would have been cheaper
- **WHEN** the actual cost is above the baseline cost
- **THEN** the strip says the traffic cost more than it would have at the baseline, with the amount

#### Scenario: Calls left out
- **WHEN** some answered calls reported no usage
- **THEN** the strip says how many were left out of both costs

### Requirement: A gateway that does not know baseline is not an error
When the gateway refuses `baseline` as an unknown parameter, the view SHALL show a notice that the gateway is older than the page, ignore the baseline, and show the table and chart as before.

#### Scenario: An older gateway
- **WHEN** the gateway answers 400 for the `baseline` parameter
- **THEN** a notice says so and the table and chart are shown without the strip
