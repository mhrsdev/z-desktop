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

/// Peek (without removing) the recorded fingerprint for this thread+path.
/// tok-020 uses this to notice redundant unchanged re-reads.
pub fn peek_fingerprint(thread_id: &str, path: &str) -> Option<u64> {
    registry().lock().unwrap().get(&(thread_id.into(), path.into())).copied()
}

/// edit-018: blind-write detection. A write is *blind* when this thread never
/// recorded a fingerprint for the path (never read it), so edit-001's stale
/// comparison has nothing to compare against — refuse before touching disk.
/// Stale-*content* refusal remains fs_write/edit-003's separate job.
pub struct WriteCheck {
    pub allowed: bool,
    pub reason: &'static str,
}

/// Gate a write by `thread_id` at `path`: allowed iff some fingerprint was
/// previously recorded for that thread+path pair.
pub fn blind_write_check(thread_id: &str, path: &Path) -> WriteCheck {
    if peek_fingerprint(thread_id, &path.to_string_lossy()).is_some() {
        WriteCheck { allowed: true, reason: "fingerprint on record" }
    } else {
        WriteCheck {
            allowed: false,
            reason: "no recorded fingerprint for this thread+path; read the file before writing",
        }
    }
}

/// ctx-007: diff this thread's recorded reads against disk right now.
/// Returns the raw registry paths under `root` whose CURRENT fingerprint
/// differs from what `thread_id` last read (pass the same root shape the
/// paths were recorded with). Unchanged files are skipped; files that
/// vanished or fail to read are skipped too — a missing file is not
/// "changed content" and the read tool surfaces ENOENT on its own.
pub fn stale_reads(thread_id: &str, root: &Path) -> Vec<String> {
    // Snapshot first, drop the guard before any file I/O (lock-order pitfall).
    let snapshot: Vec<(String, u64)> = registry()
        .lock()
        .unwrap()
        .iter()
        .filter(|((t, p), _)| t == thread_id && Path::new(p).starts_with(root))
        .map(|((_, p), fp)| (p.clone(), *fp))
        .collect();
    snapshot
        .into_iter()
        .filter_map(|(path, recorded)| match file_fingerprint(Path::new(&path)) {
            Ok(current) if current != recorded => Some(path),
            _ => None,
        })
        .collect()
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

    #[test]
    fn blind_write_check_blocks_unrecorded_and_allows_recorded_per_thread() {
        let base = format!("t-blind-{:x}", std::process::id());
        let path = std::env::temp_dir().join(format!("zdt-blind-{:x}.txt", std::process::id()));
        let path_str = path.to_string_lossy().to_string();

        // Unrecorded => blocked, reason names the missing fingerprint.
        let chk = blind_write_check(&base, &path);
        assert!(!chk.allowed);
        assert!(chk.reason.contains("no recorded fingerprint"));

        // Recorded => allowed.
        record_fingerprint(&base, &path_str, 99);
        assert!(blind_write_check(&base, &path).allowed);

        // Same path, different thread stays blind.
        let other = format!("{base}-other");
        let chk = blind_write_check(&other, &path);
        assert!(!chk.allowed);
        assert!(chk.reason.contains("no recorded fingerprint"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn stale_reads_flags_changed_and_skips_unchanged_missing_and_out_of_root() {
        let thread = format!("t-stale-{:x}", std::process::id());
        let root = std::env::temp_dir().join(format!("zdt-stale-{:x}", std::process::id()));
        let _ = std::fs::create_dir_all(&root);
        let changed = root.join("changed.txt");
        let same = root.join("same.txt");
        std::fs::write(&changed, b"v1").unwrap();
        std::fs::write(&same, b"keep").unwrap();

        // Real flow: record what the thread read, then disk moves on.
        let changed_fp = file_fingerprint(&changed).unwrap();
        record_fingerprint(&thread, &changed.to_string_lossy(), changed_fp);
        std::fs::write(&changed, b"v2").unwrap();
        let same_fp = file_fingerprint(&same).unwrap();
        record_fingerprint(&thread, &same.to_string_lossy(), same_fp);
        // Recorded but vanished since — skipped, not "stale".
        record_fingerprint(&thread, &root.join("gone.txt").to_string_lossy(), 7);
        // Wrong fingerprint OUTSIDE root — proves root scoping, not just luck.
        let outside =
            std::env::temp_dir().join(format!("zdt-stale-out-{:x}.txt", std::process::id()));
        std::fs::write(&outside, b"outside").unwrap();
        record_fingerprint(&thread, &outside.to_string_lossy(), 12345);

        let stale = stale_reads(&thread, &root);
        assert_eq!(
            stale,
            vec![changed.to_string_lossy().to_string()],
            "only the genuinely changed in-root path is flagged"
        );

        // Another thread's view of the SAME changed file is independent.
        assert!(stale_reads("no-such-thread", &root).is_empty());

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&outside);
    }

    /// edit-007: a writer hammering atomic_write while 4 readers fingerprint
    /// the same path must never surface torn content — every read hashes to
    /// exactly the old or new full payload (rename is all-or-nothing).
    #[test]
    fn concurrent_readers_never_see_torn_content_during_atomic_writes() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        let dir = std::env::temp_dir().join(format!("zdt-fp-conc-{:x}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("torn.txt");

        // Different lengths: any mixed-prefix/suffix read changes the length,
        // so a torn hash can't collide with a whole-payload hash by accident.
        let old = b"OLD-CONTENT-v1";
        let new = b"NEW-MUCH-LONGER-CONTENT-v2-that-differs";
        let old_fp = fnv1a64(old);
        let new_fp = fnv1a64(new);

        // Seed so readers never race a missing file; only torn content.
        std::fs::write(&path, old).unwrap();

        let done = Arc::new(AtomicBool::new(false));
        let stop = done.clone();

        let writer_path = path.clone();
        let writer = std::thread::spawn(move || {
            let mut writes = 0u64;
            while !stop.load(Ordering::Relaxed) {
                let payload: &[u8] = if writes % 2 == 0 { new } else { old };
                crate::atomic_write::atomic_write(&writer_path, payload).unwrap();
                writes += 1;
            }
            writes
        });

        let mut readers = Vec::new();
        for r in 0..4 {
            let thread_id = format!("t-conc-{r}");
            record_fingerprint(&thread_id, "conc.txt", 7);
            let reader_path = path.clone();
            readers.push(std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_millis(200);
                let mut reads = 0u64;
                while Instant::now() < deadline {
                    // Registry peek under the same contention load.
                    assert_eq!(peek_fingerprint(&thread_id, "conc.txt"), Some(7));
                    let fp = file_fingerprint(&reader_path)
                        .unwrap_or_else(|e| panic!("reader {r}: read failed mid-write: {e}"));
                    assert!(
                        fp == old_fp || fp == new_fp,
                        "reader {r} saw torn content after {reads} reads: {fp:#x}"
                    );
                    reads += 1;
                }
                reads
            }));
        }

        std::thread::sleep(Duration::from_millis(200));
        done.store(true, Ordering::Relaxed);

        let flips = writer.join().expect("writer panicked");
        let mut total = 0;
        for h in readers {
            total += h.join().expect("reader panicked");
        }
        assert!(flips > 0 && total > 0, "test must actually exercise the race");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
