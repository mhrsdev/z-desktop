//! Content fingerprints for the safe-editing engine (edit-001).
//!
//! fs_read records a fingerprint per file per thread; fs_write refuses a
//! stale write when the on-disk fingerprint differs from what this thread
//! last read (ZD-E-0060). FNV-1a is chosen over CRC32 for wider spread and
//! over SHA-2 because collision *accidents* are what we defend against, not
//! adversaries — an attacker who can craft contents is out of scope here
//! (they could just write via terminal_exec anyway).

use std::path::Path;

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x00000100000001b3;

/// FNV-1a 64-bit over bytes.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Fingerprint a file by streaming it in 8 KiB chunks so multi-hundred-MB
/// files never load into memory.
pub fn file_fingerprint(path: &Path) -> Result<u64, String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hash = FNV_OFFSET;
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        for &b in &buf[..n] {
            hash ^= b as u64;
            hash = hash.wrapping_mul(FNV_PRIME);
        }
    }
    Ok(hash)
}

// ponytail: registry is unbounded and lives forever; at personal scale a
// session touches hundreds of files, so this is fine. Add LRU eviction if
// long-running sessions with 100k+ reads show memory pressure.
use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::Mutex;

type Registry = Mutex<HashMap<(String, String), u64>>;

fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Record the fingerprint `thread_id` last observed at `path`.
pub fn record_fingerprint(thread_id: &str, path: &str, fp: u64) {
    registry().lock().unwrap().insert((thread_id.into(), path.into()), fp);
}

/// Take (remove) the recorded fingerprint for this thread+path, if any.
/// Taking (not peeking) means each read-write cycle re-arms naturally.
pub fn take_fingerprint(thread_id: &str, path: &str) -> Option<u64> {
    registry().lock().unwrap().remove(&(thread_id.into(), path.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors_match_the_fnv1a64_spec() {
        assert_eq!(fnv1a64(b""), 0xcbf29ce484222325);
        assert_eq!(fnv1a64(b"a"), 0xaf63dc4c8601ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x85944171f73967e8);
    }

    #[test]
    fn streaming_fingerprint_matches_in_memory_hash_and_missing_files_err() {
        let dir = std::env::temp_dir().join(format!("zdt-fp-{:x}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("sample.txt");
        std::fs::write(&path, b"foobar").unwrap();

        assert_eq!(file_fingerprint(&path).unwrap(), fnv1a64(b"foobar"));
        assert!(file_fingerprint(&dir.join("nope.txt")).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn registry_records_and_takes_per_thread_path() {
        record_fingerprint("t-reg", "a.txt", 42);
        assert_eq!(take_fingerprint("t-reg", "a.txt"), Some(42));
        // Taking removes it.
        assert_eq!(take_fingerprint("t-reg", "a.txt"), None);
        // Same path, different thread stays independent.
        record_fingerprint("t1", "b.txt", 1);
        record_fingerprint("t2", "b.txt", 2);
        assert_eq!(take_fingerprint("t1", "b.txt"), Some(1));
        assert_eq!(take_fingerprint("t2", "b.txt"), Some(2));
    }
}
