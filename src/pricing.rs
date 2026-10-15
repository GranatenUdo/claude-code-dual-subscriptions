//! Hardcoded public API list prices for the monitor's API-equivalent cost
//! estimate. Subscriptions are not billed per token: the estimate answers "what
//! would these observed tokens cost at the published API price?".
//!
//! Prices are transcribed exactly from the provider pricing pages; nothing is
//! derived. When a model, tier or context band has no published price the
//! estimate is `None` (unpriced) instead of a guess. Update [`PRICES_AS_OF`],
//! the table and the tests together.

use serde::{Deserialize, Serialize};

use crate::providers::codex::translate::model_allowlist::{MODEL_ALIASES, split_tier_suffix};
use crate::providers::codex::translate::request::ServiceTier;

pub const PRICES_AS_OF: &str = "2026-10-14";
pub const CLAUDE_PRICING_URL: &str = "https://platform.claude.com/docs/en/about-claude/pricing";
pub const OPENAI_PRICING_URL: &str = "https://developers.openai.com/api/docs/pricing";

/// Which price list applies to a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PriceFamily {
    Claude,
    OpenAi,
}

impl PriceFamily {
    /// Only native Claude and Codex traffic is priced; every other provider is
    /// outside the hardcoded price lists.
    pub fn for_provider(provider: &str) -> Option<Self> {
        match provider {
            "anthropic" => Some(Self::Claude),
            "codex" => Some(Self::OpenAi),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PriceTier {
    #[default]
    Standard,
    /// OpenAI Fast (formerly Priority) or Claude fast mode.
    Fast,
    Ultrafast,
    Flex,
}

impl PriceTier {
    pub fn label(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Fast => "fast",
            Self::Ultrafast => "ultrafast",
            Self::Flex => "flex",
        }
    }

    pub fn from_service_tier(tier: Option<&ServiceTier>) -> Self {
        match tier {
            Some(ServiceTier::Priority) => Self::Fast,
            Some(ServiceTier::Ultrafast) => Self::Ultrafast,
            Some(ServiceTier::Flex) => Self::Flex,
            None => Self::Standard,
        }
    }

    /// Wire value of an OpenAI `service_tier` field.
    pub fn from_wire(tier: Option<&str>) -> Self {
        match tier {
            Some("priority" | "fast") => Self::Fast,
            Some("ultrafast") => Self::Ultrafast,
            Some("flex") => Self::Flex,
            _ => Self::Standard,
        }
    }
}

/// Price per million tokens in units of 1/10,000 USD, so every published
/// price (down to $0.0625) is an exact integer.
type Rate = u64;

const RATE_SCALE: u128 = 10_000;

const fn usd(per_mtok: f64) -> Rate {
    (per_mtok * 10_000.0 + 0.5) as Rate
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rates {
    pub input: Rate,
    pub cache_read: Rate,
    /// `None` where the page lists no cache-write price; writes then bill at
    /// the input rate.
    pub cache_write_5m: Option<Rate>,
    pub cache_write_1h: Option<Rate>,
    pub output: Rate,
}

/// Claude row: input, 5m write, 1h write, cache read, output.
const fn claude(input: f64, write_5m: f64, write_1h: f64, read: f64, output: f64) -> Rates {
    Rates {
        input: usd(input),
        cache_read: usd(read),
        cache_write_5m: Some(usd(write_5m)),
        cache_write_1h: Some(usd(write_1h)),
        output: usd(output),
    }
}

/// OpenAI row: input, cached input, cache write, output.
const fn openai(input: f64, cached: f64, write: f64, output: f64) -> Rates {
    Rates {
        input: usd(input),
        cache_read: usd(cached),
        cache_write_5m: Some(usd(write)),
        cache_write_1h: Some(usd(write)),
        output: usd(output),
    }
}

/// OpenAI row without a published cache-write price.
const fn openai_no_write(input: f64, cached: f64, output: f64) -> Rates {
    Rates {
        input: usd(input),
        cache_read: usd(cached),
        cache_write_5m: None,
        cache_write_1h: None,
        output: usd(output),
    }
}

#[derive(Debug, Clone, Copy)]
enum LongContext {
    /// One price across the whole context window.
    None,
    /// A prompt above `above` tokens reprices the whole request.
    Rates { above: u64, rates: Rates },
    /// Above `above` tokens the page lists no price for this tier.
    Unpriced { above: u64 },
}

const OPENAI_LONG_CONTEXT: u64 = 272_000;
const HAIKU_5_5_LONG_CONTEXT: u64 = 100_000;

struct PriceEntry {
    family: PriceFamily,
    models: &'static [&'static str],
    tier: PriceTier,
    short: Rates,
    long: LongContext,
}

const fn entry(
    family: PriceFamily,
    models: &'static [&'static str],
    tier: PriceTier,
    short: Rates,
    long: LongContext,
) -> PriceEntry {
    PriceEntry {
        family,
        models,
        tier,
        short,
        long,
    }
}

const fn long(rates: Rates) -> LongContext {
    LongContext::Rates {
        above: OPENAI_LONG_CONTEXT,
        rates,
    }
}

use LongContext::None as FLAT;
use PriceFamily::{Claude as CL, OpenAi as OA};
use PriceTier::{Fast, Flex, Standard, Ultrafast};

const LONG_UNPRICED: LongContext = LongContext::Unpriced {
    above: OPENAI_LONG_CONTEXT,
};

#[rustfmt::skip]
static PRICES: &[PriceEntry] = &[
    // Claude, https://platform.claude.com/docs/en/about-claude/pricing
    entry(CL, &["claude-fable-5-1", "claude-mythos-5-1"], Standard, claude(10.0, 12.5, 20.0, 0.25, 50.0), FLAT),
    entry(CL, &["claude-fable-5", "claude-mythos-5"], Standard, claude(10.0, 12.5, 20.0, 1.0, 50.0), FLAT),
    entry(CL, &["claude-opus-5-5"], Standard, claude(4.0, 5.0, 8.0, 0.20, 20.0), FLAT),
    entry(CL, &["claude-opus-5", "claude-opus-4-8", "claude-opus-4-7", "claude-opus-4-6", "claude-opus-4-5"], Standard, claude(5.0, 6.25, 10.0, 0.50, 25.0), FLAT),
    entry(CL, &["claude-sonnet-5-5"], Standard, claude(2.0, 2.5, 4.0, 0.10, 10.0), FLAT),
    entry(CL, &["claude-sonnet-5"], Standard, claude(2.0, 2.5, 4.0, 0.20, 10.0), FLAT),
    entry(CL, &["claude-sonnet-4-6", "claude-sonnet-4-5"], Standard, claude(3.0, 3.75, 6.0, 0.30, 15.0), FLAT),
    entry(CL, &["claude-haiku-5-5"], Standard, claude(0.10, 0.125, 0.20, 0.01, 0.50),
        LongContext::Rates { above: HAIKU_5_5_LONG_CONTEXT, rates: claude(0.50, 0.625, 1.0, 0.05, 2.50) }),
    entry(CL, &["claude-haiku-4-5"], Standard, claude(1.0, 1.25, 2.0, 0.10, 5.0), FLAT),
    // Claude fast mode: fast input/output, with the standard cache multipliers
    // (5m write 1.25x, 1h write 2x, read at the model's read multiplier).
    entry(CL, &["claude-opus-5-5"], Fast, claude(8.0, 10.0, 16.0, 0.40, 40.0), FLAT),
    entry(CL, &["claude-opus-5", "claude-opus-4-8"], Fast, claude(10.0, 12.5, 20.0, 1.0, 50.0), FLAT),

    // OpenAI, https://developers.openai.com/api/docs/pricing
    entry(OA, &["gpt-6-astra"], Standard, openai(10.0, 1.0, 12.5, 50.0), long(openai(20.0, 2.0, 25.0, 75.0))),
    entry(OA, &["gpt-6.1-sol"], Standard, openai(2.0, 0.10, 2.5, 10.0), long(openai(4.0, 0.20, 5.0, 15.0))),
    entry(OA, &["gpt-6-sol"], Standard, openai(2.0, 0.20, 2.5, 10.0), long(openai(4.0, 0.40, 5.0, 15.0))),
    entry(OA, &["gpt-6-luna"], Standard, openai(0.10, 0.01, 0.125, 0.50), long(openai(0.20, 0.02, 0.25, 0.75))),
    entry(OA, &["gpt-5.6-sol"], Standard, openai(4.0, 0.40, 5.0, 20.0), long(openai(8.0, 0.80, 10.0, 30.0))),
    entry(OA, &["gpt-5.6-terra"], Standard, openai(2.0, 0.20, 2.5, 12.0), long(openai(4.0, 0.40, 5.0, 18.0))),
    entry(OA, &["gpt-5.6-luna"], Standard, openai(0.20, 0.02, 0.25, 1.20), long(openai(0.40, 0.04, 0.50, 1.80))),
    entry(OA, &["gpt-5.5"], Standard, openai_no_write(5.0, 0.50, 30.0), long(openai_no_write(10.0, 1.0, 45.0))),
    entry(OA, &["gpt-5.4"], Standard, openai_no_write(2.50, 0.25, 15.0), long(openai_no_write(5.0, 0.50, 22.50))),
    entry(OA, &["gpt-5.4-mini"], Standard, openai_no_write(0.75, 0.075, 4.50), FLAT),
    entry(OA, &["gpt-5.2", "gpt-5.3-codex"], Standard, openai_no_write(1.75, 0.175, 14.0), FLAT),

    entry(OA, &["gpt-6-astra"], Fast, openai(20.0, 2.0, 25.0, 100.0), long(openai(40.0, 4.0, 50.0, 150.0))),
    entry(OA, &["gpt-6.1-sol"], Fast, openai(4.0, 0.20, 5.0, 20.0), long(openai(8.0, 0.40, 10.0, 30.0))),
    entry(OA, &["gpt-6-sol"], Fast, openai(4.0, 0.40, 5.0, 20.0), long(openai(8.0, 0.80, 10.0, 30.0))),
    entry(OA, &["gpt-6-luna"], Fast, openai(0.20, 0.02, 0.25, 1.0), long(openai(0.40, 0.04, 0.50, 1.50))),
    entry(OA, &["gpt-5.6-sol"], Fast, openai(8.0, 0.80, 10.0, 40.0), long(openai(16.0, 1.60, 20.0, 60.0))),
    entry(OA, &["gpt-5.6-terra"], Fast, openai(4.0, 0.40, 5.0, 24.0), long(openai(8.0, 0.80, 10.0, 36.0))),
    entry(OA, &["gpt-5.6-luna"], Fast, openai(0.40, 0.04, 0.50, 2.40), long(openai(0.80, 0.08, 1.0, 3.60))),
    entry(OA, &["gpt-5.5"], Fast, openai_no_write(12.50, 1.25, 75.0), LONG_UNPRICED),
    entry(OA, &["gpt-5.4"], Fast, openai_no_write(5.0, 0.50, 30.0), LONG_UNPRICED),
    entry(OA, &["gpt-5.4-mini"], Fast, openai_no_write(1.50, 0.15, 9.0), FLAT),
    entry(OA, &["gpt-5.2", "gpt-5.3-codex"], Fast, openai_no_write(3.50, 0.35, 28.0), FLAT),

    entry(OA, &["gpt-6-astra"], Ultrafast, openai(60.0, 6.0, 75.0, 300.0), long(openai(120.0, 12.0, 150.0, 450.0))),
    entry(OA, &["gpt-6.1-sol"], Ultrafast, openai(12.0, 0.60, 15.0, 60.0), long(openai(24.0, 1.20, 30.0, 90.0))),

    entry(OA, &["gpt-6-astra"], Flex, openai(5.0, 0.50, 6.25, 25.0), long(openai(10.0, 1.0, 12.5, 37.5))),
    entry(OA, &["gpt-6.1-sol"], Flex, openai(1.0, 0.05, 1.25, 5.0), long(openai(2.0, 0.10, 2.5, 7.5))),
    entry(OA, &["gpt-6-sol"], Flex, openai(1.0, 0.10, 1.25, 5.0), long(openai(2.0, 0.20, 2.5, 7.5))),
    entry(OA, &["gpt-6-luna"], Flex, openai(0.05, 0.005, 0.0625, 0.25), long(openai(0.10, 0.01, 0.125, 0.375))),
    entry(OA, &["gpt-5.6-sol"], Flex, openai(2.0, 0.20, 2.5, 10.0), long(openai(4.0, 0.40, 5.0, 15.0))),
    entry(OA, &["gpt-5.6-terra"], Flex, openai(1.0, 0.10, 1.25, 6.0), long(openai(2.0, 0.20, 2.5, 9.0))),
    entry(OA, &["gpt-5.6-luna"], Flex, openai(0.10, 0.01, 0.125, 0.60), long(openai(0.20, 0.02, 0.25, 0.90))),
    entry(OA, &["gpt-5.5"], Flex, openai_no_write(2.50, 0.25, 15.0), long(openai_no_write(5.0, 0.50, 22.50))),
    entry(OA, &["gpt-5.4"], Flex, openai_no_write(1.25, 0.13, 7.50), long(openai_no_write(2.50, 0.25, 11.25))),
    entry(OA, &["gpt-5.4-mini"], Flex, openai_no_write(0.375, 0.0375, 2.25), FLAT),
    entry(OA, &["gpt-5.2"], Flex, openai_no_write(0.875, 0.0875, 7.0), FLAT),
];

/// Claude models whose fast-mode requests run, and bill, at standard rates.
const CLAUDE_FAST_AT_STANDARD: &[&str] = &["claude-opus-4-6"];

/// Models the proxy can route but whose price is not published.
pub const UNPRICED_MODELS: &[&str] = &["gpt-5.3-codex-spark"];

/// Token counts of one request, all in Anthropic semantics: `input` excludes
/// cache reads and writes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PricedUsage {
    pub input: u64,
    pub cache_read: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
    pub output: u64,
}

