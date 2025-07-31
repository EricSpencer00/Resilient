// Enhanced REPL for Resilient language
use crate::{Lexer, Parser, Node, Value};
use crate::typechecker;
use rustyline::error::ReadlineError;
use rustyline::{DefaultEditor, Result as RustylineResult};
use std::env;
use std::path::Path;
use std::io::{self, Write};

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
}

impl EnhancedREPL {
    pub fn new() -> Self {
        // Load history path from environment
        let history_path = match env::var("HOME") {
            Ok(home) => Path::new(&home).join(".resilient_history"),
            Err(_) => Path::new(".resilient_history").to_path_buf(),
        };

        EnhancedREPL {
            interpreter: crate::Interpreter::new(),
            type_check_enabled: false,
            history_path,
        }
    }

    pub fn run(&mut self) -> RustylineResult<()> {
        let mut rl = DefaultEditor::new()?;
        
        // Load command history
        if self.history_path.exists() {
            if let Err(err) = rl.load_history(&self.history_path) {
                eprintln!("Error loading history: {}", err);
            }
        }
        
        println!("{}Resilient Programming Language REPL (v0.1.0){}",
                 CYAN, RESET);
        println!("Type '{}help{}' for command list, '{}exit{}' to quit",
                 GREEN, RESET, RED, RESET);
        
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
                },
                Err(ReadlineError::Interrupted) => {
                    println!("CTRL-C");
                    break;
                },
                Err(ReadlineError::Eof) => {
                    println!("CTRL-D");
                    break;
                },
                Err(err) => {
                    eprintln!("Error: {}", err);
                    break;
                }
            }
        }
        
        // Save history
        if let Err(err) = rl.save_history(&self.history_path) {
            eprintln!("Error saving history: {}", err);
        }
        
        Ok(())
    }
    
    fn process_input(&mut self, input: &str) {
        // Handle special commands
        match input {
            "exit" | "quit" => {
                println!("Exiting Resilient REPL");
                std::process::exit(0);
            },
            "help" => {
                self.show_help();
                return;
            },
            "clear" => {
                print!("\x1B[2J\x1B[1;1H"); // ANSI escape code to clear screen
                io::stdout().flush().unwrap();
                return;
            },
            "typecheck" => {
                self.type_check_enabled = !self.type_check_enabled;
                println!("Type checking {}", 
                         if self.type_check_enabled { 
                             format!("{}enabled{}", GREEN, RESET) 
                         } else { 
                             format!("{}disabled{}", YELLOW, RESET) 
                         });
                return;
            },
            "examples" => {
                self.show_examples();
                return;
            },
            _ => {}
        }
        
        // Regular code evaluation
        let lexer = Lexer::new(input.to_string());
        let mut parser = Parser::new(lexer);
        
        // Parse the program
        let program = parser.parse_program();
        
        // If parser had errors, display them
        if !parser.errors.is_empty() {
            for error in &parser.errors {
                eprintln!("{}Parse error: {}{}", RED, error, RESET);
            }
            return;
        }
        
        // Get the parsed program
        let program = match program {
            Ok(program) => program,
            Err(errors) => {
                for error in errors {
                    eprintln!("{}Parse error: {}{}", RED, error, RESET);
                }
                return;
            }
        };
        
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
            },
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
        println!("  {}examples{}   - Show example code snippets", GREEN, RESET);
        println!("  {}typecheck{}  - Toggle type checking (currently {})", 
                 GREEN, RESET, 
                 if self.type_check_enabled { 
                     format!("{}enabled{}", GREEN, RESET) 
                 } else { 
                     format!("{}disabled{}", YELLOW, RESET) 
                 });
                 
        println!("\n{}Resilient Language Syntax:{}", CYAN, RESET);
        println!("  {}fn name(type param) {{ ... }}{}  - Define a function", YELLOW, RESET);
        println!("  {}let name = value;{}       - Declare a variable", YELLOW, RESET);
        println!("  {}live {{ ... }}{}             - Define a live block", YELLOW, RESET);
        println!("  {}assert(condition, \"msg\");{}  - Add an assertion", YELLOW, RESET);
    }
    
    fn show_examples(&self) {
        println!("{}Example code snippets:{}", CYAN, RESET);
        
        println!("\n{}1. Basic variable and function:{}", GREEN, RESET);
        println!("{}let x = 42;", YELLOW);
        println!("fn add(int a, int b) {{ return a + b; }}", YELLOW);
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
}
