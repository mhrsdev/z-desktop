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
}