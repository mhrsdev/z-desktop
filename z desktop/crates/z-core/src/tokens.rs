//! Local token estimation — no tokenizer dependency, no network call.
//!
//! Purpose: budget decisions BEFORE sending a request (will this fit? how
//! much history can we keep? do we need compaction?). Exact counts are not
//! required for that; a ±10% estimator is. When real usage numbers come back
//! from provider responses they should be preferred and used to calibrate.
//!
//! Model: chars/4 baseline (BPE average for English/code), with adjustments:
//! - CJK characters cost roughly one token each (~1.0–1.5), not 0.25.
//! - Whitespace runs and punctuation-heavy code skew slightly high on chars/4;
//!   a small correction keeps code estimates honest.
//! - JSON structure overhead per message (role markers etc.) is counted.

/// Estimated tokens for arbitrary text.
pub fn estimate(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    let mut cjk = 0usize;
    let mut other = 0usize;
    for ch in text.chars() {
        // CJK unified ideographs + common fullwidth ranges.
        let is_cjk = matches!(ch as u32,
            0x2E80..=0x9FFF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF | 0x20000..=0x2FA1F);
        if is_cjk {
            cjk += 1;
        } else {
            other += 1;
        }
    }
    // Baseline: ~4 chars per token for latin/text/code.
    let base = other.div_ceil(4);
    // CJK: ~1 token per character (slightly conservative at 1.0).
    let cjk_tokens = cjk;
    // Code correction: dense punctuation/symbols tokenize worse than prose.
    // Approximate by counting non-alphanumeric, non-space chars beyond 30%.
    let symbols = text
        .chars()
        .filter(|c| !c.is_alphanumeric() && !c.is_whitespace())
        .count();
    let correction = if other > 64 && symbols * 4 > other {
        (other - symbols).div_ceil(6) // denser packing for symbol runs
    } else {
        0
    };
    base.saturating_sub(correction) + cjk_tokens
}

/// A chat message as the providers see it.
#[derive(Debug, Clone)]
pub struct Message<'a> {
    pub role: &'a str,
    pub content: &'a str,
}

/// Per-message structural overhead (role tag, separators) observed across
/// OpenAI-compatible providers; small constant, counted conservatively.
const MESSAGE_OVERHEAD: usize = 4;

/// Estimate the prompt size of a full request before sending it.
pub fn estimate_messages(messages: &[Message]) -> usize {
    messages.iter().map(|m| estimate(m.content) + MESSAGE_OVERHEAD).sum::<usize>()
}

/// Estimate tokens for a tool definition block (name + description + schema).
pub fn estimate_tool_def(name: &str, description: &str, parameters_json: &str) -> usize {
    estimate(name) + estimate(description) + estimate(parameters_json) + MESSAGE_OVERHEAD
}

/// One calibration data point: a known-good token count for some text
/// (e.g. from real provider usage numbers).
#[derive(Debug, Clone)]
pub struct AccuracySample {
    pub text: String,
    pub actual_tokens: u32,
}

/// Mean absolute percentage error of [`estimate`] vs actual across samples.
///
/// Samples with zero actual tokens are skipped (percentage error undefined);
/// returns `None` when no usable samples remain.
pub fn estimator_error(samples: &[AccuracySample]) -> Option<f32> {
    let mut sum = 0.0f32;
    let mut count = 0u32;
    for s in samples {
        if s.actual_tokens == 0 {
            continue;
        }
        let err = (estimate(&s.text) as f32 - s.actual_tokens as f32).abs()
            / s.actual_tokens as f32;
        sum += err;
        count += 1;
    }
    if count == 0 { None } else { Some(sum / count as f32 * 100.0) }
}

/// The sample with the largest percentage error, as (text, error %).
pub fn worst_sample(samples: &[AccuracySample]) -> Option<(&str, f32)> {
    samples
        .iter()
        .filter(|s| s.actual_tokens > 0)
        .map(|s| {
            let err = (estimate(&s.text) as f32 - s.actual_tokens as f32).abs()
                / s.actual_tokens as f32
                * 100.0;
            (s.text.as_str(), err)
        })
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
}

