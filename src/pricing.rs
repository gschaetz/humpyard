//! Token cost in integer micro-USD. USD per million tokens equals micro-USD per token, so a price
//! multiplies directly by a token count and sums stay exact.

use switchyard_protocol::Usage;

use crate::config::Price;
use crate::num::{f64_from_u64, u64_from_f64_rounded};

/// Cost of `usage` at `price`, rounded to the nearest micro-USD.
///
/// Non-cached input and cache-creation tokens bill at the input price, cache reads at the cached
/// price (the input price when none is configured), and output plus reasoning tokens at the
/// output price.
pub fn cost_micro_usd(usage: &Usage, price: &Price) -> u64 {
    let input = f64_from_u64(
        usage
            .input_tokens
            .unwrap_or(0)
            .saturating_add(usage.cache_creation_input_tokens().unwrap_or(0)),
    );
    let cached = f64_from_u64(usage.cached_input_tokens().unwrap_or(0));
    let output = f64_from_u64(
        usage
            .output_tokens
            .unwrap_or(0)
            .saturating_add(usage.reasoning_tokens.unwrap_or(0)),
    );
    let micro = input * price.input
        + cached * price.cached_input.unwrap_or(price.input)
        + output * price.output;
    u64_from_f64_rounded(micro)
}

/// Total tokens counted against token limits: everything sent and generated.
pub fn total_tokens(usage: &Usage) -> u64 {
    [
        usage.input_tokens,
        usage.cached_input_tokens(),
        usage.cache_creation_input_tokens(),
        usage.output_tokens,
        usage.reasoning_tokens,
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
    fn reasoning_tokens_bill_as_output() {
        let mut u = usage(0, 10);
        u.reasoning_tokens = Some(90);
        assert_eq!(cost_micro_usd(&u, &PRICE), 400);
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
    fn token_total_counts_everything() {
        let mut u = usage(10, 20);
        u.reasoning_tokens = Some(5);
        u.cache = Some(Box::new(InputCacheUsage {
            cached_input_tokens: Some(100),
            cache_creation_input_tokens: Some(7),
        }));
        assert_eq!(total_tokens(&u), 142);
    }
}
