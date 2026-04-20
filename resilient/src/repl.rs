// Enhanced REPL for Resilient language
use crate::typechecker;
use crate::{Lexer, Parser, Value};
use rustyline::config::Config;
use rustyline::error::ReadlineError;
use rustyline::{Editor, Result as RustylineResult};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// RES-224: Maximum number of history entries persisted across REPL sessions.
const HISTORY_MAX_ENTRIES: usize = 1000;

// ANSI color codes for syntax highlighting
const RESET: &str = "\x1B[0m";
const RED: &str = "\x1B[31m";
const GREEN: &str = "\x1B[32m";
const YELLOW: &str = "\x1B[33m";
const BLUE: &str = "\x1B[34m";
const CYAN: &str = "\x1B[36m";

pub struct EnhancedREPL {
    interpreter: crate::Interpreter,
    type_check_enabled: bool,
    history_path: std::path::PathBuf,
    /// RES-026: optional directory of example `.rs` files. When set,
    /// the `examples` REPL command lists files in this directory and
    /// `examples <name>` prints one of them. When `None`, the legacy
    /// hardcoded snippets fire instead.
    examples_dir: Option<PathBuf>,
}

impl EnhancedREPL {
    /// Legacy constructor --- preserved for callers that don't care about
    /// `--examples-dir`. The driver now uses `with_examples_dir`.
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self::with_examples_dir(None)
    }

    /// RES-026: build a REPL pinned to a directory of example files.
    /// Pass `Some(dir)` to wire `--examples-dir <DIR>` from the driver;
    /// pass `None` to keep the original hardcoded-snippet behavior.
    pub fn with_examples_dir(examples_dir: Option<PathBuf>) -> Self {
        // RES-224: Resolve history file path.
        // Priority: $RESILIENT_HISTORY > $HOME/.resilient_history > .resilient_history (cwd)
        let history_path = if let Ok(custom) = env::var("RESILIENT_HISTORY") {
            PathBuf::from(custom)
        } else {
            match env::var("HOME") {
                Ok(home) => Path::new(&home).join(".resilient_history"),
                Err(_) => Path::new(".resilient_history").to_path_buf(),
            }
        };

        EnhancedREPL {
            interpreter: crate::Interpreter::new(),
            type_check_enabled: false,
            history_path,
            examples_dir,
        }
    }

    pub fn run(&mut self) -> RustylineResult<()> {
        // RES-224: configure the editor with the desired history limit.
        let config = Config::builder()
            .max_history_size(HISTORY_MAX_ENTRIES)?
            .build();
        let mut rl = Editor::<(), rustyline::history::DefaultHistory>::with_history(
            config,
            rustyline::history::DefaultHistory::new(),
        )?;

        // RES-224: Load persisted history; silently skip if file is absent.
        if self.history_path.exists()
            && let Err(err) = rl.load_history(&self.history_path)
        {
            eprintln!("Warning: could not load history: {}", err);
        }

        println!(
            "{}Resilient Programming Language REPL (v0.1.0){}",
            CYAN, RESET
        );
        println!(
            "Type '{}help{}' for command list, '{}exit{}' to quit",
            GREEN, RESET, RED, RESET
        );

        loop {
            // Create prompt with type checking indicator
            let prompt = if self.type_check_enabled {
                format!("{}>> [typecheck]{} ", BLUE, RESET)
            } else {
                format!("{}>> {} ", BLUE, RESET)
            };

            // Read input with tab completion
            let readline = rl.readline(&prompt);

            match readline {
                Ok(line) => {
                    let input = line.trim();

                    // Skip empty lines
                    if input.is_empty() {
                        continue;
                    }

                    // Add to history
                    rl.add_history_entry(input)?;

                    // Process the input
                    self.process_input(input);
                }
                Err(ReadlineError::Interrupted) => {
                    println!("CTRL-C");
                    break;
                }
                Err(ReadlineError::Eof) => {
                    println!("CTRL-D");
                    break;
                }
                Err(err) => {
                    eprintln!("Error: {}", err);
                    break;
                }
            }
        }

        // RES-224: Persist history; emit a warning on failure but do not crash.
        if let Err(err) = rl.save_history(&self.history_path) {
            eprintln!(
                "Warning: could not save history to {}: {}",
                self.history_path.display(),
                err
            );
        }

        Ok(())
    }

    fn process_input(&mut self, input: &str) {
        // Handle special commands
        match input {
            "exit" | "quit" => {
                println!("Exiting Resilient REPL");
                std::process::exit(0);
            }
            "help" => {
                self.show_help();
                return;
            }
            "clear" => {
                print!("\x1B[2J\x1B[1;1H"); // ANSI escape code to clear screen
                io::stdout().flush().unwrap();
                return;
            }
            "typecheck" => {
                self.type_check_enabled = !self.type_check_enabled;
                println!(
                    "Type checking {}",
                    if self.type_check_enabled {
                        format!("{}enabled{}", GREEN, RESET)
                    } else {
                        format!("{}disabled{}", YELLOW, RESET)
                    }
                );
                return;
            }
            "examples" => {
                self.show_examples();
                return;
            }
            _ => {}
        }

        // RES-026: `examples <name>` subcommand. Falls through to
        // regular code evaluation only when the dir mode isn't set ---
        // otherwise it's a typo and we say so.
        if let Some(rest) = input.strip_prefix("examples ") {
            self.show_named_example(rest.trim());
            return;
        }

        // Regular code evaluation
        let lexer = Lexer::new(input.to_string());
        let mut parser = Parser::new(lexer);

        // Parse the program
        let program = parser.parse_program();

        // If parser recorded errors, abort before type-checking/execution.
        // Errors are already printed as they happen inside the parser.
        if !parser.errors.is_empty() {
            return;
        }

        // Run type checker if enabled
        if self.type_check_enabled {
            match typechecker::TypeChecker::new().check_program(&program) {
                Ok(_) => println!("{}Type check passed{}", GREEN, RESET),
                Err(e) => {
                    eprintln!("{}Type error: {}{}", RED, e, RESET);
                    return; // Skip execution if type checking fails
                }
            }
        }

        // Evaluate the program
        match self.interpreter.eval(&program) {
            Ok(value) => {
                if !matches!(value, Value::Void) {
                    println!("{}{}{}", CYAN, value, RESET);
                }
            }
            Err(error) => {
                eprintln!("{}Error: {}{}", RED, error, RESET);
            }
        }
    }

    fn show_help(&self) {
        println!("{}Available commands:{}", CYAN, RESET);
        println!("  {}help{}       - Show this help message", GREEN, RESET);
        println!("  {}exit{}       - Exit the REPL", GREEN, RESET);
        println!("  {}clear{}      - Clear the screen", GREEN, RESET);
        if self.examples_dir.is_some() {
            println!(
                "  {}examples{}        - List example files in --examples-dir",
                GREEN, RESET
            );
            println!(
                "  {}examples <name>{} - Print the contents of one example file",
                GREEN, RESET
            );
        } else {
            println!(
                "  {}examples{}   - Show example code snippets",
                GREEN, RESET
            );
        }
        println!(
            "  {}typecheck{}  - Toggle type checking (currently {})",
            GREEN,
            RESET,
            if self.type_check_enabled {
                format!("{}enabled{}", GREEN, RESET)
            } else {
                format!("{}disabled{}", YELLOW, RESET)
            }
        );

        println!("\n{}Resilient Language Syntax:{}", CYAN, RESET);
        println!(
            "  {}fn name(type param) {{ ... }}{}  - Define a function",
            YELLOW, RESET
        );
        println!(
            "  {}let name = value;{}       - Declare a variable",
            YELLOW, RESET
        );
        println!(
            "  {}live {{ ... }}{}             - Define a live block",
            YELLOW, RESET
        );
        println!(
            "  {}assert(condition, \"msg\");{}  - Add an assertion",
            YELLOW, RESET
        );
    }

    fn show_examples(&self) {
        // RES-026: prefer the dynamic listing when --examples-dir was
        // wired; fall back to the legacy hardcoded snippets so the
        // bare REPL still does something useful.
        if let Some(dir) = &self.examples_dir {
            match Self::list_examples_in(dir) {
                Ok(text) => {
                    println!("{}Example files in {}:{}", CYAN, dir.display(), RESET);
                    print!("{}", text);
                }
                Err(e) => {
                    eprintln!("{}examples: {}{}", RED, e, RESET);
                }
            }
            return;
        }

        println!("{}Example code snippets:{}", CYAN, RESET);

        println!("\n{}1. Basic variable and function:{}", GREEN, RESET);
        println!("{}let x = 42;", YELLOW);
        println!("fn add(int a, int b) {{ return a + b; }}");
        println!("add(x, 10);{}", RESET);

        println!("\n{}2. Live block example:{}", GREEN, RESET);
        println!("{}live {{", YELLOW);
        println!("  let result = 100 / 0; // This would normally crash");
        println!("  println(\"Result: \" + result);");
        println!("}}{}", RESET);

        println!("\n{}3. Assertion example:{}", GREEN, RESET);
        println!("{}let age = 25;", YELLOW);
        println!("assert(age >= 18, \"Must be an adult\");");
        println!("println(\"Access granted\");{}", RESET);
    }

    /// RES-026: handle `examples <name>` --- print the contents of a
    /// single example file. `name` is treated as a basename only;
    /// any '/' or '..' is rejected up front.
    fn show_named_example(&self, name: &str) {
        let Some(dir) = &self.examples_dir else {
            eprintln!(
                "{}examples: '{}' subcommand requires --examples-dir{}",
                RED, name, RESET
            );
            return;
        };
        if name.contains('/') || name.contains("..") || name.is_empty() {
            eprintln!(
                "{}examples: name must be a single basename, not a path{}",
                RED, RESET
            );
            return;
        }
        let candidate = if name.ends_with(".res") {
            dir.join(name)
        } else {
            dir.join(format!("{}.res", name))
        };
        match fs::read_to_string(&candidate) {
            Ok(body) => {
                println!("{}--- {} ---{}", CYAN, candidate.display(), RESET);
                print!("{}", body);
                if !body.ends_with('\n') {
                    println!();
                }
            }
            Err(_) => {
                eprintln!(
                    "{}examples: no such file '{}' in {}{}",
                    RED,
                    name,
                    dir.display(),
                    RESET
                );
            }
        }
    }

    /// RES-026: pure helper --- returns the example listing as a String
    /// so unit tests can assert on it without fighting stdout capture.
    /// Sorted alphabetically; one basename per line.
    pub(crate) fn list_examples_in(dir: &Path) -> Result<String, String> {
        let entries =
            fs::read_dir(dir).map_err(|e| format!("could not read {}: {}", dir.display(), e))?;
        let mut names: Vec<String> = entries
            .flatten()
            .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("res"))
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        names.sort();
        let mut out = String::new();
        for n in names {
            out.push_str("  ");
            out.push_str(&n);
            out.push('\n');
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tmp(label: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("res_026_{}_{}", label, std::process::id()));
        // Wipe any leftover from a prior run so each test starts clean.
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).expect("create tmp dir");
        p
    }

    #[test]
    fn list_examples_in_returns_sorted_basenames() {
        let dir = make_tmp("listing");
        fs::write(dir.join("foo.res"), "fn main() {}\n").unwrap();
        fs::write(dir.join("alpha.res"), "fn main() {}\n").unwrap();
        fs::write(dir.join("ignored.txt"), "not resilient\n").unwrap();

        let listing = EnhancedREPL::list_examples_in(&dir).unwrap();
        assert!(
            listing.contains("alpha.res"),
            "missing alpha.res:\n{}",
            listing
        );
        assert!(listing.contains("foo.res"), "missing foo.res:\n{}", listing);
        assert!(
            !listing.contains("ignored.txt"),
            "non-.res file should be filtered:\n{}",
            listing
        );
        // Alphabetical: alpha must precede foo.
        let a = listing.find("alpha.res").unwrap();
        let f = listing.find("foo.res").unwrap();
        assert!(a < f, "expected alpha before foo:\n{}", listing);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_examples_in_errors_cleanly_on_missing_dir() {
        let bogus = std::env::temp_dir().join("res_026_definitely_not_here");
        let _ = fs::remove_dir_all(&bogus);
        let err = EnhancedREPL::list_examples_in(&bogus).expect_err("missing dir must error");
        assert!(err.contains("could not read"), "got: {}", err);
    }

    #[test]
    fn with_examples_dir_stores_the_path() {
        let dir = make_tmp("ctor");
        let repl = EnhancedREPL::with_examples_dir(Some(dir.clone()));
        assert_eq!(repl.examples_dir.as_deref(), Some(dir.as_path()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_constructor_leaves_examples_dir_unset() {
        let repl = EnhancedREPL::new();
        assert!(repl.examples_dir.is_none());
    }

    // -- RES-224: history persistence --------------------------------------

    /// RESILIENT_HISTORY env var overrides the default ~/.resilient_history path.
    #[test]
    fn history_path_uses_resilient_history_env_var() {
        let tmp =
            std::env::temp_dir().join(format!("res_224_custom_history_{}", std::process::id()));
        // Set env var, build REPL, then restore environment state.
        // SAFETY: test binary is single-threaded in this test; env mutation is acceptable.
        unsafe {
            std::env::set_var("RESILIENT_HISTORY", tmp.to_str().unwrap());
        }
        let repl = EnhancedREPL::with_examples_dir(None);
        unsafe {
            std::env::remove_var("RESILIENT_HISTORY");
        }
        assert_eq!(
            repl.history_path, tmp,
            "history_path should match RESILIENT_HISTORY env var"
        );
    }

    /// When RESILIENT_HISTORY is absent the history filename defaults to
    /// `.resilient_history` (regardless of the parent directory).
    #[test]
    fn history_path_defaults_to_home_dir() {
        unsafe {
            std::env::remove_var("RESILIENT_HISTORY");
        }
        let repl = EnhancedREPL::with_examples_dir(None);
        let filename = repl
            .history_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        assert_eq!(
            filename, ".resilient_history",
            "default history file should be named .resilient_history"
        );
    }

    /// HISTORY_MAX_ENTRIES constant must be exactly 1000.
    #[test]
    fn history_max_entries_constant_is_1000() {
        assert_eq!(HISTORY_MAX_ENTRIES, 1000);
    }
}
