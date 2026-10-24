//! The version URL serves the already published, version-pinned installer bytes.
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

#[test]
fn version_endpoint_is_exact_published_raw_installer() {
    // GitHub v1.2.0 install.sh asset: 5687 bytes; digest independently
    // checked against the downloaded bytes and release-manifest installer entry.
    let bytes = fs::read(root().join("docs/v1.2.0")).expect("versioned raw installer endpoint");
    assert_eq!(bytes.len(), 5687);
    assert_eq!(
        format!("{:x}", Sha256::digest(&bytes)),
        "171f1709c74e32bb76b3cd532deac5abf9ede02d70555f196ff43d59552f3d74"
    );
    let script = std::str::from_utf8(&bytes).unwrap();
    assert!(script.starts_with("#!/usr/bin/env bash\n"));
    assert!(script.contains("TAG=\"${RZ_VERSION:-v1.2.0}\""));
    assert!(!script.contains("<!DOCTYPE") && !script.starts_with("---"));
    assert!(script.contains("download checksum does not match; installation unchanged"));
}

#[test]
fn documented_short_url_is_additive_to_existing_release_command() {
    let command = "curl -fsSL https://ericspencer.us/Resilient/v1.2.0 | bash";
    for path in ["README.md", "docs/getting-started.md"] {
        let text = fs::read_to_string(root().join(path)).unwrap();
        assert!(text.contains(command), "missing version URL in {path}");
        assert!(text.contains("curl -fsSL https://github.com/EricSpencer00/Resilient/releases/download/v1.2.0/install.sh | bash"), "existing release command removed from {path}");
    }
    let landing = fs::read_to_string(root().join("docs/_layouts/landing.html")).unwrap();
    assert!(landing.contains("curl -fsSL https://github.com/EricSpencer00/Resilient/releases/download/v1.2.0/install.sh | bash"));
}
