//! RES-4115 (E-E4 increment 1): `rz explain E####` and `rz errors list`.
//!
//! Single source of truth for both commands is the checked-in
//! `docs/errors/E00NN.md` page — the same file the docs site
//! renders. `include_str!` embeds each page at compile time so
//! `rz explain` works from any working directory and any install
//! layout (no runtime dependency on the repo checkout being
//! present next to the binary).
//!
//! A generated-docs pipeline (docs/errors/*.md produced *from*
//! `diag::codes` instead of hand-authored) is the natural follow-up
//! once the registry is the single source of truth end to end; for
//! now the docs page is authoritative and this module just renders
//! it in the terminal.

use crate::diag::codes;

/// Maps an `E00NN` code to its embedded docs page source, or `None`
/// for an unregistered code. Kept as an explicit match (rather than
/// a macro over `codes::all()`) so `include_str!`'s compile-time
/// path checking catches a missing docs page immediately.
fn doc_for_code(code: &str) -> Option<&'static str> {
    Some(match code {
        "E0001" => include_str!("../../docs/errors/E0001.md"),
        "E0002" => include_str!("../../docs/errors/E0002.md"),
        "E0003" => include_str!("../../docs/errors/E0003.md"),
        "E0004" => include_str!("../../docs/errors/E0004.md"),
        "E0005" => include_str!("../../docs/errors/E0005.md"),
        "E0006" => include_str!("../../docs/errors/E0006.md"),
        "E0007" => include_str!("../../docs/errors/E0007.md"),
        "E0008" => include_str!("../../docs/errors/E0008.md"),
        "E0009" => include_str!("../../docs/errors/E0009.md"),
        "E0010" => include_str!("../../docs/errors/E0010.md"),
        "E0011" => include_str!("../../docs/errors/E0011.md"),
        "E0012" => include_str!("../../docs/errors/E0012.md"),
        "E0013" => include_str!("../../docs/errors/E0013.md"),
        "E0014" => include_str!("../../docs/errors/E0014.md"),
        "E0015" => include_str!("../../docs/errors/E0015.md"),
        "E0016" => include_str!("../../docs/errors/E0016.md"),
        "E0017" => include_str!("../../docs/errors/E0017.md"),
        "E0018" => include_str!("../../docs/errors/E0018.md"),
        "E0019" => include_str!("../../docs/errors/E0019.md"),
        "E0020" => include_str!("../../docs/errors/E0020.md"),
        _ => return None,
    })
}

/// Strip a leading `---\n...\n---\n` YAML front-matter block (Jekyll
/// page metadata) so `rz explain` prints only the human-readable
/// body. If the doc doesn't start with a front-matter fence, the
/// text is returned unchanged.
fn strip_front_matter(doc: &str) -> &str {
    let Some(rest) = doc.strip_prefix("---\n") else {
        return doc;
    };
    match rest.find("\n---\n") {
        Some(idx) => rest[idx + 5..].trim_start_matches('\n'),
        None => doc,
    }
}

/// `rz explain <CODE>` — print the long-form explanation for a
/// registered error code. `rz errors list` — print every registered
/// code with its one-line summary.
///
/// Returns `None` when `args` doesn't invoke either subcommand, so
/// the caller's dispatch chain falls through to the next handler.
pub fn dispatch_explain_subcommand(args: &[String]) -> Option<i32> {
    match args.get(1).map(String::as_str) {
        Some("explain") => Some(run_explain(args.get(2).map(String::as_str))),
        Some("errors") => match args.get(2).map(String::as_str) {
            Some("list") | None => Some(run_errors_list()),
            Some("--help") | Some("-h") => {
                print_explain_help();
                Some(0)
            }
            Some(other) => {
                eprintln!("error: unknown `rz errors` subcommand `{other}`");
                eprintln!("Try `rz errors list`.");
                Some(1)
            }
        },
        _ => None,
    }
}

fn run_explain(code_arg: Option<&str>) -> i32 {
    let Some(raw) = code_arg else {
        print_explain_help();
        return 1;
    };
    if raw == "--help" || raw == "-h" {
        print_explain_help();
        return 0;
    }
    let code = raw.to_ascii_uppercase();
    match doc_for_code(&code) {
        Some(doc) => {
            println!("{}", strip_front_matter(doc).trim_end());
            0
        }
        None => {
            eprintln!("error: unknown error code `{raw}`");
            eprintln!("Run `rz errors list` to see every registered code.");
            1
        }
    }
}

fn run_errors_list() -> i32 {
    println!("Registered Resilient diagnostic codes:\n");
    for code in codes::all() {
        let summary = doc_for_code(code.as_str())
            .and_then(|doc| {
                strip_front_matter(doc)
                    .lines()
                    .find(|l| l.starts_with("# "))
            })
            .map(|title_line| title_line.trim_start_matches("# ").to_string())
            .unwrap_or_else(|| code.to_string());
        println!("  {:<8} {}", code.as_str(), summary);
    }
    println!("\nRun `rz explain <CODE>` for the full explanation of any code.");
    0
}

fn print_explain_help() {
    println!(
        "Usage: rz explain <CODE>\n       rz errors list\n\n\
         Print the long-form explanation for a Resilient diagnostic\n\
         code (e.g. `rz explain E0007`), or list every registered\n\
         code with `rz errors list`.\n\n\
         See also: docs/errors/ (the same content, rendered as a\n\
         static site)."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explain_prints_known_code_body() {
        let code = "explain".to_string();
        let arg = "E0007".to_string();
        let args = vec!["rz".to_string(), code, arg];
        assert_eq!(dispatch_explain_subcommand(&args), Some(0));
    }

    #[test]
    fn explain_rejects_unknown_code() {
        let args = vec!["rz".to_string(), "explain".to_string(), "E9999".to_string()];
        assert_eq!(dispatch_explain_subcommand(&args), Some(1));
    }

    #[test]
    fn explain_is_case_insensitive() {
        let args = vec!["rz".to_string(), "explain".to_string(), "e0001".to_string()];
        assert_eq!(dispatch_explain_subcommand(&args), Some(0));
    }

    #[test]
    fn explain_without_code_shows_help_and_fails() {
        let args = vec!["rz".to_string(), "explain".to_string()];
        assert_eq!(dispatch_explain_subcommand(&args), Some(1));
    }

    #[test]
    fn errors_list_runs_and_covers_every_registered_code() {
        let args = vec!["rz".to_string(), "errors".to_string(), "list".to_string()];
        assert_eq!(dispatch_explain_subcommand(&args), Some(0));
        // Every code in the registry must have an embedded doc page,
        // otherwise `doc_for_code` silently falls back to the bare
        // code as its own "summary" — assert real docs exist for all.
        for code in codes::all() {
            assert!(
                doc_for_code(code.as_str()).is_some(),
                "missing embedded docs page for {}",
                code.as_str()
            );
        }
    }

    #[test]
    fn non_explain_args_fall_through() {
        let args = vec!["rz".to_string(), "run".to_string(), "foo.rz".to_string()];
        assert_eq!(dispatch_explain_subcommand(&args), None);
    }

    #[test]
    fn front_matter_is_stripped() {
        let doc = "---\ntitle: X\n---\n\n# Heading\nbody\n";
        assert_eq!(strip_front_matter(doc), "# Heading\nbody\n");
    }

    #[test]
    fn front_matter_missing_is_passthrough() {
        let doc = "# Heading\nbody\n";
        assert_eq!(strip_front_matter(doc), doc);
    }
}
