//! RES-4116 (D-E3): compile-time stdlib portability lint.
//!
//! Rejects calls to Tier-2/3 stdlib builtins (per
//! `docs/STDLIB_PORTABILITY.md`) when the nearest `rz.toml` /
//! `resilient.toml` manifest declares a `[target.TRIPLE]` profile that
//! is a `no_std` / bare-metal embedded target — mirroring the parse
//! done by `target_profiles::parse_target_profiles` (RES-2614).
//!
//! ## Scope (first increment)
//!
//! This pass only fires when static target-profile information is
//! actually available: a manifest exists next to the source file and
//! declares at least one `[target.TRIPLE]` section whose `features`
//! list contains `"no_std"`, or whose triple is a known bare-metal
//! embedded triple (`thumbv7em-none-eabihf`, `thumbv6m-none-eabi`,
//! `riscv32imac-unknown-none-elf`). When no manifest is present, or no
//! declared target is embedded/no_std, the pass is a no-op — this
//! keeps the false-positive rate at zero rather than guessing at an
//! implicit build target (RES-4116 acceptance criteria).
//!
//! `wasm32` targets are deliberately NOT covered by this pass yet:
//! `env`/`http_get`/`http_post`/`exec`/`exec_shell`/`file_*` need
//! graceful `Err`-returning wasm32 stubs (mirroring the `file_io.rs`
//! VFS pattern already used for `fs`/time) before they can safely
//! resolve instead of hard-rejecting — tracked as a follow-up ticket.
//!
//! ## Diagnostic
//!
//! ```text
//! examples/bad.rz:3:12: error[stdlib-portability]: builtin `exec` requires
//! Tier 3 (process control), which is unavailable on target
//! `thumbv7em-none-eabihf` (declared as a no_std/embedded profile in rz.toml)
//! ```

#![allow(dead_code)]

use crate::Node;
use crate::uniqueness_walk::visit;
use std::path::Path;

/// Known bare-metal / no_std embedded target triples. Kept as a fixed
/// list (rather than a substring heuristic) so the lint never
/// misclassifies a host or wasm triple as embedded.
const EMBEDDED_TRIPLES: &[&str] = &[
    "thumbv7em-none-eabihf",
    "thumbv6m-none-eabi",
    "riscv32imac-unknown-none-elf",
];

/// A Tier-2/3 builtin with no portable no_std implementation, and a
/// short human-readable description of the resource it needs.
struct TierBuiltin {
    name: &'static str,
    tier: u8,
    resource: &'static str,
}

/// RES-4116: the enforcement table. Every entry here is a stdlib
/// builtin that unconditionally needs host OS / heap / network
/// facilities not present on a bare-metal `no_std` target. Builtins
/// that already have a portable no_std or wasm32-VFS path (`fs`
/// read/write route through `file_io::vfs_*` on wasm32; core/alloc
/// collection builtins) are intentionally excluded.
const TIER_BUILTINS: &[TierBuiltin] = &[
    TierBuiltin {
        name: "file_read",
        tier: 2,
        resource: "file I/O",
    },
    TierBuiltin {
        name: "file_write",
        tier: 2,
        resource: "file I/O",
    },
    TierBuiltin {
        name: "file_exists",
        tier: 2,
        resource: "file metadata",
    },
    TierBuiltin {
        name: "file_is_dir",
        tier: 2,
        resource: "file metadata",
    },
    TierBuiltin {
        name: "file_is_file",
        tier: 2,
        resource: "file metadata",
    },
    TierBuiltin {
        name: "file_size",
        tier: 2,
        resource: "file metadata",
    },
    TierBuiltin {
        name: "file_stat",
        tier: 2,
        resource: "file metadata",
    },
    TierBuiltin {
        name: "dir_list",
        tier: 2,
        resource: "file metadata",
    },
    TierBuiltin {
        name: "env",
        tier: 2,
        resource: "environment access",
    },
    TierBuiltin {
        name: "http_get",
        tier: 2,
        resource: "networking",
    },
    TierBuiltin {
        name: "http_post",
        tier: 2,
        resource: "networking",
    },
    TierBuiltin {
        name: "exec",
        tier: 3,
        resource: "process control",
    },
    TierBuiltin {
        name: "exec_shell",
        tier: 3,
        resource: "process control",
    },
    TierBuiltin {
        name: "tcp_connect",
        tier: 2,
        resource: "networking",
    },
    TierBuiltin {
        name: "tcp_listen",
        tier: 2,
        resource: "networking",
    },
    TierBuiltin {
        name: "tcp_accept",
        tier: 2,
        resource: "networking",
    },
    TierBuiltin {
        name: "tcp_read",
        tier: 2,
        resource: "networking",
    },
    TierBuiltin {
        name: "tcp_write",
        tier: 2,
        resource: "networking",
    },
    TierBuiltin {
        name: "tcp_close",
        tier: 2,
        resource: "networking",
    },
    TierBuiltin {
        name: "tcp_set_timeout",
        tier: 2,
        resource: "networking",
    },
    TierBuiltin {
        name: "udp_bind",
        tier: 2,
        resource: "networking",
    },
    TierBuiltin {
        name: "udp_send_to",
        tier: 2,
        resource: "networking",
    },
    TierBuiltin {
        name: "udp_recv_from",
        tier: 2,
        resource: "networking",
    },
    TierBuiltin {
        name: "udp_close",
        tier: 2,
        resource: "networking",
    },
];

