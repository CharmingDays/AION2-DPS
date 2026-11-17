//! The app's data folder and crash-safe file writes.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Copies of unreadable files kept per file (`<name>.corrupt-<unix secs>`).
pub const KEEP_CORRUPT: usize = 3;

/// `%LOCALAPPDATA%\aion2dps`, or the temp directory when that variable is missing.
pub fn folder() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("aion2dps")
}

/// Write next to `path`, flush it to disk and rename it over the old file,
/// so a crash mid-write leaves either the old file or the new one, never a
/// partial one. On Windows `std::fs::rename` replaces an existing file
/// (`MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`, or a POSIX-style rename).
/// Each write uses its own temp name, so two saves of the same file at once
/// cannot mix their bytes.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)?;
        }
    }
    let temp = unique_temp_path(path);
    let written = (|| {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()
    })();
    let result = written.and_then(|()| rename_replacing(&temp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

/// Rename over `to`. A virus scanner or indexer holding the old file open
/// for a moment makes Windows refuse with "access denied", so that is
/// retried a few times before giving up.
fn rename_replacing(from: &Path, to: &Path) -> io::Result<()> {
    let mut tries = 0;
    loop {
        match std::fs::rename(from, to) {
            Err(err) if err.kind() == io::ErrorKind::PermissionDenied && tries < 4 => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(25 * tries));
            }
            other => return other,
        }
    }
}

/// `<name>.tmp`, the temp name used by callers that write their own temp file.
pub fn temp_path(path: &Path) -> PathBuf {
    with_suffix(path, ".tmp")
}

/// `<name>.tmp-<pid>-<n>`, so concurrent writes never share a temp file.
fn unique_temp_path(path: &Path) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    with_suffix(path, &format!(".tmp-{}-{n}", std::process::id()))
}

pub fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// Move a file that could not be read aside as `<name>.corrupt-<unix secs>`
/// (copy when the move fails), so the next save does not destroy it, and
/// delete all but the newest [`KEEP_CORRUPT`] such copies. Returns where the
/// copy went.
pub fn keep_corrupt(path: &Path) -> Option<PathBuf> {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut kept = with_suffix(path, &format!(".corrupt-{secs:010}"));
    let mut n = 1;
    while kept.exists() {
        kept = with_suffix(path, &format!(".corrupt-{secs:010}-{n}"));
        n += 1;
    }
    let moved = std::fs::rename(path, &kept).is_ok() || std::fs::copy(path, &kept).is_ok();
    prune_corrupt(path, KEEP_CORRUPT);
    if moved {
        tracing::warn!("{} could not be read; kept it as {}", path.display(), kept.display());
        Some(kept)
    } else {
        tracing::warn!("{} could not be read, and a copy could not be kept", path.display());
        None
    }
}

/// The `<name>.corrupt-*` copies next to `path`, oldest first.
pub fn corrupt_copies(path: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return Vec::new();
    };
    let dir = if dir.as_os_str().is_empty() { Path::new(".") } else { dir };
    let prefix = format!("{}.corrupt-", name.to_string_lossy());
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
                .map(|entry| entry.path())
                .collect()
        })
        .unwrap_or_default();
    // Fixed-width seconds, so name order is age order.
    found.sort();
    found
}

fn prune_corrupt(path: &Path, keep: usize) {
    let copies = corrupt_copies(path);
    let extra = copies.len().saturating_sub(keep);
    for old in &copies[..extra] {
        let _ = std::fs::remove_file(old);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aion2dps-storage-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn an_atomic_write_replaces_the_old_file_and_leaves_no_temp() {
        let dir = temp_dir("atomic");
        let path = dir.join("settings.json");
        write_atomic(&path, b"{\"a\":1}").unwrap();
        write_atomic(&path, b"{\"a\":2}").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"{\"a\":2}");
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(Result::ok).collect();
        assert_eq!(left.len(), 1, "only the file itself is left");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_file_is_kept_aside_and_only_the_newest_copies_stay() {
        let dir = temp_dir("corrupt");
        let path = dir.join("alerts.json");
        for old in ["0000000001", "0000000002", "0000000003"] {
            std::fs::write(with_suffix(&path, &format!(".corrupt-{old}")), "old").unwrap();
        }
        std::fs::write(&path, "{not json").unwrap();
        let kept = keep_corrupt(&path).expect("a copy is kept");
        assert!(!path.exists(), "the damaged file is moved aside");
        assert_eq!(std::fs::read_to_string(&kept).unwrap(), "{not json");
        let copies = corrupt_copies(&path);
        assert_eq!(copies.len(), KEEP_CORRUPT);
        assert!(copies.contains(&kept));
        assert!(!copies.iter().any(|p| p.to_string_lossy().ends_with("corrupt-0000000001")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
