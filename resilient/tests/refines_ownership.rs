use std::{fs, process::Command};

#[test]
fn mappings_and_spec_paths_follow_retained_declarations() {
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/refines_ownership.json")).unwrap();
    for case in cases.as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let root =
            std::env::temp_dir().join(format!("refines_ownership_{}_{name}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        for (path, source) in case["files"].as_object().unwrap() {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, source.as_str().unwrap()).unwrap();
        }
        let mut command = Command::new(env!("CARGO_BIN_EXE_rz"));
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("RESILIENT_") {
                command.env_remove(key);
            }
        }
        let output = command
            .arg(root.join("entry.rz"))
            .args(["--typecheck", "--no-cache"])
            .output()
            .unwrap();
        let error = case["error"].as_str();
        assert_eq!(
            output.status.success(),
            error.is_none(),
            "{name}: {output:?}"
        );
        if let Some(error) = error {
            assert!(
                String::from_utf8_lossy(&output.stderr).contains(error),
                "{name}: {output:?}"
            );
        } else {
            assert!(
                String::from_utf8_lossy(&output.stdout).contains("Type check passed"),
                "{name}: {output:?}"
            );
        }
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn in_memory_verification_does_not_reuse_another_mapping() {
    let threads:Vec<_>=(0..6).map(|index|std::thread::spawn(move || {
        for iteration in 0..3 {
            let mapped=(index+iteration)%2==0;
            let source=if mapped {
                "@refines(spec=\"counter.tla\",action=\"Inc\") fn f(int value){return foreign(value);} extern \"counter\" {fn foreign(value:Int)->Int;};"
            } else {"fn f(int value){return value;} println(77);"};
            let result=resilient::run_program_with_options(source,resilient::RunOptions{deny_unproven_invariants:true});
            assert_eq!(result.ok,!mapped,"{source}: {:?}",result.errors);
            if mapped {assert!(result.errors.join("\n").contains("error[refinement]"));assert!(result.stdout.is_empty());}
            else {assert_eq!(result.stdout.trim(),"77");}
        }
    })).collect();
    for thread in threads {
        thread.join().unwrap();
    }
}
