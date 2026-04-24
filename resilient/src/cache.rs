/// RES-355: incremental compilation — function-level bytecode cache.
///
/// The cache lives in `.resilient_cache/` next to the source file.
/// Each entry is a JSON file named `<sha256-of-source>.json` with a
/// small header that records the compiler version and the source hash.
/// On a cache hit (file exists, `compiler_version` matches) the
/// compiler skips re-parsing the source file.
///
/// Cache misses (file missing, or wrong `compiler_version`) fall through
/// to a normal compilation run.  Correctness is never affected by a
/// miss — the cache is a pure optimisation layer.
///
/// `--no-cache` bypasses both read and write so a single run is
/// isolated regardless of what is on disk.
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// A resolved cache decision.
#[derive(Debug, PartialEq)]
pub enum CacheResult {
    /// Source hash and compiler version both matched a stored entry.
    Hit,
    /// No valid entry was found; the caller must compile normally.
    Miss,
}

/// Write a new cache entry for `source_file`.
///
/// `cache_dir` is the `.resilient_cache/` directory (created if absent).
/// `source_hash` is the hex-encoded SHA-256 of the file's UTF-8 source
/// text.  Errors are silently swallowed — a write failure must never
/// prevent a successful compilation from completing.
pub fn write_entry(cache_dir: &Path, source_hash: &str) {
    if let Err(_e) = fs::create_dir_all(cache_dir) {
        return;
    }
    let path = entry_path(cache_dir, source_hash);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let entry = serde_json::json!({
        "compiler_version": env!("CARGO_PKG_VERSION"),
        "source_hash": source_hash,
        "cached_at": timestamp,
    });
    let _ = serde_json::to_string_pretty(&entry).map(|text| fs::write(&path, text));
}

/// Check whether a valid cache entry exists for the given `source_hash`.
///
/// Returns [`CacheResult::Hit`] only when an entry file exists *and* its
/// `compiler_version` field matches `env!("CARGO_PKG_VERSION")`.
pub fn check(cache_dir: &Path, source_hash: &str) -> CacheResult {
    let path = entry_path(cache_dir, source_hash);
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return CacheResult::Miss,
    };
    let json: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return CacheResult::Miss,
    };
    let stored_version = json
        .get("compiler_version")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if stored_version == env!("CARGO_PKG_VERSION") {
        CacheResult::Hit
    } else {
        CacheResult::Miss
    }
}

/// Return the `.resilient_cache/` directory that sits next to `source_file`.
pub fn cache_dir_for(source_file: &str) -> PathBuf {
    Path::new(source_file)
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(".resilient_cache")
}

/// Path of the JSON entry file for a given source hash inside `cache_dir`.
fn entry_path(cache_dir: &Path, source_hash: &str) -> PathBuf {
    cache_dir.join(format!("{}.json", source_hash))
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Create a temporary directory that is deleted when the guard is dropped.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(prefix: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "resilient_cache_test_{}_{}",
                prefix,
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("failed to create temp dir");
            TempDir(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    // ------------------------------------------------------------------
    // cache hit detection
    // ------------------------------------------------------------------

    #[test]
    fn cache_miss_when_directory_empty() {
        let tmp = TempDir::new("miss_empty");
        let result = check(tmp.path(), "deadbeef");
        assert_eq!(result, CacheResult::Miss);
    }

    #[test]
    fn cache_hit_after_write() {
        let tmp = TempDir::new("hit_after_write");
        let hash = "abc123";
        write_entry(tmp.path(), hash);
        assert_eq!(check(tmp.path(), hash), CacheResult::Hit);
    }

    #[test]
    fn cache_miss_for_different_hash() {
        let tmp = TempDir::new("miss_diff_hash");
        write_entry(tmp.path(), "hash_a");
        assert_eq!(check(tmp.path(), "hash_b"), CacheResult::Miss);
    }

    // ------------------------------------------------------------------
    // version mismatch invalidation
    // ------------------------------------------------------------------

    #[test]
    fn version_mismatch_returns_miss() {
        let tmp = TempDir::new("version_mismatch");
        let hash = "version_test_hash";
        // Write a cache entry with a deliberately different version string.
        let path = entry_path(tmp.path(), hash);
        let stale = serde_json::json!({
            "compiler_version": "0.0.0-stale",
            "source_hash": hash,
            "cached_at": 0u64,
        });
        fs::write(&path, serde_json::to_string_pretty(&stale).unwrap())
            .expect("could not write stale entry");
        assert_eq!(check(tmp.path(), hash), CacheResult::Miss);
    }

    // ------------------------------------------------------------------
    // write creates the directory
    // ------------------------------------------------------------------

    #[test]
    fn write_creates_cache_dir() {
        let base = TempDir::new("create_dir");
        // Use a nested path that doesn't exist yet.
        let cache = base.path().join("nested").join(".resilient_cache");
        assert!(!cache.exists());
        write_entry(&cache, "somehash");
        assert!(cache.exists());
    }

    // ------------------------------------------------------------------
    // cache_dir_for derives path from source file
    // ------------------------------------------------------------------

    #[test]
    fn cache_dir_for_adjacent_to_source() {
        let dir = cache_dir_for("/some/project/src/main.rs");
        assert_eq!(dir, PathBuf::from("/some/project/src/.resilient_cache"));
    }

    #[test]
    fn cache_dir_for_no_parent_falls_back_to_dot() {
        // A bare filename with no path component should use ".".
        let dir = cache_dir_for("program.rs");
        assert_eq!(dir, PathBuf::from(".resilient_cache"));
    }

    // ------------------------------------------------------------------
    // malformed JSON entry → miss (not a panic)
    // ------------------------------------------------------------------

    #[test]
    fn malformed_json_is_a_miss() {
        let tmp = TempDir::new("bad_json");
        let hash = "badhash";
        let path = entry_path(tmp.path(), hash);
        fs::write(&path, b"not valid json").expect("write failed");
        assert_eq!(check(tmp.path(), hash), CacheResult::Miss);
    }
}