/// One-line human summary of estimator accuracy: overall MAPE plus the worst
/// sample (first 30 chars) and its error.
pub fn estimate_accuracy_report(samples: &[(String, u32)]) -> String {
    let owned: Vec<AccuracySample> = samples
        .iter()
        .map(|(text, tokens)| AccuracySample { text: text.clone(), actual_tokens: *tokens })
        .collect();
    let Some(mape) = estimator_error(&owned) else {
        return "no samples".to_string();
    };
    // Same zero-filter as estimator_error, so a Some here implies a worst exists.
    let (worst_text, worst_err) =
        worst_sample(&owned).expect("usable samples imply a worst sample");
    let head: String = worst_text.chars().take(30).collect();
    format!("estimator MAPE {mape:.1}% (worst: {head}… {worst_err:.1}%)")
}

/// Budget verdict for a would-be request.
#[derive(Debug, PartialEq)]
pub enum Budget {
    /// Comfortably inside the limit.
    Ok { estimated: usize },
    /// Over the soft target but under the hard limit — trim history first.
    Trim { estimated: usize, over_by: usize },
    /// Would exceed even after trimming — must compact or refuse.
    Compact { estimated: usize, over_by: usize },
}

/// Classify a request against a model context budget.
///
/// `soft_target` is where we want steady-state requests to sit (leaving room
/// for the completion); `hard_limit` is the model's real context window.
pub fn check_budget(estimated: usize, soft_target: usize, hard_limit: usize) -> Budget {
    assert!(hard_limit >= soft_target, "soft target must be <= hard limit");
    if estimated <= soft_target {
        Budget::Ok { estimated }
    } else if estimated <= hard_limit {
        Budget::Trim { estimated, over_by: estimated - soft_target }
    } else {
        Budget::Compact { estimated, over_by: estimated - hard_limit }
    }
}

/// Cache hit rate as a fraction of total lookups.
///
/// Returns 0.0 when there were no lookups at all (nothing to be a ratio of),
/// and the result is clamped to [0, 1] so pathological inputs can't leak out
/// of range.
pub fn cache_hit_rate(hits: u64, misses: u64) -> f32 {
    let total = hits.saturating_add(misses);
    if total == 0 {
        return 0.0;
    }
    (hits as f32 / total as f32).clamp(0.0, 1.0)
}

/// Running tally of cache hits/misses with an on-demand hit-rate readout.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
}

impl CacheStats {
    pub fn record_hit(&mut self) {
        self.hits = self.hits.saturating_add(1);
    }

    pub fn record_miss(&mut self) {
        self.misses = self.misses.saturating_add(1);
    }

