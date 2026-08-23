//! Atomic file writes (ADR-0010 §(1b), edit-004).
//!
//! Crash/kill mid-write must leave the target old-or-new, never partial or
//! empty. Ordering is the contract: write temp in the target's own directory
//! (same-volume rename by construction) → fsync the temp → rename over the
//! target → best-effort parent-dir sync on POSIX so the rename itself tends
//! to survive power loss.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Write `contents` to `path` atomically via same-directory temp + rename.
pub fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), String> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("target");
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    // pid+counter keeps concurrent writers (and prior crashed runs) from
    // colliding on one temp name inside the target directory.
    let temp = parent.join(format!(".{name}.{}.{}.tmp", std::process::id(), n));

    let fail = |temp: &Path, e: String| -> String {
        let _ = std::fs::remove_file(temp); // never leak a half-written temp
        e
    };

    let write_result = (|| {
        use std::io::Write;
        let mut f = std::fs::File::create(&temp).map_err(|e| e.to_string())?;
        f.write_all(contents).map_err(|e| e.to_string())?;
        // fsync BEFORE rename; otherwise a crash window exists where the
        // rename is durable but the bytes behind it are not (zero-length file).
        f.sync_all().map_err(|e| e.to_string())
    })();
    if let Err(e) = write_result {
        return Err(fail(&temp, e));
    }

    let mut last_err = None;
    for attempt in 0..5u32 {
        match std::fs::rename(&temp, path) {
            Ok(()) => {
                last_err = None;
                break;
            }
            Err(e) => {
                last_err = Some(e.to_string());
                // Windows only: an antivirus/indexer holding the target open
                // causes transient sharing violations at rename time; brief
                // backoff then give up with an actionable error. POSIX renames
                // have no such failure mode — retrying there would just stall.
                if cfg!(windows) && attempt < 4 {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                } else {
                    break;
                }
            }
        }
    }
    if let Some(e) = last_err {
        return Err(fail(&temp, format!("rename into place failed: {e}")));
    }

    #[cfg(unix)]
    // Best effort: syncing the parent directory makes the rename itself
    // durable across power loss. Errors are ignored deliberately — some
    // filesystems refuse dir fsync and ADR-0010 accepts that degradation.
    if let Ok(dir) = std::fs::File::open(parent) {
        let _ = dir.sync_all();
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("zcore-aw-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn creates_a_fresh_file_with_exact_contents() {
        let dir = temp_dir("fresh");
        let target = dir.join("new.txt");
        atomic_write(&target, b"exact bytes\n").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"exact bytes\n");
    }

    #[test]
    fn overwrite_replaces_existing_contents() {
        let dir = temp_dir("over");
        let target = dir.join("file.txt");
        std::fs::write(&target, "old contents").unwrap();
        atomic_write(&target, b"new").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
    }

    #[test]
    fn concurrent_readers_see_old_or_new_never_partial() {
        let dir = temp_dir("race");
        let target = dir.join("race.txt");
        // Distinct, self-contained payloads a reader can match exactly.
        let versions: Vec<String> = (0..50)
            .map(|i| format!("version-{i:02}|{}", "x".repeat(400)))
            .collect();
        std::fs::write(&target, "seed").unwrap();

        use std::sync::atomic::AtomicBool;
        use std::sync::Arc;
        let stop = Arc::new(AtomicBool::new(false));
        let mut readers = Vec::new();
        for _ in 0..8 {
            let target = target.clone();
            let stop = stop.clone();
            let valid: Arc<Vec<String>> = {
                let mut all = vec!["seed".to_string()];
                all.extend(versions.iter().cloned());
                Arc::new(all)
            };
            readers.push(std::thread::spawn(move || {
                let mut reads = 0usize;
                while !stop.load(Ordering::Relaxed) {
                    let text = std::fs::read_to_string(&target).expect("read during writes");
                    assert!(
                        valid.iter().any(|v| *v == text),
                        "partial/torn read of {} bytes",
                        text.len()
                    );
                    reads += 1;
                }
                reads
            }));
        }
        for v in &versions {
            atomic_write(&target, v.as_bytes()).unwrap();
        }
        stop.store(true, Ordering::Relaxed);
        let total: usize = readers.into_iter().map(|h| h.join().unwrap()).sum();
        assert!(total > 0, "readers must have observed at least one state");
    }

    #[test]
    fn no_temp_files_remain_after_successful_writes() {
        let dir = temp_dir("cleanup");
        let target = dir.join("clean.txt");
        for i in 0..100 {
            atomic_write(&target, format!("value-{i}").as_bytes()).unwrap();
        }
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "value-99");
        let leftovers: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "leaked temp files: {leftovers:?}");
    }
}
