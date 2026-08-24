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

/// Write `contents` only when they differ from what's on disk.
/// Returns `Ok(false)` if `path` already holds exactly these bytes (no write,
/// no mtime churn); otherwise performs an atomic write and returns `Ok(true)`.
pub fn write_if_changed(path: &Path, contents: &[u8]) -> Result<bool, String> {
    match std::fs::read(path) {
        Ok(existing) if existing == contents => Ok(false),
        // Missing file or differing bytes: (re)write atomically.
        _ => atomic_write(path, contents).map(|()| true),
    }
}

/// Like [`write_if_changed`] but keeps a single rolling `.bak` of the previous
/// contents (edit-005). Before overwriting an existing, differing file, the old
/// bytes are copied to `<name>.<ext>.bak` (or `<name>.bak` when extensionless);
/// each change overwrites the prior backup. A missing target is written plain
/// with no backup; identical contents still skip without refreshing the backup.
pub fn write_with_backup(path: &Path, contents: &[u8]) -> Result<bool, String> {
    let backup = match path.extension() {
        Some(ext) => {
            let mut e = ext.to_os_string();
            e.push(".bak");
            path.with_extension(e)
        }
        None => {
            let mut p = path.as_os_str().to_os_string();
            p.push(".bak");
            std::path::PathBuf::from(p)
        }
    };
    match std::fs::read(path) {
        Ok(existing) if existing == contents => Ok(false),
        Ok(_) => {
            std::fs::copy(path, &backup).map_err(|e| format!("backup write failed: {e}"))?;
            atomic_write(path, contents).map(|()| true)
        }
        Err(_) => atomic_write(path, contents).map(|()| true),
    }
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

    fn mtime_secs(p: &Path) -> std::time::SystemTime {
        std::fs::metadata(p).unwrap().modified().unwrap()
    }

    #[test]
    fn write_if_changed_skips_identical_content() {
        let dir = temp_dir("wic-same");
        let target = dir.join("same.txt");
        atomic_write(&target, b"stable").unwrap();
        // Ensure any coarse-mtime filesystem would still show a difference
        // if a write had actually happened.
        std::thread::sleep(std::time::Duration::from_millis(50));
        let before = mtime_secs(&target);

        assert_eq!(write_if_changed(&target, b"stable").unwrap(), false);
        assert_eq!(std::fs::read(&target).unwrap(), b"stable");
        assert_eq!(mtime_secs(&target), before, "skip must not touch the file");
    }

    #[test]
    fn write_if_changed_writes_when_content_differs() {
        let dir = temp_dir("wic-diff");
        let target = dir.join("diff.txt");
        std::fs::write(&target, b"old").unwrap();

        assert_eq!(write_if_changed(&target, b"new bytes").unwrap(), true);
        assert_eq!(std::fs::read(&target).unwrap(), b"new bytes");

        // And a second call with the now-current content skips.
        assert_eq!(write_if_changed(&target, b"new bytes").unwrap(), false);
    }

    #[test]
    fn write_if_changed_writes_missing_file() {
        let dir = temp_dir("wic-missing");
        let target = dir.join("absent.txt");

        assert_eq!(write_if_changed(&target, b"created").unwrap(), true);
        assert_eq!(std::fs::read(&target).unwrap(), b"created");
    }

    #[test]
    fn write_if_changed_skips_empty_vs_empty() {
        let dir = temp_dir("wic-empty");
        let target = dir.join("empty.txt");
        std::fs::write(&target, b"").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        let before = mtime_secs(&target);

        assert_eq!(write_if_changed(&target, b"").unwrap(), false);
        assert!(std::fs::read(&target).unwrap().is_empty());
        assert_eq!(mtime_secs(&target), before);
    }

    #[test]
    fn backup_created_on_change_and_holds_previous_content() {
        let dir = temp_dir("wb-change");
        let target = dir.join("cfg.toml");
        std::fs::write(&target, b"v1").unwrap();
        let bak = dir.join("cfg.toml.bak");

        assert!(!bak.exists());
        assert_eq!(write_with_backup(&target, b"v2").unwrap(), true);
        assert_eq!(std::fs::read(&target).unwrap(), b"v2");
        assert_eq!(
            std::fs::read(&bak).unwrap(),
            b"v1",
            ".bak must hold previous content"
        );

        // Rolling: a second change refreshes the single .bak.
        assert_eq!(write_with_backup(&target, b"v3").unwrap(), true);
        assert_eq!(std::fs::read(&bak).unwrap(), b"v2");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
    }

    #[test]
    fn backup_identical_content_skips_without_refreshing_backup() {
        let dir = temp_dir("wb-same");
        let target = dir.join("same.txt");
        let bak = dir.join("same.txt.bak");
        std::fs::write(&target, b"older").unwrap();
        // Change once: bak now holds "older", target holds "current".
        assert_eq!(write_with_backup(&target, b"current").unwrap(), true);
        assert_eq!(std::fs::read(&bak).unwrap(), b"older");

        // Identical-content call must skip without refreshing the backup.
        std::thread::sleep(std::time::Duration::from_millis(50));
        let before = mtime_secs(&bak);

        assert_eq!(write_with_backup(&target, b"current").unwrap(), false);
        assert_eq!(std::fs::read(&target).unwrap(), b"current");
        assert_eq!(std::fs::read(&bak).unwrap(), b"older");
        assert_eq!(mtime_secs(&bak), before, "skip must not refresh the backup");
    }

    #[test]
    fn backup_first_write_makes_no_bak() {
        let dir = temp_dir("wb-first");
        let target = dir.join("fresh.txt");

        assert_eq!(write_with_backup(&target, b"created").unwrap(), true);
        assert_eq!(std::fs::read(&target).unwrap(), b"created");
        assert!(
            !dir.join("fresh.txt.bak").exists(),
            "missing target must not produce a backup"
        );
    }
}
