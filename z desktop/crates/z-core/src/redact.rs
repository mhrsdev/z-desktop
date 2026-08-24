//! Secret redaction — one filter applied to every surface that can carry
//! text out of the process: tool output, logs, traces, persisted events.
//!
//! Design rules:
//! - Fail open for *detection* (a missed pattern is a bug, not a crash) but
//!   the replacement is unconditional once matched.
//! - Keep a short fingerprint (`[redacted:sk-…xY]`) so humans debugging can
//!   tell WHICH secret leaked where, without recovering it.
//! - Patterns are prefix-anchored to known token formats first (low false
//!   positives); generic key=value assignment patterns run last.

use regex::Regex;
use std::sync::OnceLock;

/// A named pattern with its replacement label.
struct Rule {
    regex: Regex,
    /// Shown inside `[redacted:<label>…]`.
    label: &'static str,
}

fn rules() -> &'static Vec<Rule> {
    static RULES: OnceLock<Vec<Rule>> = OnceLock::new();
    RULES.get_or_init(|| {
        let raw: &[(&str, &str)] = &[
            // Known provider token formats.
            (r"sk-ant-[A-Za-z0-9_\-]{20,}", "sk-ant"),
            (r"sk-[A-Za-z0-9_\-]{20,}", "sk"),
            (r"xai-[A-Za-z0-9]{20,}", "xai"),
            (r"gh[pousr]_[A-Za-z0-9]{36,}", "gh"),
            (r"AKIA[0-9A-Z]{16}", "aws"),
            (r"AIza[0-9A-Za-z_\-]{35}", "gcp"),
            // Authorization headers / bearer tokens (group 1 = visible prefix).
            (r"(?i)(bearer\s+)[A-Za-z0-9._~+/=-]{16,}", "bearer"),
            // JSON / env-style assignments of sensitive keys.
            (
                r#"(?i)("?(?:api[_-]?key|apikey|secret|token|password|authorization)"?\s*[:=]\s*"?)[^",\s}]{8,}"#,
                "assign",
            ),
        ];
        raw.iter()
            .filter_map(|(pattern, label)| {
                Regex::new(pattern).ok().map(|regex| Rule { regex, label })
            })
            .collect()
    })
}

/// Replace every recognised secret in `input` with a fingerprinted marker.
pub fn redact(input: &str) -> String {
    let mut text = input.to_string();
    for rule in rules() {
        text = rule
            .regex
            .replace_all(&text, |caps: &regex::Captures| {
                let whole = caps.get(0).map(|m| m.as_str()).unwrap_or("");
                // For assignment rules keep the `key=` part visible.
                let value_part = if caps.len() > 1 {
                    let prefix_len = caps.get(1).map(|m| m.end()).unwrap_or(0);
                    &whole[prefix_len..]
                } else {
                    whole
                };
                let fingerprint = fingerprint(value_part);
                if caps.len() > 1 {
                    format!("{}[redacted:{}…{}]", &whole[..whole.len() - value_part.len()], rule.label, fingerprint)
                } else {
                    format!("[redacted:{}…{}]", rule.label, fingerprint)
                }
            })
            .to_string();
    }
    text
}

/// First 2 + last 2 chars of the secret — enough to correlate, useless to recover.
fn fingerprint(secret: &str) -> String {
    let chars: Vec<char> = secret.chars().collect();
    if chars.len() <= 4 {
        return "*".repeat(chars.len());
    }
    format!("{}{}", chars[..2].iter().collect::<String>(), chars[chars.len() - 2..].iter().collect::<String>())
}

/// Observability counters for redaction activity.
///
/// Pure counters only — deliberately NOT wired into [`redact`] / the scan
/// path yet. Integration into tools.rs (recording scans/hits per surface)
/// is a later change; this is just the shared counter surface.
#[derive(Debug, Clone, PartialEq)]
pub struct RedactionReport {
    /// Number of times a text buffer was scanned.
    pub total_scans: u64,
    /// Per-kind hit counts, insertion-ordered by first sighting.
    pub hits_by_kind: Vec<(String, u64)>,
    /// Wall-clock ms of the most recent hit, if any.
    pub last_hit_at_ms: Option<u128>,
}

impl Default for RedactionReport {
    fn default() -> Self {
        Self { total_scans: 0, hits_by_kind: Vec::new(), last_hit_at_ms: None }
    }
}

