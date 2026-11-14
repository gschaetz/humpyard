//! Token cost in integer micro-USD. USD per million tokens equals micro-USD per token, so a price
//! multiplies directly by a token count and sums stay exact.

use switchyard_protocol::Usage;

use crate::config::Price;
use crate::num::{f64_from_u64, u64_from_f64_rounded};

/// Cost of `usage` at `price`, rounded to the nearest micro-USD. The rates are the base price, or
/// the tier whose threshold the call's whole prompt (new input, cache reads and cache writes)
/// exceeds.
///
/// Non-cached input and cache-creation tokens bill at the input price, cache reads at the cached
/// price (the input price when none is configured), and output tokens at the output price.
///
/// Reasoning tokens are *part of* the output tokens in the chat, responses and messages wire
/// formats, so they are not added again (see `tests/usage.rs` and
/// `tests/switchyard_assumptions.rs`, which pin this against Switchyard's decoders).
pub fn cost_micro_usd(usage: &Usage, price: &Price) -> u64 {
    let prompt = usage
        .input_tokens
        .unwrap_or(0)
        .saturating_add(usage.cache_creation_input_tokens().unwrap_or(0))
        .saturating_add(usage.cached_input_tokens().unwrap_or(0));
    let rates = price.rates_for(prompt);
    let input = f64_from_u64(
        usage
            .input_tokens
            .unwrap_or(0)
            .saturating_add(usage.cache_creation_input_tokens().unwrap_or(0)),
    );
    let cached = f64_from_u64(usage.cached_input_tokens().unwrap_or(0));
    let output = f64_from_u64(usage.output_tokens.unwrap_or(0));
    let micro = input * rates.input
        + cached * rates.cached_input.unwrap_or(rates.input)
        + output * rates.output;
    u64_from_f64_rounded(micro)
}

/// The cost of token sums at an untiered `price`, in micro-USD (fractional), by the same rules as
/// [`cost_micro_usd`]: `input` is the fresh input plus the cache-creation tokens (billed at the input price), `cached`
/// the cache reads (the cached price, or the input price when none is set), `output` the output tokens (reasoning
/// included). With no tiers the cost is linear in the counts, so it can be had from sums instead of per call: pricing
/// one call here and in [`cost_micro_usd`] gives the same figure, and over many calls it differs from a sum of
/// per-call roundings by at most half a micro-USD a call.
pub fn linear_cost_micro_usd(price: &Price, input: u64, cached: u64, output: u64) -> f64 {
    f64_from_u64(input) * price.input
        + f64_from_u64(cached) * price.cached_input.unwrap_or(price.input)
        + f64_from_u64(output) * price.output
}

/// Why a name cannot be a baseline (`add-baseline-cost`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaselineRefusal {
    /// No target of the configuration uses an endpoint of that name.
    Unknown,
    /// The endpoint has no price.
    NoPrice,
    /// The price has tiers, so a call's cost depends on its prompt size and sums cannot give it.
    Tiered,
}

/// The price of the endpoint named `name` (`provider/model`) among `endpoints` (provider, model, price), the first
/// that names it winning, or why it cannot be a baseline.
///
/// # Errors
/// [`BaselineRefusal`] when no endpoint has that name, it has no price, or its price has tiers.
pub fn baseline_price<'a>(
    mut endpoints: impl Iterator<Item = (&'a str, &'a str, Option<&'a Price>)>,
    name: &str,
) -> Result<&'a Price, BaselineRefusal> {
    let (_, _, price) = endpoints
        .find(|(provider, model, _)| {
            name.strip_prefix(*provider)
                .and_then(|rest| rest.strip_prefix('/'))
                .is_some_and(|rest| rest == *model)
        })
        .ok_or(BaselineRefusal::Unknown)?;
    let price = price.ok_or(BaselineRefusal::NoPrice)?;
    if price.tiers.is_empty() {
        Ok(price)
    } else {
        Err(BaselineRefusal::Tiered)
    }
}

/// A lower bound on what a request costs at `price`, in micro-USD (fractional): `prompt_low` tokens at the
/// lowest input rate and `output` tokens at the lowest output rate, over the base price and every tier.
/// It can only under-count, so a request whose bound exceeds a ceiling is certain to exceed it.
pub fn cost_lower_bound_micro_usd(price: &Price, prompt_low: u64, output: u64) -> f64 {
    f64_from_u64(prompt_low) * price.min_input() + f64_from_u64(output) * price.min_output()
}

