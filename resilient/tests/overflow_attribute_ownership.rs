use std::{fs, process::Command};

#[test]
fn overflow_check_enrollment_follows_the_declaration() {
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/overflow_attribute_ownership.json")).unwrap();
    for case in cases.as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let root =
            std::env::temp_dir().join(format!("overflow_ownership_{}_{name}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        for (path, source) in case["files"].as_object().unwrap() {
            fs::write(root.join(path), source.as_str().unwrap()).unwrap();
        }
        let mut modes = vec!["interpreter", "match", "direct"];
        if cfg!(feature = "jit") {
            modes.push("jit");
        }
        for mode in modes {
            let mut command = Command::new(env!("CARGO_BIN_EXE_rz"));
            for (key, _) in std::env::vars_os() {
                if key.to_string_lossy().starts_with("RESILIENT_") {
                    command.env_remove(key);
                }
            }
            command
                .arg(root.join("entry.rz"))
                .args(["--typecheck-strict", "--no-cache"]);
            if mode == "jit" {
                command.arg("--jit");
            } else if mode != "interpreter" {
                command.arg("--vm").env("RESILIENT_DISPATCH", mode);
            }
            let output = command.output().unwrap();
            let reject = cfg!(feature = "z3") && case["reject"].as_bool().unwrap();
            assert_eq!(
                output.status.success(),
                !reject,
                "{name}/{mode}: {output:?}"
            );
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert_eq!(
                stdout
                    .trim_end()
                    .trim_end_matches("Program executed successfully")
                    .trim_end(),
                if reject { "" } else { "77" },
                "{name}/{mode}: {output:?}"
            );
            if reject {
                assert!(
                    String::from_utf8_lossy(&output.stderr).contains("overflow counterexample"),
                    "{name}/{mode}: {output:?}"
                );
            }
        }
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn in_memory_invocations_keep_overflow_enrollment_isolated() {
    let threads: Vec<_> = (0..6)
        .map(|index| {
            std::thread::spawn(move || {
                for iteration in 0..3 {
                    let enrolled = (index + iteration) % 2 == 0;
                    let attribute = if enrolled { "#[overflow_checked]" } else { "" };
                    let source =
                        format!("{attribute} fn f(int x) ensures x+1>x {{return x;}} println(77);");
                    let output = resilient::run_program_with_options(
                        &source,
                        resilient::RunOptions {
                            deny_unproven_invariants: true,
                        },
                    );
                    let reject = cfg!(feature = "z3") && enrolled;
                    assert_eq!(output.ok, !reject, "{source}: {:?}", output.errors);
                    assert_eq!(output.stdout.trim(), if reject { "" } else { "77" });
                    if reject {
                        assert!(output.errors.join("\n").contains("overflow counterexample"));
                    }
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
}
