# Design

## Context

The query view (archived `add-gui-query-view`, ADR 0035) draws its times in UTC on purpose, so that a point's text is the
same wherever the page is read (`chart-data.ts`, the axis note in `chart.tsx`). The query lives in the URL fragment and is
parsed against fixed lists (`query.ts`). The bars are drawn in the table's order, from the same rows. See proposal.md for
the motivation.

## Goals / Non-Goals

**Goals:**
- Times in the operator's own zone by default, with UTC one click away and shareable.
- A ranking that is ordered by what it shows.

**Non-Goals:**
- Any change to the API, the buckets or the data read.
- A zone picker beyond the browser's zone and UTC, or a stored per-user preference.
- Changing the table's order, or the noise of the chart's rate lines on thin buckets.

## Decisions

**1. Two choices, `local` and `utc`, default `local`.** The browser knows its zone and the operator's question is "when
did it start for me". A list of named zones is more control than anyone asked for. The choice is a query value, so it is
read against a fixed list, kept in the fragment and shared with the link.

**2. Formatting takes the choice; the data does not.** The chart's time text is produced by functions that take the choice
and the zone name, and a change of choice redraws from the data already in memory, with no request. Bucket starts stay
epoch-aligned in UTC, so in a zone with a 30 or 45 minute offset a tick can fall on a half hour; the text is the true
local time to the minute.

**3. The axis note names the zone.** "times are UTC" becomes the zone in use (the short name the browser gives, such as
CST, or UTC), so a screenshot says which it is.

**4. Tests fix the zone.** The time functions are tested with the zone set explicitly (UTC, US Central across a daylight
saving change, and a half-hour zone) so the results do not depend on the machine that runs them.

**5. The ranking sorts by its own value.** Highest first, groups with no value last, ties by name. The table keeps its
order, because the table's problems-first order (health, then how well a rate is backed) answers a different question.
*Alternative:* keep a confidence-weighted order for the rate metrics; rejected, because a bar shorter than the one below
it reads as a bug.

**6. Thin bars say how many calls.** A rate or latency bar for a group with fewer than 20 calls in the window shows the count (under the requests metric the value is the count), so a
100% rate from one call is placed honestly and labelled as thin. 20 is a plain round number, not a statistical claim.

## Risks / Trade-offs

- [A shared screenshot shows local times the reader cannot map] → The axis note names the zone.
- [Daylight saving gives a repeated or skipped hour in a long window] → Ticks are computed from the bucket starts, which
  are real instants, so a repeated hour appears twice with the same label; covered by the test in decision 4.
- [A one-call group tops a rate ranking] → It is labelled with its call count.

## Migration Plan

None. The new fragment value is optional and its absence means `local`; a saved link without it opens in local time.

## Open Questions

None.
