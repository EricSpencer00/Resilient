//! RES-3237: certificate docs match the emitted manifest JSON shape.

#[test]
fn certificates_doc_describes_current_manifest_shape() {
    let doc = include_str!("../../docs/CERTIFICATES.md");

    assert!(
        doc.contains("`manifest.json` enumerating every obligation"),
        "certificate docs should use the emitted manifest filename"
    );
    assert!(
        doc.contains("\"program\": \"examples/sensor.rz\""),
        "sample manifest should include the emitted program field"
    );
    assert!(
        doc.contains("\"obligations\": ["),
        "sample manifest should include the emitted obligations field"
    );
    assert!(
        doc.contains("optional metadata such as schema version")
            && doc.contains("without breaking older consumers"),
        "docs should explain that metadata can be added later"
    );
    assert!(
        !doc.contains("MANIFEST.json"),
        "docs should not use the retired uppercase manifest filename"
    );
    assert!(
        !doc.contains("\"compiler\": \"resilient 0.1.0\""),
        "sample manifest should not include stale compiler metadata"
    );
    assert!(
        !doc.contains("| `schema`      | string  | yes"),
        "schema should not be documented as a required emitted field"
    );
    assert!(
        !doc.contains("| `compiler`    | string  | yes"),
        "compiler should not be documented as a required emitted field"
    );
}
