use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn run_src(tag: &str, src: &str) -> (String, String, Option<i32>) {
    let mut path: PathBuf = std::env::temp_dir();
    path.push(format!("res_775_{tag}_{}.rz", std::process::id()));
    {
        let mut f = std::fs::File::create(&path).expect("create temp .rz");
        f.write_all(src.as_bytes()).expect("write src");
    }
    let output = Command::new(bin())
        .arg(&path)
        .output()
        .expect("spawn resilient");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn try_catch_runtime_dispatches_to_matching_handler() {
    let src = "\
        fn read_sensor(int addr)\n\
            requires addr >= 0\n\
            fails Timeout\n\
        {\n\
            return addr;\n\
        }\n\
        \n\
        fn main() {\n\
            try {\n\
                let v = read_sensor(42);\n\
                println(v);\n\
            } catch Timeout {\n\
                println(-1);\n\
            }\n\
        }\n\
        \n\
        main();\n\
    ";
    let (stdout, stderr, code) = run_src("matching_handler", src);
    assert_eq!(
        code,
        Some(0),
        "try/catch runtime dispatch must exit 0; stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.contains("-1"),
        "expected handler output in stdout, got:\nstdout={stdout}\nstderr={stderr}"
    );
    assert!(
        stdout.contains("Program executed successfully"),
        "driver success banner missing; stdout={stdout}"
    );
}

#[test]
fn unchecked_inner_handler_propagates_to_outer_try() {
    let src = "\
        fn read_sensor(int addr)\n\
            requires addr >= 0\n\
            fails Timeout, HardwareFault\n\
        {\n\
            return addr;\n\
        }\n\
        \n\
        fn main() {\n\
            try {\n\
                try {\n\
                    let v = read_sensor(42);\n\
                    println(v);\n\
                } catch HardwareFault {\n\
                    println(-2);\n\
                }\n\
            } catch Timeout {\n\
                println(-1);\n\
            }\n\
        }\n\
        \n\
        main();\n\
    ";
    let (stdout, stderr, code) = run_src("outer_propagation", src);
    assert_eq!(
        code,
        Some(0),
        "outer try should intercept propagated failure; stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.contains("-1"),
        "expected outer handler output in stdout, got:\nstdout={stdout}\nstderr={stderr}"
    );
    assert!(
        !stdout.contains("-2"),
        "inner non-matching handler must stay inactive; stdout={stdout}"
    );
}