impl PricedUsage {
    /// Every input token of the prompt, which selects the long-context band.
    pub fn prompt_tokens(&self) -> u64 {
        self.input
            .saturating_add(self.cache_read)
            .saturating_add(self.cache_write_5m)
            .saturating_add(self.cache_write_1h)
    }
}

/// Canonical model ID used as the price key: strips the `[1m]` context hint,
/// local tier suffixes, Claude date snapshots and local aliases.
pub fn normalize_model(family: PriceFamily, model: &str) -> String {
    let model = strip_context_hint(model.trim());
    match family {
        PriceFamily::Claude => {
            let model = match model {
                "haiku" => "claude-haiku-4-5",
                "sonnet" => "claude-sonnet-5",
                "opus" => "claude-opus-5-5",
                "fable" => "claude-fable-5",
                model => model,
            };
            strip_date_suffix(model).to_string()
        }
        PriceFamily::OpenAi => {
            let model = MODEL_ALIASES
                .iter()
                .find(|(alias, _)| *alias == model)
                .map(|(_, target)| *target)
                .unwrap_or(model);
            split_tier_suffix(model)
                .map(|(base, _)| base)
                .unwrap_or(model)
                .to_string()
        }
    }
}

/// Tier implied by a local `-fast` / `-ultrafast` model suffix.
pub fn tier_from_model_suffix(model: &str) -> Option<PriceTier> {
    split_tier_suffix(strip_context_hint(model.trim()))
        .map(|(_, tier)| PriceTier::from_service_tier(Some(&tier)))
}