fn find_tier_builtin(name: &str) -> Option<&'static TierBuiltin> {
    TIER_BUILTINS.iter().find(|b| b.name == name)
}

/// A declared `[target.TRIPLE]` profile that is provably no_std/embedded.
struct EmbeddedTarget {
    triple: String,
}

fn is_embedded_profile(triple: &str, profile: &crate::target_profiles::TargetProfile) -> bool {
    EMBEDDED_TRIPLES.contains(&triple) || profile.features.iter().any(|f| f == "no_std")
}

/// Load the nearest manifest (matching `target_profiles::check`'s
/// lookup) and return the embedded/no_std target profiles it declares,
/// if any. `None` when no manifest is present or it declares no
/// embedded targets — the pass is then a no-op.
fn embedded_targets_for(source_path: &str) -> Vec<EmbeddedTarget> {
    let source_dir = Path::new(source_path).parent().unwrap_or(Path::new("."));
    let manifest_path = ["rz.toml", "resilient.toml"]
        .iter()
        .map(|name| source_dir.join(name))
        .find(|p| p.exists());

    let Some(manifest_path) = manifest_path else {
        return Vec::new();
    };
    let Ok(manifest_content) = std::fs::read_to_string(&manifest_path) else {
        return Vec::new();
    };

    let profiles = crate::target_profiles::parse_target_profiles(&manifest_content);
    let mut triples: Vec<String> = profiles
        .iter()
        .filter(|(triple, profile)| is_embedded_profile(triple, profile))
        .map(|(triple, _)| triple.clone())
        .collect();
    triples.sort();
    triples
        .into_iter()
        .map(|triple| EmbeddedTarget { triple })
        .collect()
}