/// Thread-safe accumulator of [`RedactionReport`] snapshots.
#[derive(Debug, Default)]
pub struct RedactionStats {
    inner: std::sync::Mutex<RedactionReport>,
}

impl RedactionStats {
    /// Record one completed scan pass over some text buffer.
    pub fn record_scan(&self) {
        if let Ok(mut r) = self.inner.lock() {
            r.total_scans += 1;
        }
    }

    /// Record one hit of `kind` (rule label, e.g. "sk", "bearer") at `ts_ms`.
    pub fn record_hit(&self, kind: &str, ts_ms: u128) {
        if let Ok(mut r) = self.inner.lock() {
            match r.hits_by_kind.iter_mut().find(|(k, _)| k == kind) {
                Some((_, n)) => *n += 1,
                None => r.hits_by_kind.push((kind.to_string(), 1)),
            }
            r.last_hit_at_ms = Some(ts_ms);
        }
    }

    /// Point-in-time copy; mutating it never affects the accumulator.
    pub fn snapshot(&self) -> RedactionReport {
        self.inner.lock().map(|r| r.clone()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_provider_tokens_are_redacted_with_fingerprints() {
        let out = redact("key is sk-proj-abcdefghij0123456789 done");
        assert!(out.contains("[redacted:sk…sk89]"), "{out}");
        assert!(!out.contains("abcdefghij"));

        let out = redact("token xai-abcdefghijklmnopqrst here");
        assert!(out.contains("[redacted:xai…xast]"), "{out}");
        assert!(!out.contains("cdefghijklmn"), "{out}");
    }

    #[test]
    fn github_aws_google_formats_are_redacted() {
        assert!(redact("ghp_0123456789abcdefghijklmnopqrstuvwxyz").contains("[redacted:"));
        assert!(redact("id AKIAIOSFODNN7EXAMPLE end").contains("[redacted:aws"));
        assert!(redact("k AIzaSyA1234567890abcdefghijklmnopqrstuv").contains("[redacted:gcp"));
    }

    #[test]
    fn bearer_headers_and_assignments_keep_their_prefix() {
        let out = redact("Authorization: Bearer abcdef0123456789abcdef");
        assert!(out.contains("Bearer"), "{out}");
        assert!(out.contains("[redacted:"), "{out}");

        let out = redact(r#"{"api_key": "supersecretvalue123"}"#);
        assert!(out.contains(r#""api_key": ""#), "{out}");
        assert!(!out.contains("supersecretvalue"), "{out}");
    }

    #[test]
    fn ordinary_text_passes_through_untouched() {
        let src = "fn main() { println!(42); } // no secrets here";
        assert_eq!(redact(src), src);
    }

    #[test]
    fn short_strings_near_patterns_do_not_panic() {
        assert_eq!(redact(""), "");
        assert_eq!(redact("sk-"), "sk-");
        assert_eq!(redact("abcd"), "abcd");
    }

    #[test]
    fn empty_stats_default_to_zeroed_report() {
        let stats = RedactionStats::default();
        let snap = stats.snapshot();
        assert_eq!(snap.total_scans, 0);
        assert!(snap.hits_by_kind.is_empty());
        assert_eq!(snap.last_hit_at_ms, None);
    }

    #[test]
    fn hits_are_counted_per_kind() {
        let stats = RedactionStats::default();
        stats.record_scan();
        stats.record_scan();
        stats.record_scan();
        stats.record_hit("sk", 100);
        stats.record_hit("sk", 150);
        stats.record_hit("bearer", 175);

        let snap = stats.snapshot();
        assert_eq!(snap.total_scans, 3);
        assert_eq!(
            snap.hits_by_kind,
            vec![("sk".to_string(), 2), ("bearer".to_string(), 1)]
        );
        assert_eq!(snap.last_hit_at_ms, Some(175));
    }

    #[test]
    fn snapshot_is_an_isolated_copy() {
        let stats = RedactionStats::default();
        stats.record_hit("sk", 42);
        let mut snap = stats.snapshot();

        // Mutating further state and the returned copy leaves both independent.
        stats.record_hit("sk", 99);
        snap.hits_by_kind.push(("ghost".to_string(), 7));

        assert_eq!(stats.snapshot().hits_by_kind, vec![("sk".to_string(), 2)]);
        assert_eq!(stats.snapshot().last_hit_at_ms, Some(99));
        assert_eq!(snap.last_hit_at_ms, Some(42));
    }
}