/// Total tokens counted against token limits: everything sent and generated. Reasoning tokens are
/// already inside the output count.
pub fn total_tokens(usage: &Usage) -> u64 {
    [
        usage.input_tokens,
        usage.cached_input_tokens(),
        usage.cache_creation_input_tokens(),
        usage.output_tokens,
    ]
    .into_iter()
    .fold(0, |sum, n| sum.saturating_add(n.unwrap_or(0)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use switchyard_protocol::InputCacheUsage;

    fn usage(input: u64, output: u64) -> Usage {
        Usage {
            input_tokens: Some(input),
            output_tokens: Some(output),
            ..Usage::default()
        }
    }

    const PRICE: Price = Price {
        input: 1.0,
        output: 4.0,
        cached_input: None,
        tiers: Vec::new(),
    };

    #[test]
    fn spec_example_costs_three_thousandths_of_a_dollar() {
        // 1,000 input at $1/M plus 500 output at $4/M is $0.003 = 3,000 micro-USD.
        assert_eq!(cost_micro_usd(&usage(1_000, 500), &PRICE), 3_000);
    }

    #[test]
    fn the_linear_cost_of_one_call_is_the_cost_of_that_call() {
        let mut u = usage(1_234, 567);
        u.cache = Some(Box::new(InputCacheUsage {
            cached_input_tokens: Some(8_000),
            cache_creation_input_tokens: Some(300),
        }));
        let cheap = Price {
            cached_input: Some(0.1),
            ..PRICE
        };
        for price in [&PRICE, &cheap] {
            // input = fresh + cache creation, cached = cache reads, as the ledger sums them.
            let linear = linear_cost_micro_usd(price, 1_234 + 300, 8_000, 567);
            assert_eq!(
                u64_from_f64_rounded(linear),
                cost_micro_usd(&u, price),
                "{price:?}"
            );
        }
    }

    #[test]
    fn the_linear_cost_of_sums_is_the_sum_of_the_costs_to_within_a_half_micro_usd_a_call() {
        let calls = [
            (10_u64, 3_u64, 2_u64),
            (999, 0, 41),
            (5, 5_000, 7),
            (1, 1, 1),
        ];
        let mut each = 0_u64;
        let (mut input, mut cached, mut output) = (0, 0, 0);
        for (i, c, o) in calls {
            let mut u = usage(i, o);
            u.cache = Some(Box::new(InputCacheUsage {
                cached_input_tokens: Some(c),
                cache_creation_input_tokens: None,
            }));
            each += cost_micro_usd(&u, &PRICE);
            (input, cached, output) = (input + i, cached + c, output + o);
        }
        let whole = linear_cost_micro_usd(&PRICE, input, cached, output);
        assert!((whole - f64_from_u64(each)).abs() <= 0.5 * f64_from_u64(calls.len() as u64));
    }

    #[test]
    fn a_baseline_is_a_priced_endpoint_without_tiers() {
        let plain = PRICE;
        let tiers = tiered();
        let endpoints = || {
            [
                ("openai", "gpt-big", Some(&plain)),
                ("groq", "llama", None),
                ("anthropic", "claude-big", Some(&tiers)),
                ("meta", "x/y", Some(&plain)),
            ]
            .into_iter()
        };
        assert_eq!(baseline_price(endpoints(), "openai/gpt-big"), Ok(&plain));
        assert_eq!(baseline_price(endpoints(), "meta/x/y"), Ok(&plain));
        assert_eq!(
            baseline_price(endpoints(), "groq/llama"),
            Err(BaselineRefusal::NoPrice)
        );
        assert_eq!(
            baseline_price(endpoints(), "anthropic/claude-big"),
            Err(BaselineRefusal::Tiered)
        );
        for unknown in [
            "",
            "openai",
            "openai/",
            "openai/gpt",
            "gpt-big",
            "OPENAI/gpt-big",
            "x/y",
        ] {
            assert_eq!(
                baseline_price(endpoints(), unknown),
                Err(BaselineRefusal::Unknown),
                "{unknown:?}"
            );
        }
    }

    #[test]
    fn cached_input_uses_its_own_price_or_falls_back_to_input() {
        let mut u = usage(100, 0);
        u.cache = Some(Box::new(InputCacheUsage {
            cached_input_tokens: Some(1_000),
            cache_creation_input_tokens: None,
        }));
        let cheap = Price {
            cached_input: Some(0.1),
            ..PRICE
        };
        assert_eq!(cost_micro_usd(&u, &cheap), 100 + 100);
        assert_eq!(cost_micro_usd(&u, &PRICE), 100 + 1_000);
    }

    fn tiered() -> Price {
        Price {
            tiers: vec![
                crate::config::PriceTier {
                    above_tokens: 100_000,
                    input: 3.0,
                    output: 12.0,
                    cached_input: Some(0.3),
                },
                crate::config::PriceTier {
                    above_tokens: 500_000,
                    input: 6.0,
                    output: 24.0,
                    cached_input: None,
                },
            ],
            ..PRICE
        }
    }

    #[test]
    fn a_tier_applies_only_once_the_prompt_exceeds_its_threshold() {
        let p = tiered();
        // At exactly the threshold the base price still applies; one token over switches tiers.
        assert_eq!(cost_micro_usd(&usage(100_000, 1_000), &p), 100_000 + 4_000);
        assert_eq!(
            cost_micro_usd(&usage(100_001, 1_000), &p),
            100_001 * 3 + 1_000 * 12
        );
        // The highest threshold exceeded wins, and a tier without a cached price uses its input.
        assert_eq!(cost_micro_usd(&usage(500_001, 0), &p), 500_001 * 6);
        assert_eq!(p.rates_for(0).input, 1.0);
        assert_eq!(p.rates_for(500_000).input, 3.0);
        assert_eq!(p.rates_for(500_001).input, 6.0);
    }

    #[test]
    fn the_prompt_size_that_picks_the_tier_includes_cached_and_cache_written_tokens() {
        let mut u = usage(10, 0);
        u.cache = Some(Box::new(InputCacheUsage {
            cached_input_tokens: Some(95_000),
            cache_creation_input_tokens: Some(5_000),
        }));
        // 10 + 95,000 + 5,000 = 100,010 prompt tokens: over the first tier, though only 10 are
        // fresh input. Cache writes bill at the tier's input, cache reads at its cached price.
        assert_eq!(
            cost_micro_usd(&u, &tiered()),
            (10 + 5_000) * 3 + 28_500 // 95,000 cached reads at $0.30/M
        );
    }

    #[test]
    fn the_highest_rate_stands_for_the_price_and_a_paid_tier_is_not_free() {
        assert_eq!(tiered().max_output(), 24.0);
        assert_eq!(PRICE.max_output(), 4.0);
        assert!(!tiered().is_free());
        let free_then_paid = Price {
            input: 0.0,
            output: 0.0,
            tiers: vec![crate::config::PriceTier {
                above_tokens: 1,
                input: 1.0,
                output: 1.0,
                cached_input: None,
            }],
            ..PRICE
        };
        assert!(
            !free_then_paid.is_free(),
            "free only up to a size is not free"
        );
        assert!(Price::FREE.is_free());
    }

    #[test]
    fn reasoning_tokens_are_part_of_output_and_not_billed_twice() {
        // 10 prompt tokens, 100 completion tokens of which 90 are reasoning.
        let mut u = usage(10, 100);
        u.reasoning_tokens = Some(90);
        assert_eq!(cost_micro_usd(&u, &PRICE), 10 + 100 * 4);
        assert_eq!(total_tokens(&u), 110);
    }

    #[test]
    fn free_price_and_missing_usage_cost_nothing() {
        assert_eq!(cost_micro_usd(&usage(5_000, 5_000), &Price::FREE), 0);
        assert_eq!(cost_micro_usd(&Usage::default(), &PRICE), 0);
    }

    #[test]
    fn fractional_micro_dollars_round_to_nearest() {
        let tiny = Price {
            input: 0.3,
            output: 0.0,
            cached_input: None,
            tiers: Vec::new(),
        };
        assert_eq!(cost_micro_usd(&usage(5, 0), &tiny), 2); // 1.5 rounds up
    }

    #[test]
    fn token_total_counts_input_cache_and_output_once() {
        let mut u = usage(10, 20);
        u.reasoning_tokens = Some(5);
        u.cache = Some(Box::new(InputCacheUsage {
            cached_input_tokens: Some(100),
            cache_creation_input_tokens: Some(7),
        }));
        assert_eq!(total_tokens(&u), 137); // reasoning (5) is inside output (20)
    }
}

#[cfg(test)]
mod properties {
    use super::*;
    use proptest::prelude::*;
    use switchyard_protocol::InputCacheUsage;

    /// Token counts up to a trillion: far beyond any real call, still exact in `f64`.
    const REALISTIC: u64 = 1_000_000_000_000;

    fn usage_strategy(max: u64) -> impl Strategy<Value = Usage> {
        (
            prop::option::of(0..=max),
            prop::option::of(0..=max),
            prop::option::of(0..=max),
            prop::option::of((prop::option::of(0..=max), prop::option::of(0..=max))),
        )
            .prop_map(|(input, output, reasoning, cache)| Usage {
                input_tokens: input,
                output_tokens: output,
                reasoning_tokens: reasoning,
                cache: cache.map(|(cached, created)| {
                    Box::new(InputCacheUsage {
                        cached_input_tokens: cached,
                        cache_creation_input_tokens: created,
                    })
                }),
                ..Usage::default()
            })
    }

    fn price_strategy() -> impl Strategy<Value = Price> {
        (
            0.0..1000.0f64,
            0.0..1000.0f64,
            prop::option::of(0.0..1000.0f64),
        )
            .prop_map(|(input, output, cached_input)| Price {
                input,
                output,
                cached_input,
                tiers: Vec::new(),
            })
    }

    proptest! {
        #[test]
        fn free_prices_always_cost_nothing(usage in usage_strategy(u64::MAX)) {
            prop_assert_eq!(cost_micro_usd(&usage, &Price::FREE), 0);
        }

        #[test]
        fn cost_never_panics_and_never_exceeds_the_priciest_rate_times_tokens(
            usage in usage_strategy(REALISTIC),
            price in price_strategy(),
        ) {
            let cost = cost_micro_usd(&usage, &price);
            let max_rate = price.input.max(price.output).max(price.cached_input.unwrap_or(0.0));
            // +1 for rounding to the nearest micro-USD.
            let ceiling = f64_from_u64(total_tokens(&usage)) * max_rate + 1.0;
            prop_assert!(f64_from_u64(cost) <= ceiling, "cost {cost} above ceiling {ceiling}");
        }

        #[test]
        fn more_output_tokens_never_cost_less(
            usage in usage_strategy(REALISTIC),
            extra in 0..=REALISTIC,
            price in price_strategy(),
        ) {
            let mut bigger = usage.clone();
            bigger.output_tokens = Some(usage.output_tokens.unwrap_or(0) + extra);
            prop_assert!(cost_micro_usd(&bigger, &price) >= cost_micro_usd(&usage, &price));
        }

        #[test]
        fn costs_add_up_within_one_micro_dollar_of_rounding(
            a in 0..=REALISTIC / 2,
            b in 0..=REALISTIC / 2,
            price in price_strategy(),
        ) {
            let of = |n: u64| Usage { input_tokens: Some(n), ..Usage::default() };
            let together = cost_micro_usd(&of(a + b), &price);
            let apart = cost_micro_usd(&of(a), &price) + cost_micro_usd(&of(b), &price);
            prop_assert!(together.abs_diff(apart) <= 1, "{together} vs {apart}");
        }

        #[test]
        fn reasoning_tokens_never_change_cost_or_totals(
            usage in usage_strategy(REALISTIC),
            reasoning in 0..=u64::MAX,
            price in price_strategy(),
        ) {
            let mut with = usage.clone();
            with.reasoning_tokens = Some(reasoning);
            let mut without = usage;
            without.reasoning_tokens = None;
            prop_assert_eq!(cost_micro_usd(&with, &price), cost_micro_usd(&without, &price));
            prop_assert_eq!(total_tokens(&with), total_tokens(&without));
        }

        #[test]
        fn the_token_total_saturates_and_covers_every_part(usage in usage_strategy(u64::MAX)) {
            let total = total_tokens(&usage);
            for part in [usage.input_tokens, usage.output_tokens] {
                prop_assert!(total >= part.unwrap_or(0));
            }
        }
    }
}
