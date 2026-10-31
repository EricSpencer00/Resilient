#![cfg(feature = "z3")]
use std::{fs, process::Command};

fn check(name: &str, source: &str, proofs: [bool; 3], success: [bool; 3]) {
    for (index, policy) in ["wrap", "saturate", "trap"].iter().enumerate() {
        for mode in ["interpreter", "match", "direct"] {
            let root = std::env::temp_dir().join(format!(
                "loop_machine_{}_{}_{}_{}",
                std::process::id(),
                name,
                policy,
                mode
            ));
            fs::create_dir_all(&root).unwrap();
            let path = root.join("input.rz");
            fs::write(&path, source).unwrap();
            let certs = root.join("certs");
            let mut command = Command::new(env!("CARGO_BIN_EXE_rz"));
            for (key, _) in std::env::vars_os() {
                if key.to_string_lossy().starts_with("RESILIENT_") {
                    command.env_remove(key);
                }
            }
            command
                .env("RESILIENT_OVERFLOW_MODE", policy)
                .args([
                    "--typecheck",
                    "--no-cache",
                    "--verbose",
                    "--emit-certificate",
                ])
                .arg(&certs)
                .arg(&path);
            if mode != "interpreter" {
                command.arg("--vm").env("RESILIENT_DISPATCH", mode);
            }
            let output = command.output().unwrap();
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(!stderr.contains("runtime check elided"), "{stderr}");
            assert!(
                String::from_utf8_lossy(&output.stdout).contains("Type check passed"),
                "{name}/{policy}/{mode}: {stderr}"
            );
            assert_eq!(
                output.status.success(),
                success[index],
                "{name}/{policy}/{mode}: {stderr}"
            );
            let manifest: serde_json::Value =
                serde_json::from_slice(&fs::read(certs.join("manifest.json")).unwrap()).unwrap();
            let obligations = manifest["obligations"].as_array().unwrap();
            let loops: Vec<_> = obligations
                .iter()
                .filter(|item| {
                    item["kind"]
                        .as_str()
                        .is_some_and(|kind| kind.starts_with("loop_invariant"))
                })
                .collect();
            assert_eq!(
                !loops.is_empty(),
                proofs[index],
                "{name}/{policy}/{mode}: {manifest}\n{stderr}"
            );
            for obligation in loops {
                let query_path = certs.join(obligation["cert"].as_str().unwrap());
                let query = fs::read_to_string(&query_path).unwrap();
                assert!(query.contains("(set-logic QF_BV)"), "{query}");
                assert!(
                    query.contains(&format!("; overflow-policy: {policy}")),
                    "{query}"
                );
                let replay = Command::new("z3").arg(&query_path).output().unwrap();
                assert!(replay.status.success(), "{replay:?}");
                assert_eq!(
                    String::from_utf8_lossy(&replay.stdout).trim(),
                    "unsat",
                    "{replay:?}"
                );
            }
            fs::remove_dir_all(root).unwrap();
        }
    }
}
macro_rules! case {
    ($name:ident, $source:literal, $proofs:expr, $success:expr) => {
        #[test]
        fn $name() {
            check(stringify!($name), $source, $proofs, $success);
        }
    };
}
case!(
    while_overflow,
    "let i=9223372036854775807; let j=0; while j<1 invariant i>=0 { i=i+1; j=j+1; }",
    [false, true, false],
    [false, true, false]
);
case!(
    for_overflow,
    "let i=9223372036854775807; for x in [1,2] invariant i>=0 { i=i+x; }",
    [false, true, false],
    [false, true, false]
);
case!(
    while_safe,
    "let i=0; while i<2 invariant i>=0 && i<=2 { i=i+1; }",
    [true; 3],
    [true; 3]
);
case!(
    for_wrap_valid,
    "let i=9223372036854775807; for x in [1,2] invariant i<=9223372036854775807 { i=i+x; }",
    [true, true, false],
    [true, true, false]
);
case!(
    mul_overflow,
    "let i=9223372036854775807; for x in [2,2] invariant i>=0 { i=i*x; }",
    [false, true, false],
    [false, true, false]
);
case!(
    sub_overflow,
    "let i=-9223372036854775807; for x in [2,1] invariant i<=0 { i=i-x; }",
    [false, true, false],
    [false, true, false]
);
case!(
    neg_overflow,
    "let i=-9223372036854775807; for x in [1,1] invariant i<=0 { i=i-1; i=-i; }",
    [true, false, false],
    [true, false, false]
);
case!(
    div_overflow,
    "let i=-9223372036854775807; for x in [1,1] invariant i<=0 { i=i-1; i=i/-1; }",
    [true, false, false],
    [true, false, false]
);
case!(
    rem_overflow,
    "let i=-9223372036854775807; for x in [1,1] invariant i<=0 { i=i-1; i=i%-1; }",
    [true, true, false],
    [true, true, false]
);
case!(
    unused_div_zero,
    "let i=0; for x in [0] invariant true { i=1/x; }",
    [false; 3],
    [false; 3]
);
case!(
    unreachable_div_zero,
    "fn zero() -> int { return 0; } let d=zero(); let i=0; while false invariant i==0 { i=1/d; }",
    [true; 3],
    [true; 3]
);
case!(
    short_circuit,
    "while false invariant true || (1/0==0) { }",
    [true; 3],
    [true; 3]
);
case!(
    condition_div_zero,
    "fn zero() -> int { return 0; } let d=zero(); let i=0; while 1/d>0 invariant i==0 { i=i; }",
    [false; 3],
    [false; 3]
);

case!(
    finite_relational_trace,
    "let i=0; let j=0; let n=5; while i<n invariant i+j>=0 { j=j+1; i=i+1; }",
    [true; 3],
    [true; 3]
);

case!(
    late_trace_violation,
    "let i=0; let j=0; while i<20 invariant i+j<39 { j=j+1; i=i+1; }",
    [false; 3],
    [false; 3]
);

case!(
    float_nan_domain,
    "let x=sqrt(-1.0); while false invariant x==x { }",
    [false; 3],
    [false; 3]
);
case!(
    float_parameter_domain,
    "fn check(float x) { while false invariant x==x { } } check(sqrt(-1.0));",
    [false; 3],
    [false; 3]
);
case!(
    integer_parameter_domain,
    "fn check(int x) { while false invariant x==x { } } check(1);",
    [true; 3],
    [true; 3]
);
case!(
    for_shadow_float_domain,
    "let x=sqrt(-1.0); for x in [1] { } while false invariant x==x { }",
    [false; 3],
    [false; 3]
);
