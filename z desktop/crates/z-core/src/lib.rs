//! Z Desktop Agent Core — headless, UI-independent.
//!
//! The core owns threads, turns, tools, providers, the repository index and
//! session persistence. It talks to clients exclusively through the
//! [`z_protocol`] contract: commands in, events out. It never depends on a UI
//! crate, so the same binary can later serve a CLI or a remote client.

pub mod journal;
pub mod provider;
pub mod redact;
pub mod sandbox;
pub mod tokens;
pub mod repo;
pub mod runtime;
pub mod tools;
pub mod fingerprint;
pub mod atomic_write;

/// Monotonic, collision-free-enough identifiers for a single-user local app:
/// millisecond timestamp plus a process-wide counter.
pub fn new_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{prefix}-{ms:x}-{n:x}")
}

#[cfg(test)]
mod tests {
    #[test]
    fn ids_are_prefixed_and_unique() {
        let a = super::new_id("turn");
        let b = super::new_id("turn");
        assert!(a.starts_with("turn-") && b.starts_with("turn-"));
        assert_ne!(a, b);
    }
}