use std::{fs, process::Command};

fn rejected(name: &str, files: &[(&str, &str)], expected: &[(&str, usize)], absent: &[&str]) {
    let root = std::env::temp_dir().join(format!("strict_origins_{}_{name}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    for (file, source) in files {
        fs::write(root.join(file), source).unwrap();
    }
    for vm in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rz"));
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("RESILIENT_") {
                command.env_remove(key);
            }
        }
        command
            .arg("--deny-unproven-invariants")
            .arg(root.join("entry.rz"));
        if vm {
            command.arg("--vm");
        }
        let output = command.output().unwrap();
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{output:?}");
        assert!(!String::from_utf8_lossy(&output.stdout).contains("FORBIDDEN"));
        assert!(error.contains("unproven-loop-invariant:"), "{error}");
        for (file, line) in expected {
            if *file == "<unknown origin>" {
                assert!(
                    error.contains(&format!("<unknown origin>:{line}:")),
                    "{error}"
                );
                continue;
            }
            assert!(
                error.contains(&format!("{}:{line}:", root.join(file).display())),
                "{error}"
            );
        }
        let proof_errors = error
            .lines()
            .filter(|line| line.contains("unproven-loop-invariant:"))
            .collect::<Vec<_>>()
            .join("\n");
        for file in absent {
            assert!(
                !proof_errors.contains(&format!("{}:", root.join(file).display())),
                "{error}"
            );
        }
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn imported_position_does_not_belong_to_entry() {
    rejected(
        "dependency",
        &[
            ("entry.rz", "use \"dep.rz\";\nprintln(\"FORBIDDEN\");"),
            (
                "dep.rz",
                "\n\n\n\n\nfn unused() { while false invariant false {} }",
            ),
        ],
        &[("dep.rz", 6)],
        &["entry.rz"],
    );
}

#[test]
fn alias_collisions_keep_distinct_sources() {
    rejected(
        "aliases",
        &[
            (
                "entry.rz",
                "use \"a.rz\" as a; use \"b.rz\" as b; println(\"FORBIDDEN\");",
            ),
            ("a.rz", "fn f() { while false invariant false {} }"),
            ("b.rz", "fn f() { while false invariant false {} }"),
        ],
        &[("a.rz", 1), ("b.rz", 1)],
        &["entry.rz"],
    );
}

#[test]
fn reexport_retains_leaf_source() {
    rejected(
        "reexport",
        &[
            ("entry.rz", "use \"middle.rz\"; println(\"FORBIDDEN\");"),
            ("middle.rz", "pub use \"leaf.rz\";"),
            ("leaf.rz", "\n\nfn f() { while false invariant false {} }"),
        ],
        &[("leaf.rz", 3)],
        &["entry.rz", "middle.rz"],
    );
}

#[test]
fn filtering_private_declarations_preserves_alignment() {
    rejected(
        "filter",
        &[
            (
                "entry.rz",
                "use \"dep.rz\";\nwhile false invariant false {} println(\"FORBIDDEN\");",
            ),
            (
                "dep.rz",
                "fn hidden() { while false invariant false {} }\npub fn visible() { while false invariant false {} }",
            ),
        ],
        &[("dep.rz", 2), ("entry.rz", 2)],
        &[],
    );
}

#[test]
fn local_without_imports_has_entry_source() {
    rejected(
        "local",
        &[(
            "entry.rz",
            "\nwhile false invariant false {} println(\"FORBIDDEN\");",
        )],
        &[("entry.rz", 2)],
        &[],
    );
}

#[cfg(feature = "z3")]
#[test]
fn proven_import_cannot_mask_colliding_unproven_site() {
    rejected(
        "mixed",
        &[
            (
                "entry.rz",
                "use \"good.rz\"; use \"bad.rz\"; println(\"FORBIDDEN\");",
            ),
            ("good.rz", "while false invariant true {}"),
            ("bad.rz", "while false invariant false {}"),
        ],
        &[("bad.rz", 1)],
        &["entry.rz", "good.rz"],
    );
}

#[test]
fn synthetic_impl_insertion_does_not_shift_source_labels() {
    rejected(
        "synthetic",
        &[(
            "entry.rz",
            r#"struct Score { int value, }
trait Display { fn show(self) -> string; }
trait Loud { fn shout(self) -> string; }
impl Display for Score { fn show(self) -> string { return "ok"; } }
impl<T: Display> Loud for T { fn shout(self) -> string { return self.show(); } }
while false invariant false {} println("FORBIDDEN");
"#,
        )],
        &[("<unknown origin>", 6)],
        &["entry.rz"],
    );
}