/// RES-4116: called from `typechecker.rs` `<EXTENSION_PASSES>`.
///
/// No-op when no manifest is present or no declared `[target.TRIPLE]`
/// section is embedded/no_std (static-info-unavailable case — the
/// pass deliberately skips rather than guesses). Otherwise, every
/// `CallExpression` invoking a builtin from [`TIER_BUILTINS`] is a
/// hard compile error naming the builtin, its tier/resource, and the
/// offending target triple.
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let embedded_targets = embedded_targets_for(source_path);
    if embedded_targets.is_empty() {
        return Ok(());
    }

    let mut errors: Vec<String> = Vec::new();
    visit(program, &mut |node| {
        if let Node::CallExpression { function, span, .. } = node
            && let Node::Identifier { name, .. } = function.as_ref()
            && let Some(builtin) = find_tier_builtin(name)
        {
            for target in &embedded_targets {
                errors.push(format!(
                    "{}:{}:{}: error[stdlib-portability]: builtin `{}` requires Tier {} ({}), \
                     which is unavailable on target `{}` (declared as a no_std/embedded profile)",
                    source_path,
                    span.start.line,
                    span.start.column,
                    builtin.name,
                    builtin.tier,
                    builtin.resource,
                    target.triple,
                ));
            }
        }
    });

    if errors.is_empty() {
        Ok(())
    } else {
        errors.sort();
        errors.dedup();
        Err(errors.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn write_manifest(manifest: &str, src: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("__resilient_stdlib_portability_{id}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("rz.toml"), manifest).unwrap();
        let src_path = dir.join("main.rz");
        std::fs::write(&src_path, src.as_bytes()).unwrap();
        (dir, src_path)
    }

    #[test]
    fn no_manifest_is_a_no_op() {
        let (prog, _) = crate::parse("fn main() { exec(\"ls\"); }");
        let result = check(&prog, "/nonexistent/dir/main.rz");
        assert!(result.is_ok());
    }

    #[test]
    fn manifest_without_embedded_target_is_a_no_op() {
        let manifest = r#"[package]
name = "a"
version = "1.0.0"
[target.x86_64-unknown-linux]
features = ["std"]
"#;
        let (dir, src_path) = write_manifest(manifest, "fn main() { exec(\"ls\"); }");
        let (prog, _) = crate::parse("fn main() { exec(\"ls\"); }");
        let result = check(&prog, src_path.to_str().unwrap());
        assert!(result.is_ok(), "unexpected error: {:?}", result);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn embedded_target_by_triple_rejects_process_control() {
        let manifest = r#"[package]
name = "a"
version = "1.0.0"
[target.thumbv7em-none-eabihf]
opt_level = "s"
"#;
        let src = "fn main() { exec(\"ls\"); }";
        let (dir, src_path) = write_manifest(manifest, src);
        let (prog, _) = crate::parse(src);
        let result = check(&prog, src_path.to_str().unwrap());
        let err = result.expect_err("expected embedded target to reject exec()");
        assert!(err.contains("builtin `exec`"), "unexpected error: {err}");
        assert!(
            err.contains("thumbv7em-none-eabihf"),
            "unexpected error: {err}"
        );
        assert!(err.contains("Tier 3"), "unexpected error: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn embedded_target_by_no_std_feature_rejects_env() {
        let manifest = r#"[package]
name = "a"
version = "1.0.0"
[target.custom-riscv]
features = ["no_std"]
"#;
        let src = "fn main() { env(\"HOME\"); }";
        let (dir, src_path) = write_manifest(manifest, src);
        let (prog, _) = crate::parse(src);
        let result = check(&prog, src_path.to_str().unwrap());
        let err = result.expect_err("expected no_std feature to reject env()");
        assert!(err.contains("builtin `env`"), "unexpected error: {err}");
        assert!(err.contains("custom-riscv"), "unexpected error: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn embedded_target_reports_line_and_column() {
        let manifest = r#"[package]
name = "a"
version = "1.0.0"
[target.thumbv6m-none-eabi]
opt_level = "s"
"#;
        let src = "fn main() {\n    http_get(\"http://x\");\n}\n";
        let (dir, src_path) = write_manifest(manifest, src);
        let (prog, _) = crate::parse(src);
        let result = check(&prog, src_path.to_str().unwrap());
        let err = result.expect_err("expected error");
        assert!(err.contains(":2:"), "expected line 2 in diagnostic: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn core_only_program_is_never_rejected() {
        let manifest = r#"[package]
name = "a"
version = "1.0.0"
[target.thumbv7em-none-eabihf]
opt_level = "s"
"#;
        let src = "fn add(int x, int y) -> int { return x + y; }";
        let (dir, src_path) = write_manifest(manifest, src);
        let (prog, _) = crate::parse(src);
        let result = check(&prog, src_path.to_str().unwrap());
        assert!(result.is_ok(), "unexpected error: {:?}", result);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn multiple_violations_are_all_reported() {
        let manifest = r#"[package]
name = "a"
version = "1.0.0"
[target.riscv32imac-unknown-none-elf]
opt_level = "s"
"#;
        let src = "fn main() {\n    exec(\"ls\");\n    env(\"HOME\");\n}\n";
        let (dir, src_path) = write_manifest(manifest, src);
        let (prog, _) = crate::parse(src);
        let result = check(&prog, src_path.to_str().unwrap());
        let err = result.expect_err("expected error");
        assert!(err.contains("builtin `exec`"), "unexpected error: {err}");
        assert!(err.contains("builtin `env`"), "unexpected error: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tier_table_lookup() {
        assert_eq!(find_tier_builtin("exec").unwrap().tier, 3);
        assert_eq!(find_tier_builtin("env").unwrap().tier, 2);
        assert!(find_tier_builtin("print").is_none());
    }
}
