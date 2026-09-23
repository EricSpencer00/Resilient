use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "res-pkg-archive-budget-{}-{id}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn manifest() -> PublishManifest {
    PublishManifest {
        name: "budget-test".into(),
        version: "1.0.0".into(),
        description: None,
        entry: PathBuf::from("main.rz"),
    }
}

fn limits(max_file_count: usize, max_file_bytes: u64, max_archive_bytes: u64) -> ArchiveLimits {
    ArchiveLimits {
        max_file_count,
        max_file_bytes,
        max_archive_bytes,
    }
}

fn assert_limit_error(result: Result<Vec<u8>, PkgPublishError>, expected_resource: &str) {
    match result {
        Err(PkgPublishError::ArchiveLimitExceeded { resource, .. }) => {
            assert_eq!(resource, expected_resource);
        }
        other => panic!("expected {expected_resource} limit error, got {other:?}"),
    }
}

#[test]
fn exact_archive_budget_keeps_valid_deterministic_tar_output() {
    let root = TestDir::new();
    fs::write(root.0.join("main.rz"), vec![b'x'; 512]).unwrap();
    let files = [PathBuf::from("main.rz")];
    let budget = limits(1, 512, 2048);

    let first = make_tarball_with_limits(&root.0, &manifest(), &files, budget).unwrap();
    let second = make_tarball_with_limits(&root.0, &manifest(), &files, budget).unwrap();

    assert_eq!(first.len(), 2048);
    assert_eq!(&first[257..262], b"ustar");
    assert!(first[1024..].iter().all(|byte| *byte == 0));
    assert_eq!(first, second);
}

#[test]
fn oversized_file_is_rejected_from_metadata() {
    let root = TestDir::new();
    fs::write(root.0.join("large.rz"), b"four").unwrap();
    let result = make_tarball_with_limits(
        &root.0,
        &manifest(),
        &[PathBuf::from("large.rz")],
        limits(1, 3, 2048),
    );

    assert_limit_error(result, "file size");
}

#[test]
fn aggregate_archive_size_is_preflighted_before_body_reads() {
    let root = TestDir::new();
    fs::write(root.0.join("first.rz"), vec![b'a'; 512]).unwrap();
    fs::write(root.0.join("second.rz"), vec![b'b'; 512]).unwrap();
    let files = [PathBuf::from("first.rz"), PathBuf::from("second.rz")];
    let result = make_tarball_with_limits(&root.0, &manifest(), &files, limits(2, 512, 3071));

    assert_limit_error(result, "archive size");
}

#[test]
fn file_count_is_rejected_before_inspecting_paths() {
    let root = TestDir::new();
    let files = [PathBuf::from("missing-a"), PathBuf::from("missing-b")];
    let result = make_tarball_with_limits(&root.0, &manifest(), &files, limits(1, 10, 2048));

    assert_limit_error(result, "publishable file count");
}

#[test]
fn file_collection_stops_at_its_count_budget() {
    let root = TestDir::new();
    fs::write(root.0.join("first.rz"), b"a").unwrap();
    fs::write(root.0.join("second.rz"), b"b").unwrap();

    match collect_publishable_files_with_file_limit(&root.0, 1) {
        Err(PkgPublishError::ArchiveLimitExceeded { resource, limit }) => {
            assert_eq!(resource, "publishable file count");
            assert_eq!(limit, 1);
        }
        other => panic!("expected file-count limit error, got {other:?}"),
    }
}

#[test]
fn oversized_gitignore_is_rejected_before_reading() {
    let root = TestDir::new();
    let ignore = fs::File::create(root.0.join(".gitignore")).unwrap();
    ignore.set_len(MAX_PUBLISH_FILE_BYTES + 1).unwrap();

    match collect_publishable_files(&root.0) {
        Err(PkgPublishError::ArchiveLimitExceeded { resource, .. }) => {
            assert_eq!(resource, "gitignore size");
        }
        other => panic!("expected gitignore size limit error, got {other:?}"),
    }
}