fn strip_context_hint(model: &str) -> &str {
    let bytes = model.as_bytes();
    if bytes.len() >= 4 && bytes[bytes.len() - 4..].eq_ignore_ascii_case(b"[1m]") {
        &model[..model.len() - 4]
    } else {
        model
    }
}

fn strip_date_suffix(model: &str) -> &str {
    match model.rsplit_once('-') {
        Some((base, date)) if date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit()) => base,
        _ => model,
    }
}

fn find_entry(family: PriceFamily, model: &str, tier: PriceTier) -> Option<&'static PriceEntry> {
    PRICES
        .iter()
        .find(|entry| entry.family == family && entry.tier == tier && entry.models.contains(&model))
}

/// Rates for one request, or `None` when no published price applies.
pub fn rates_for(
    family: PriceFamily,
    model: &str,
    tier: PriceTier,
    prompt_tokens: u64,
) -> Option<Rates> {
    let model = normalize_model(family, model);
    let entry = find_entry(family, &model, tier).or_else(|| {
        // The pricing page bills fast-mode requests on these models at
        // standard rates; any other model without a fast price is unpriced.
        (family == PriceFamily::Claude
            && tier == PriceTier::Fast
            && CLAUDE_FAST_AT_STANDARD.contains(&model.as_str()))
        .then(|| find_entry(family, &model, PriceTier::Standard))
        .flatten()
    })?;
    match entry.long {
        LongContext::None => Some(entry.short),
        LongContext::Rates { above, rates } if prompt_tokens > above => Some(rates),
        LongContext::Unpriced { above } if prompt_tokens > above => None,
        LongContext::Rates { .. } | LongContext::Unpriced { .. } => Some(entry.short),
    }
}

