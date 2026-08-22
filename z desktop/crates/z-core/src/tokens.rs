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
}