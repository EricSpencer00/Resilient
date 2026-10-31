#![cfg(all(feature = "z3", unix))]

use std::{fs, process::Command};

fn check(name: &str, source: &str, proof: bool, success: bool) {
    let root = std::env::temp_dir().join(format!("loop_expansion_{}_{name}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let source_path = root.join("input.rz");
    fs::write(&source_path, source).unwrap();
    for mode in ["interpreter", "match", "direct"] {
        let output = Command::new("python3")
            .args([
                "-c",
                r#"
import os,resource,subprocess,sys
binary,source,mode,proof,success=sys.argv[1:]
env={k:v for k,v in os.environ.items() if not k.startswith('RESILIENT_')}
env['RESILIENT_OVERFLOW_MODE']='wrap'
args=[binary,source,'--typecheck','--no-cache','--verbose']
if mode!='interpreter':
    args.append('--vm');env['RESILIENT_DISPATCH']=mode
def limit():
    resource.setrlimit(resource.RLIMIT_AS,(512*1024**2,512*1024**2))
    resource.setrlimit(resource.RLIMIT_CORE,(0,0))
r=subprocess.run(args,env=env,capture_output=True,text=True,timeout=10,preexec_fn=limit)
assert 'Type check passed' in r.stdout,(r.returncode,r.stdout,r.stderr)
assert (r.returncode==0)==(success=='true'),(r.returncode,r.stdout,r.stderr)
assert ('invariant proven' in r.stderr)==(proof=='true'),r.stderr
if success=='false':assert 'loop invariant violated' in r.stderr,r.stderr
"#,
            ])
            .arg(env!("CARGO_BIN_EXE_rz"))
            .arg(&source_path)
            .arg(mode)
            .arg(proof.to_string())
            .arg(success.to_string())
            .output()
            .unwrap();
        assert!(output.status.success(), "{name}/{mode}: {output:?}");
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn while_substitution_budget_precedes_allocation() {
    check(
        "while_large",
        &format!(
            "let x=0; while false invariant x==0 {{ {} }}",
            "x=x+x;".repeat(24)
        ),
        false,
        true,
    );
}
#[test]
fn for_assignment_expansion_budget_precedes_allocation() {
    check(
        "for_large",
        &format!(
            "let x=0; for i in [1] invariant x==0 {{ {} }}",
            "x=x+x;".repeat(24)
        ),
        false,
        true,
    );
}
#[test]
fn for_iteration_expansion_shares_budget() {
    check(
        "for_iterations",
        &format!(
            "let x=0; for i in [{}] invariant x==0 {{ x=x+x; }}",
            vec!["1"; 24].join(",")
        ),
        false,
        true,
    );
}
#[test]
fn finite_while_trace_expansion_shares_budget() {
    check(
        "while_trace",
        "let x=1; let i=0; while i<24 invariant x>=0 { x=x+x; i=i+1; }",
        false,
        true,
    );
}
#[test]
fn small_supported_proofs_remain_available() {
    check(
        "while_small",
        &format!(
            "let x=0; while false invariant x==0 {{ {} }}",
            "x=x+x;".repeat(5)
        ),
        true,
        true,
    );
    check(
        "for_small",
        "let x=0; for i in [1,1,1] invariant x==0 { x=x+x; }",
        true,
        true,
    );
}
#[test]
fn exhausted_proof_budget_does_not_disable_runtime_guard() {
    check(
        "runtime_guard",
        &format!(
            "let x=1; while false invariant x==0 {{ {} }}",
            "x=x+x;".repeat(24)
        ),
        false,
        false,
    );
}
