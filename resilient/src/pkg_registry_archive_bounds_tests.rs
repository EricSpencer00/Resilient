use super::{PkgRegistryError, extract_ustar};

fn header_with_size(size: &str) -> Vec<u8> {
    let mut header = vec![0u8; 512];
    header[..8].copy_from_slice(b"pkg/file");
    let encoded = format!("{size}\0");
    header[124..124 + encoded.len()].copy_from_slice(encoded.as_bytes());
    header[156] = b'0';
    header
}

fn temp_destination(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("rz-archive-bounds-{label}-{}", std::process::id()))
}

#[test]
fn rejects_entry_size_that_exceeds_target_or_archive() {
    // Eleven octal digits exceed u32::MAX while remaining representable in a
    // USTAR size field. On 32-bit targets conversion must fail; on wider
    // targets the body bound must still reject the truncated archive.
    let bytes = header_with_size("77777777777");
    let dest = temp_destination("oversized");

    let err = extract_ustar(&bytes, &dest).unwrap_err();

    assert!(matches!(err, PkgRegistryError::ExtractFailed { .. }));
    let _ = std::fs::remove_dir_all(dest);
}

#[test]
fn rejects_truncated_entry_padding_before_writing() {
    let mut bytes = header_with_size("1");
    bytes.push(b'x');
    let dest = temp_destination("padding");

    let err = extract_ustar(&bytes, &dest).unwrap_err();

    assert!(matches!(err, PkgRegistryError::ExtractFailed { .. }));
    assert!(!dest.join("file").exists());
    let _ = std::fs::remove_dir_all(dest);
}