/// API-equivalent cost of one request in micro-USD, rounded to the nearest
/// micro-dollar, or `None` when the request cannot be priced.
pub fn estimate_micros(
    family: PriceFamily,
    model: &str,
    tier: PriceTier,
    usage: &PricedUsage,
) -> Option<u64> {
    let rates = rates_for(family, model, tier, usage.prompt_tokens())?;
    let write_5m = rates.cache_write_5m.unwrap_or(rates.input);
    let write_1h = rates.cache_write_1h.unwrap_or(write_5m);
    let scaled = u128::from(usage.input) * u128::from(rates.input)
        + u128::from(usage.cache_read) * u128::from(rates.cache_read)
        + u128::from(usage.cache_write_5m) * u128::from(write_5m)
        + u128::from(usage.cache_write_1h) * u128::from(write_1h)
        + u128::from(usage.output) * u128::from(rates.output);
    u64::try_from((scaled + RATE_SCALE / 2) / RATE_SCALE).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const M: u64 = 1_000_000;

    fn usage(input: u64, output: u64) -> PricedUsage {
        PricedUsage {
            input,
            output,
            ..PricedUsage::default()
        }
    }

    fn cost(family: PriceFamily, model: &str, tier: PriceTier, usage: PricedUsage) -> Option<u64> {
        estimate_micros(family, model, tier, &usage)
    }

    /// Published rates for one row as (input, read, write_5m, write_1h,
    /// output), with missing cache-write prices billed like the page says.
    fn per_mtok(
        family: PriceFamily,
        model: &str,
        tier: PriceTier,
        long: bool,
    ) -> (u64, u64, u64, u64, u64) {
        let prompt = if long { 300_000 } else { 50_000 };
        let rates = rates_for(family, model, tier, prompt).unwrap();
        let w5 = rates.cache_write_5m.unwrap_or(rates.input);
        (
            rates.input,
            rates.cache_read,
            w5,
            rates.cache_write_1h.unwrap_or(w5),
            rates.output,
        )
    }

    fn d(per_mtok: f64) -> u64 {
        usd(per_mtok)
    }

    #[test]
    fn opus_5_5_million_in_and_out_costs_24_dollars() {
        assert_eq!(
            cost(CL, "claude-opus-5-5", Standard, usage(M, M)),
            Some(24 * M)
        );
    }

    #[test]
    fn claude_rows_match_published_prices() {
        let rows: &[(&str, PriceTier, f64, f64, f64, f64, f64)] = &[
            ("claude-fable-5-1", Standard, 10.0, 0.25, 12.5, 20.0, 50.0),
            ("claude-mythos-5-1", Standard, 10.0, 0.25, 12.5, 20.0, 50.0),
            ("claude-fable-5", Standard, 10.0, 1.0, 12.5, 20.0, 50.0),
            ("claude-mythos-5", Standard, 10.0, 1.0, 12.5, 20.0, 50.0),
            ("claude-opus-5-5", Standard, 4.0, 0.20, 5.0, 8.0, 20.0),
            ("claude-opus-5", Standard, 5.0, 0.50, 6.25, 10.0, 25.0),
            ("claude-opus-4-8", Standard, 5.0, 0.50, 6.25, 10.0, 25.0),
            ("claude-opus-4-7", Standard, 5.0, 0.50, 6.25, 10.0, 25.0),
            ("claude-opus-4-6", Standard, 5.0, 0.50, 6.25, 10.0, 25.0),
            ("claude-opus-4-5", Standard, 5.0, 0.50, 6.25, 10.0, 25.0),
            ("claude-sonnet-5-5", Standard, 2.0, 0.10, 2.5, 4.0, 10.0),
            ("claude-sonnet-5", Standard, 2.0, 0.20, 2.5, 4.0, 10.0),
            ("claude-sonnet-4-6", Standard, 3.0, 0.30, 3.75, 6.0, 15.0),
            ("claude-sonnet-4-5", Standard, 3.0, 0.30, 3.75, 6.0, 15.0),
            ("claude-haiku-5-5", Standard, 0.10, 0.01, 0.125, 0.20, 0.50),
            ("claude-haiku-4-5", Standard, 1.0, 0.10, 1.25, 2.0, 5.0),
            ("claude-opus-5-5", Fast, 8.0, 0.40, 10.0, 16.0, 40.0),
            ("claude-opus-5", Fast, 10.0, 1.0, 12.5, 20.0, 50.0),
            ("claude-opus-4-8", Fast, 10.0, 1.0, 12.5, 20.0, 50.0),
        ];
        for &(model, tier, input, read, w5, w1, out) in rows {
            assert_eq!(
                per_mtok(CL, model, tier, false),
                (d(input), d(read), d(w5), d(w1), d(out)),
                "{model} {tier:?}"
            );
        }
    }

    #[test]
    fn haiku_5_5_reprices_whole_request_above_100k_prompt_tokens() {
        let at = PricedUsage {
            input: 40_000,
            cache_read: 40_000,
            cache_write_5m: 10_000,
            cache_write_1h: 10_000,
            output: 1_000,
        };
        assert_eq!(at.prompt_tokens(), 100_000);
        let above = PricedUsage {
            input: 40_001,
            ..at
        };
        let rates =
            |u: &PricedUsage| rates_for(CL, "claude-haiku-5-5", Standard, u.prompt_tokens());
        assert_eq!(rates(&at).unwrap().input, usd(0.10));
        assert_eq!(rates(&above).unwrap().input, usd(0.50));
        assert_eq!(rates(&above).unwrap().cache_read, usd(0.05));
        assert_eq!(rates(&above).unwrap().cache_write_5m, Some(usd(0.625)));
        assert_eq!(rates(&above).unwrap().cache_write_1h, Some(usd(1.0)));
        assert_eq!(rates(&above).unwrap().output, usd(2.50));
    }

    #[test]
    fn openai_rows_match_published_prices() {
        // (model, tier, input, cached, write, output) for short context.
        let rows: &[(&str, PriceTier, f64, f64, f64, f64)] = &[
            ("gpt-6-astra", Standard, 10.0, 1.0, 12.5, 50.0),
            ("gpt-6.1-sol", Standard, 2.0, 0.10, 2.5, 10.0),
            ("gpt-6-sol", Standard, 2.0, 0.20, 2.5, 10.0),
            ("gpt-6-luna", Standard, 0.10, 0.01, 0.125, 0.50),
            ("gpt-5.6-sol", Standard, 4.0, 0.40, 5.0, 20.0),
            ("gpt-5.6-terra", Standard, 2.0, 0.20, 2.5, 12.0),
            ("gpt-5.6-luna", Standard, 0.20, 0.02, 0.25, 1.20),
            ("gpt-5.5", Standard, 5.0, 0.50, 5.0, 30.0),
            ("gpt-5.4", Standard, 2.50, 0.25, 2.50, 15.0),
            ("gpt-5.4-mini", Standard, 0.75, 0.075, 0.75, 4.50),
            ("gpt-5.2", Standard, 1.75, 0.175, 1.75, 14.0),
            ("gpt-5.3-codex", Standard, 1.75, 0.175, 1.75, 14.0),
            ("gpt-6-astra", Fast, 20.0, 2.0, 25.0, 100.0),
            ("gpt-6.1-sol", Fast, 4.0, 0.20, 5.0, 20.0),
            ("gpt-6-sol", Fast, 4.0, 0.40, 5.0, 20.0),
            ("gpt-6-luna", Fast, 0.20, 0.02, 0.25, 1.0),
            ("gpt-5.6-sol", Fast, 8.0, 0.80, 10.0, 40.0),
            ("gpt-5.6-terra", Fast, 4.0, 0.40, 5.0, 24.0),
            ("gpt-5.6-luna", Fast, 0.40, 0.04, 0.50, 2.40),
            ("gpt-5.5", Fast, 12.50, 1.25, 12.50, 75.0),
            ("gpt-5.4", Fast, 5.0, 0.50, 5.0, 30.0),
            ("gpt-5.4-mini", Fast, 1.50, 0.15, 1.50, 9.0),
            ("gpt-5.2", Fast, 3.50, 0.35, 3.50, 28.0),
            ("gpt-5.3-codex", Fast, 3.50, 0.35, 3.50, 28.0),
            ("gpt-6-astra", Ultrafast, 60.0, 6.0, 75.0, 300.0),
            ("gpt-6.1-sol", Ultrafast, 12.0, 0.60, 15.0, 60.0),
            ("gpt-6-astra", Flex, 5.0, 0.50, 6.25, 25.0),
            ("gpt-6.1-sol", Flex, 1.0, 0.05, 1.25, 5.0),
            ("gpt-6-sol", Flex, 1.0, 0.10, 1.25, 5.0),
            ("gpt-6-luna", Flex, 0.05, 0.005, 0.0625, 0.25),
            ("gpt-5.6-sol", Flex, 2.0, 0.20, 2.5, 10.0),
            ("gpt-5.6-terra", Flex, 1.0, 0.10, 1.25, 6.0),
            ("gpt-5.6-luna", Flex, 0.10, 0.01, 0.125, 0.60),
            ("gpt-5.5", Flex, 2.50, 0.25, 2.50, 15.0),
            ("gpt-5.4", Flex, 1.25, 0.13, 1.25, 7.50),
            ("gpt-5.4-mini", Flex, 0.375, 0.0375, 0.375, 2.25),
            ("gpt-5.2", Flex, 0.875, 0.0875, 0.875, 7.0),
        ];
        for &(model, tier, input, cached, write, out) in rows {
            assert_eq!(
                per_mtok(OA, model, tier, false),
                (d(input), d(cached), d(write), d(write), d(out)),
                "{model} {tier:?}"
            );
        }
    }

    #[test]
    fn openai_long_context_rows_match_published_prices() {
        let rows: &[(&str, PriceTier, f64, f64, f64, f64)] = &[
            ("gpt-6-astra", Standard, 20.0, 2.0, 25.0, 75.0),
            ("gpt-6.1-sol", Standard, 4.0, 0.20, 5.0, 15.0),
            ("gpt-6-sol", Standard, 4.0, 0.40, 5.0, 15.0),
            ("gpt-6-luna", Standard, 0.20, 0.02, 0.25, 0.75),
            ("gpt-5.6-sol", Standard, 8.0, 0.80, 10.0, 30.0),
            ("gpt-5.6-terra", Standard, 4.0, 0.40, 5.0, 18.0),
            ("gpt-5.6-luna", Standard, 0.40, 0.04, 0.50, 1.80),
            ("gpt-5.5", Standard, 10.0, 1.0, 10.0, 45.0),
            ("gpt-5.4", Standard, 5.0, 0.50, 5.0, 22.50),
            ("gpt-6-astra", Fast, 40.0, 4.0, 50.0, 150.0),
            ("gpt-6.1-sol", Fast, 8.0, 0.40, 10.0, 30.0),
            ("gpt-6-sol", Fast, 8.0, 0.80, 10.0, 30.0),
            ("gpt-6-luna", Fast, 0.40, 0.04, 0.50, 1.50),
            ("gpt-5.6-sol", Fast, 16.0, 1.60, 20.0, 60.0),
            ("gpt-5.6-terra", Fast, 8.0, 0.80, 10.0, 36.0),
            ("gpt-5.6-luna", Fast, 0.80, 0.08, 1.0, 3.60),
            ("gpt-6-astra", Ultrafast, 120.0, 12.0, 150.0, 450.0),
            ("gpt-6.1-sol", Ultrafast, 24.0, 1.20, 30.0, 90.0),
            ("gpt-6-astra", Flex, 10.0, 1.0, 12.5, 37.5),
            ("gpt-6.1-sol", Flex, 2.0, 0.10, 2.5, 7.5),
            ("gpt-6-sol", Flex, 2.0, 0.20, 2.5, 7.5),
            ("gpt-6-luna", Flex, 0.10, 0.01, 0.125, 0.375),
            ("gpt-5.6-sol", Flex, 4.0, 0.40, 5.0, 15.0),
            ("gpt-5.6-terra", Flex, 2.0, 0.20, 2.5, 9.0),
            ("gpt-5.6-luna", Flex, 0.20, 0.02, 0.25, 0.90),
            ("gpt-5.5", Flex, 5.0, 0.50, 5.0, 22.50),
            ("gpt-5.4", Flex, 2.50, 0.25, 2.50, 11.25),
        ];
        for &(model, tier, input, cached, write, out) in rows {
            assert_eq!(
                per_mtok(OA, model, tier, true),
                (d(input), d(cached), d(write), d(write), d(out)),
                "{model} {tier:?}"
            );
        }
    }

    #[test]
    fn openai_long_context_starts_above_272k_prompt_tokens() {
        let short = PricedUsage {
            input: 72_000,
            cache_read: 200_000,
            ..PricedUsage::default()
        };
        assert_eq!(short.prompt_tokens(), 272_000);
        let long = PricedUsage {
            input: 72_001,
            ..short
        };
        assert_eq!(
            rates_for(OA, "gpt-6-astra", Standard, short.prompt_tokens())
                .unwrap()
                .input,
            usd(10.0)
        );
        assert_eq!(
            rates_for(OA, "gpt-6-astra", Standard, long.prompt_tokens())
                .unwrap()
                .input,
            usd(20.0)
        );
    }

    #[test]
    fn missing_prices_are_unpriced_not_estimated() {
        let small = usage(1_000, 1_000);
        let long = usage(300_000, 1_000);
        assert_eq!(cost(OA, "gpt-5.3-codex-spark", Standard, small), None);
        assert_eq!(cost(OA, "gpt-9-unknown", Standard, small), None);
        assert_eq!(cost(CL, "claude-future-model", Standard, small), None);
        // No Ultrafast row outside astra and 6.1-sol; no Flex row for codex.
        assert_eq!(cost(OA, "gpt-6-sol", Ultrafast, small), None);
        assert_eq!(cost(OA, "gpt-5.3-codex", Flex, small), None);
        // Fast long context is unpublished for gpt-5.5 and gpt-5.4.
        assert_eq!(cost(OA, "gpt-5.5", Fast, long), None);
        assert_eq!(cost(OA, "gpt-5.4", Fast, long), None);
        assert!(cost(OA, "gpt-5.5", Fast, small).is_some());
        // Claude has no Flex or Ultrafast tiers.
        assert_eq!(cost(CL, "claude-opus-5-5", Flex, small), None);
    }

    #[test]
    fn claude_fast_without_fast_price_bills_standard_only_where_published() {
        assert_eq!(
            cost(CL, "claude-opus-4-6", Fast, usage(M, 0)),
            cost(CL, "claude-opus-4-6", Standard, usage(M, 0))
        );
        assert_eq!(cost(CL, "claude-sonnet-5-5", Fast, usage(M, 0)), None);
        assert_eq!(cost(CL, "claude-opus-4-7", Fast, usage(M, 0)), None);
    }

    #[test]
    fn model_ids_are_normalized_before_lookup() {
        assert_eq!(
            normalize_model(CL, "claude-haiku-4-5-20251001"),
            "claude-haiku-4-5"
        );
        assert_eq!(
            normalize_model(CL, "claude-opus-5-5[1m]"),
            "claude-opus-5-5"
        );
        assert_eq!(normalize_model(CL, "opus"), "claude-opus-5-5");
        assert_eq!(normalize_model(OA, "gpt-6-astra[1M]"), "gpt-6-astra");
        assert_eq!(normalize_model(OA, "gpt-6-astra-ultrafast"), "gpt-6-astra");
        assert_eq!(normalize_model(OA, "gpt-6.1-sol-fast"), "gpt-6.1-sol");
        assert_eq!(normalize_model(OA, "opus"), "gpt-6-sol");
        assert_eq!(tier_from_model_suffix("gpt-6-astra-fast[1m]"), Some(Fast));
        assert_eq!(tier_from_model_suffix("gpt-6-astra"), None);
        // Exact IDs only: the Opus 5 and Opus 5.5 lines are priced separately.
        assert_ne!(
            cost(CL, "claude-opus-5", Standard, usage(M, 0)),
            cost(CL, "claude-opus-5-5", Standard, usage(M, 0))
        );
    }

    #[test]
    fn rounding_is_to_the_nearest_micro_dollar() {
        // 1 token of gpt-6-luna flex cached input is $0.000000005.
        let read = PricedUsage {
            cache_read: 1,
            ..PricedUsage::default()
        };
        assert_eq!(cost(OA, "gpt-6-luna", Flex, read), Some(0));
        let read = PricedUsage {
            cache_read: 100,
            ..PricedUsage::default()
        };
        assert_eq!(cost(OA, "gpt-6-luna", Flex, read), Some(1));
    }

    #[test]
    fn every_routable_model_and_tier_is_priced_or_listed_unpriced() {
        use crate::providers::codex::translate::model_allowlist::tier_model_variants;
        for model in crate::registry::CODEX_MODELS {
            let mut names = vec![model.to_string()];
            names.extend(tier_model_variants(model));
            for name in names {
                let tier = tier_from_model_suffix(&name).unwrap_or(Standard);
                let priced = cost(OA, &name, tier, usage(1_000, 1_000)).is_some();
                assert!(
                    priced || UNPRICED_MODELS.contains(model),
                    "{name} ({tier:?}) has no price and is not listed unpriced"
                );
            }
        }
        for alias in crate::registry::ANTHROPIC_STYLE_ALIASES {
            assert!(
                cost(CL, alias, Standard, usage(1_000, 1_000)).is_some(),
                "Claude alias {alias} has no price"
            );
        }
    }
}