    pub fn hit_rate(&self) -> f32 {
        cache_hit_rate(self.hits, self.misses)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_tiny_inputs_are_zero_or_small() {
        assert_eq!(estimate(""), 0);
        assert_eq!(estimate("hi"), 1);
        assert_eq!(estimate("    "), 1);
    }

    #[test]
    fn english_prose_follows_chars_per_four() {
        // 400 ascii chars ≈ 100 tokens.
        let text = "word ".repeat(80); // 400 chars
        let n = estimate(&text);
        assert!((90..=110).contains(&n), "got {n}");
    }

    #[test]
    fn cjk_costs_about_one_token_per_char() {
        let text = "你好世界".repeat(25); // 100 CJK chars
        let n = estimate(&text);
        assert!((95..=105).contains(&n), "got {n}");
    }

    #[test]
    fn mixed_text_counts_both_parts() {
        // 200 ascii + 50 CJK ≈ 50 + 50 = 100.
        let text = format!("{}{}", "a".repeat(200), "世".repeat(50));
        let n = estimate(&text);
        assert!((85..=115).contains(&n), "got {n}");
    }

    #[test]
    fn messages_include_structural_overhead() {
        let msgs = vec![Message { role: "user", content: "hello world" }];
        let n = estimate_messages(&msgs);
        assert!(n > estimate("hello world"), "overhead missing");
        assert_eq!(n, estimate("hello world") + MESSAGE_OVERHEAD);
    }

    #[test]
    fn budget_classification_matches_spec() {
        assert_eq!(check_budget(100, 200, 400), Budget::Ok { estimated: 100 });
        assert_eq!(
            check_budget(300, 200, 400),
            Budget::Trim { estimated: 300, over_by: 100 }
        );
        assert_eq!(
            check_budget(500, 200, 400),
            Budget::Compact { estimated: 500, over_by: 100 }
        );
    }

    #[test]
    fn estimator_is_monotonic() {
        let short = "some reasonably sized input text";
        let long = short.repeat(10);
        assert!(estimate(&long) > estimate(short));
    }

    #[test]
    fn large_input_estimates_fast_enough_for_budget_checks() {
        // 1 MiB of mixed content must estimate in well under a millisecond-ish
        // scale (no regex, single pass). We just assert it completes quickly
        // relative to a generous bound.
        let text = "code fn() {} // comment 你好 ".repeat(30_000); // ~870 KB
        let start = std::time::Instant::now();
        let _ = estimate(&text);
        assert!(start.elapsed().as_millis() < 100, "too slow");
    }

    #[test]
    fn cache_hit_rate_zero_total_is_zero() {
        assert_eq!(cache_hit_rate(0, 0), 0.0);
    }

    #[test]
    fn cache_hit_rate_is_ratio_with_tolerance() {
        // 1 hit out of 3 lookups.
        let rate = cache_hit_rate(1, 2);
        assert!((rate - 1.0 / 3.0).abs() < 1e-6, "got {rate}");
    }

    #[test]
    fn cache_stats_accumulates_hits_and_misses() {
        let mut stats = CacheStats::default();
        assert_eq!(stats, CacheStats { hits: 0, misses: 0 });
        stats.record_hit();
        stats.record_hit();
        stats.record_miss();
        assert_eq!(stats.hits, 2);
        assert_eq!(stats.misses, 1);
        let rate = stats.hit_rate();
        assert!((rate - 2.0 / 3.0).abs() < 1e-6, "got {rate}");
    }

    #[test]
    fn cache_hit_rate_all_hits_is_one() {
        assert_eq!(cache_hit_rate(5, 0), 1.0);
    }

    fn sample(text: &str, actual_tokens: u32) -> AccuracySample {
        AccuracySample { text: text.to_string(), actual_tokens }
    }

    #[test]
    fn estimator_error_empty_is_none() {
        assert_eq!(estimator_error(&[]), None);
        assert_eq!(worst_sample(&[]), None);
    }

    #[test]
    fn estimator_error_perfect_is_zero() {
        let samples = vec![sample("hello world", estimate("hello world") as u32)];
        assert_eq!(estimator_error(&samples), Some(0.0));
    }

    #[test]
    fn estimator_error_averages_known_errors() {
        // estimate("hello world") = 3. Craft samples with known error:
        // actual 6 → |3-6|/6 = 50%, actual 12 → 75%. Mean of (50, 75) = 62.5%.
        let samples = vec![sample("hello world", 6), sample("hello world", 12)];
        let err = estimator_error(&samples).unwrap();
        assert!((err - 62.5).abs() < 1e-4, "got {err}");
    }

    #[test]
    fn zero_actual_samples_are_skipped() {
        // Only a zero-actual sample → no usable data.
        assert_eq!(estimator_error(&[sample("hello", 0)]), None);
        assert_eq!(worst_sample(&[sample("hello", 0)]), None);
        // Mixed: zero-actual ignored, others averaged.
        let samples = vec![sample("hello world", 0), sample("hello world", 6)];
        let err = estimator_error(&samples).unwrap();
        assert!((err - 50.0).abs() < 1e-4, "got {err}");
    }

    #[test]
    fn worst_sample_returns_largest_error() {
        // estimate("hello world") = 3: error vs 12 is 75%, vs 6 is 50% → worst is 12.
        let samples = vec![sample("hello world", 12), sample("hello world", 6)];
        let (text, err) = worst_sample(&samples).unwrap();
        assert_eq!(text, "hello world");
        assert!((err - 75.0).abs() < 1e-4, "got {err}");
    }

    #[test]
    fn accuracy_report_with_samples_contains_mape_and_worst() {
        // Same known errors as above: MAPE 62.5%, worst sample error 75%.
        let samples = vec![
            ("hello world".to_string(), 12u32),
            ("hello world".to_string(), 6),
        ];
        let report = estimate_accuracy_report(&samples);
        assert!(report.contains("62.5"), "got {report}");
        assert!(report.contains("75.0"), "got {report}");
        assert!(report.contains("worst: hello world"), "got {report}");
    }

    #[test]
    fn accuracy_report_empty_says_no_samples() {
        assert_eq!(estimate_accuracy_report(&[]), "no samples");
    }
}