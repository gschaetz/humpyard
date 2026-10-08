//! Token cost in integer micro-USD. USD per million tokens equals micro-USD per token, so a price
//! multiplies directly by a token count and sums stay exact.

use switchyard_protocol::Usage;

use crate::config::Price;
use crate::num::{f64_from_u64, u64_from_f64_rounded};

/// Cost of `usage` at `price`, rounded to the nearest micro-USD.
///
/// Non-cached input and cache-creation tokens bill at the input price, cache reads at the cached
/// price (the input price when none is configured), and output tokens at the output price.
///
/// Reasoning tokens are *part of* the output tokens in the chat, responses and messages wire
/// formats, so they are not added again (see `tests/usage.rs` and
/// `tests/switchyard_assumptions.rs`, which pin this against Switchyard's decoders).
pub fn cost_micro_usd(usage: &Usage, price: &Price) -> u64 {
    let input = f64_from_u64(
        usage
            .input_tokens
            .unwrap_or(0)
            .saturating_add(usage.cache_creation_input_tokens().unwrap_or(0)),
    );
    let cached = f64_from_u64(usage.cached_input_tokens().unwrap_or(0));
    let output = f64_from_u64(usage.output_tokens.unwrap_or(0));
    let micro = input * price.input
        + cached * price.cached_input.unwrap_or(price.input)
        + output * price.output;
    u64_from_f64_rounded(micro)
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
    };

    #[test]
    fn spec_example_costs_three_thousandths_of_a_dollar() {
        // 1,000 input at $1/M plus 500 output at $4/M is $0.003 = 3,000 micro-USD.
        assert_eq!(cost_micro_usd(&usage(1_000, 500), &PRICE), 3_000);
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
