//! RES-3253: self-host docs and comments use the current `.rz` extension.

#[test]
fn self_host_copy_uses_rz_extension() {
    let readme = include_str!("../../self-host/README.md");
    let lexer = include_str!("../../self-host/lexer.rz");

    assert!(
        readme.contains("Walk `resilient/examples/*.rz`"),
        "self-host README should point at current .rz examples"
    );
    assert!(
        lexer.contains("running `lexer.rz` directly"),
        "self-host lexer comment should mention lexer.rz"
    );
    assert!(
        !readme.contains("*.{rz,res}"),
        "self-host README should not mention the retired .res extension"
    );
    assert!(
        !lexer.contains("lexer.res"),
        "self-host lexer comment should not mention the retired .res extension"
    );
}
