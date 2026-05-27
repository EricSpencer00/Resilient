//! Debug execution engine for the Resilient DAP server.
//!
//! Wraps the tree-walking interpreter with breakpoint management,
//! step control, call-stack tracking, and scope/variable inspection.
//! The DAP server (`dap_server.rs`) drives execution through this
//! module's public API; all debug state lives here.

use std::collections::HashMap;
use std::sync::mpsc;

use crate::output_sink;
use crate::{Interpreter, Lexer, Node, Parser, Value, run_pending_actors};

/// How the debugger should proceed after a pause.
#[derive(Debug, Clone, PartialEq)]
pub enum StepMode {
    /// Run until the next breakpoint or program end.
    Continue,
    /// Execute one statement without entering function calls.
    StepOver { depth: usize },
    /// Step into the next function call.
    StepIn,
    /// Run until the current function returns.
    StepOut { target_depth: usize },
}

/// A single frame on the debug call stack.
#[derive(Debug, Clone)]
pub struct DebugFrame {
    pub id: u32,
    pub name: String,
    pub file: String,
    pub line: u32,
    pub column: u32,
}

/// A scope within a stack frame (locals, globals, etc.).
#[derive(Debug, Clone)]
pub struct DebugScope {
    pub name: String,
    pub variables_reference: u32,
    pub variables: HashMap<String, String>,
}

/// Why the debugger paused.
#[derive(Debug, Clone)]
pub enum StopReason {
    Breakpoint,
    Step,
    Exception(String),
}

/// Commands sent from the DAP server to the debug execution thread.
#[derive(Debug)]
pub enum DebugCommand {
    Continue,
    StepOver,
    StepIn,
    StepOut,
    Evaluate(String, mpsc::Sender<Result<String, String>>),
    Disconnect,
}

/// Events sent from the debug execution thread back to the DAP server.
#[derive(Debug)]
pub enum DebugEvent {
    Stopped {
        reason: StopReason,
        frames: Vec<DebugFrame>,
        scopes: Vec<DebugScope>,
    },
    Output(String),
    Terminated,
}

/// Breakpoint stored by (file, line).
#[derive(Debug, Clone)]
struct Breakpoint {
    line: u32,
    enabled: bool,
}

/// The debug execution state. Created once per debug session.
pub struct DebugState {
    /// Breakpoints keyed by normalized file path.
    breakpoints: HashMap<String, Vec<Breakpoint>>,
    /// Current step mode.
    step_mode: StepMode,
    /// Current call stack depth (0 = top-level).
    call_depth: usize,
    /// Captured call stack for reporting.
    frames: Vec<DebugFrame>,
    /// Last-known scopes for variable inspection.
    scopes: Vec<DebugScope>,
    /// The source file path being debugged.
    source_file: String,
    /// The source text.
    source_text: String,
    /// Channel to receive commands from the DAP server.
    cmd_rx: mpsc::Receiver<DebugCommand>,
    /// Channel to send events to the DAP server.
    event_tx: mpsc::Sender<DebugEvent>,
    /// Frame ID counter.
    next_frame_id: u32,
    /// Whether the debugger has been asked to disconnect.
    disconnected: bool,
}

impl DebugState {
    pub fn new(
        source_file: String,
        source_text: String,
        cmd_rx: mpsc::Receiver<DebugCommand>,
        event_tx: mpsc::Sender<DebugEvent>,
    ) -> Self {
        DebugState {
            breakpoints: HashMap::new(),
            step_mode: StepMode::StepIn, // pause at first statement
            call_depth: 0,
            frames: Vec::new(),
            scopes: Vec::new(),
            source_file,
            source_text,
            cmd_rx,
            event_tx,
            next_frame_id: 1,
            disconnected: false,
        }
    }

    /// Set breakpoints for a given file. Replaces any existing breakpoints
    /// for that file. Returns the lines that were actually set.
    pub fn set_breakpoints(&mut self, file: &str, lines: &[u32]) -> Vec<u32> {
        let bps: Vec<Breakpoint> = lines
            .iter()
            .map(|&line| Breakpoint {
                line,
                enabled: true,
            })
            .collect();
        let verified: Vec<u32> = bps.iter().map(|bp| bp.line).collect();
        self.breakpoints.insert(file.to_string(), bps);
        verified
    }

    /// Run the program. This is the main debug execution loop.
    /// It parses the source, walks the AST statement by statement,
    /// checking breakpoints and step conditions at each statement.
    pub fn run(&mut self) {
        let lexer = Lexer::new(&self.source_text);
        let mut parser = Parser::new(lexer);
        let program = parser.parse_program();

        if !parser.errors.is_empty() {
            for e in &parser.errors {
                let _ = self
                    .event_tx
                    .send(DebugEvent::Output(format!("Parse error: {}\n", e)));
            }
            let _ = self.event_tx.send(DebugEvent::Terminated);
            return;
        }

        // Extract the top-level statements from the Program node.
        let statements = match &program {
            Node::Program(stmts) => stmts.clone(),
            _ => {
                let _ = self.event_tx.send(DebugEvent::Output(
                    "Internal error: parser did not produce a Program node\n".to_string(),
                ));
                let _ = self.event_tx.send(DebugEvent::Terminated);
                return;
            }
        };

        // Create the interpreter.
        let mut interp = Interpreter::new();

        // First pass: hoist functions (same as eval_program).
        for stmt in &statements {
            if matches!(
                stmt.node,
                Node::Function { .. } | Node::ImplBlock { .. } | Node::ModuleDecl { .. }
            ) && let Err(e) = interp.eval(&stmt.node)
            {
                let _ = self.event_tx.send(DebugEvent::Output(format!(
                    "Error during function hoisting: {}\n",
                    e
                )));
                let _ = self.event_tx.send(DebugEvent::Terminated);
                return;
            }
        }

        // Second pass: execute non-function statements one at a time.
        for stmt in &statements {
            if self.disconnected {
                break;
            }

            // Skip already-hoisted declarations.
            if matches!(
                stmt.node,
                Node::Function { .. } | Node::ImplBlock { .. } | Node::ModuleDecl { .. }
            ) {
                continue;
            }

            let line = stmt.span.start.line as u32;
            let col = stmt.span.start.column as u32;

            // Check if we should pause here.
            let should_pause = self.should_pause(line);

            if should_pause {
                // Build the current frame.
                self.update_frames(&interp, line, col, "<module>");

                // Send stopped event and wait for a command.
                let _ = self.event_tx.send(DebugEvent::Stopped {
                    reason: if self.hit_breakpoint(line) {
                        StopReason::Breakpoint
                    } else {
                        StopReason::Step
                    },
                    frames: self.frames.clone(),
                    scopes: self.scopes.clone(),
                });

                // Wait for the next command.
                if !self.wait_for_command() {
                    break;
                }
            }

            // Execute the statement, capturing output.
            let (result, captured) = output_sink::with_captured_output(|| interp.eval(&stmt.node));

            if !captured.is_empty() {
                let _ = self.event_tx.send(DebugEvent::Output(captured));
            }

            match result {
                Ok(Value::Return(_)) => break,
                Ok(_) => {}
                Err(e) => {
                    // Report the error as an exception stop.
                    self.update_frames(&interp, line, col, "<module>");
                    let _ = self.event_tx.send(DebugEvent::Stopped {
                        reason: StopReason::Exception(e.clone()),
                        frames: self.frames.clone(),
                        scopes: self.scopes.clone(),
                    });
                    // Wait for disconnect or continue.
                    self.wait_for_command();
                    let _ = self
                        .event_tx
                        .send(DebugEvent::Output(format!("Runtime error: {}\n", e)));
                    break;
                }
            }
        }

        // Run any pending actors.
        let (actor_result, actor_out) =
            output_sink::with_captured_output(|| run_pending_actors(&mut interp));
        if !actor_out.is_empty() {
            let _ = self.event_tx.send(DebugEvent::Output(actor_out));
        }
        if let Err(e) = actor_result {
            let _ = self
                .event_tx
                .send(DebugEvent::Output(format!("Actor error: {}\n", e)));
        }

        let _ = self.event_tx.send(DebugEvent::Terminated);
    }

    /// Check if execution should pause at the given line.
    fn should_pause(&self, line: u32) -> bool {
        // Always pause on breakpoint.
        if self.hit_breakpoint(line) {
            return true;
        }

        // Check step mode.
        match &self.step_mode {
            StepMode::Continue => false,
            StepMode::StepIn => true,
            StepMode::StepOver { depth } => self.call_depth <= *depth,
            StepMode::StepOut { target_depth } => self.call_depth < *target_depth,
        }
    }

    /// Check if there is a breakpoint at the given line.
    fn hit_breakpoint(&self, line: u32) -> bool {
        if let Some(bps) = self.breakpoints.get(&self.source_file) {
            return bps.iter().any(|bp| bp.enabled && bp.line == line);
        }
        false
    }

    /// Update the debug frames and scopes from the interpreter state.
    fn update_frames(&mut self, interp: &Interpreter, line: u32, col: u32, name: &str) {
        let frame_id = self.next_frame_id;
        self.next_frame_id += 1;

        // Collect local variables from the interpreter environment.
        let variables = self.collect_variables(interp);

        self.frames = vec![DebugFrame {
            id: frame_id,
            name: name.to_string(),
            file: self.source_file.clone(),
            line,
            column: col,
        }];

        self.scopes = vec![DebugScope {
            name: "Locals".to_string(),
            variables_reference: frame_id,
            variables,
        }];
    }

    /// Collect visible variables from the interpreter's environment.
    fn collect_variables(&self, interp: &Interpreter) -> HashMap<String, String> {
        let mut vars = HashMap::new();
        let names = interp.env.local_names();
        for name in &names {
            // Skip builtins (they start with lowercase and are registered functions).
            if let Some(val) = interp.env.get(name) {
                match &val {
                    Value::Builtin { .. } => continue,
                    Value::Function(_) => {
                        // Include user-defined functions with a short repr.
                        vars.insert(name.clone(), "<function>".to_string());
                    }
                    _ => {
                        vars.insert(name.clone(), format!("{}", val));
                    }
                }
            }
        }
        vars
    }

    /// Wait for a command from the DAP server. Returns false if we
    /// should stop execution (disconnect received).
    fn wait_for_command(&mut self) -> bool {
        loop {
            match self.cmd_rx.recv() {
                Ok(DebugCommand::Continue) => {
                    self.step_mode = StepMode::Continue;
                    return true;
                }
                Ok(DebugCommand::StepOver) => {
                    self.step_mode = StepMode::StepOver {
                        depth: self.call_depth,
                    };
                    return true;
                }
                Ok(DebugCommand::StepIn) => {
                    self.step_mode = StepMode::StepIn;
                    return true;
                }
                Ok(DebugCommand::StepOut) => {
                    self.step_mode = StepMode::StepOut {
                        target_depth: self.call_depth,
                    };
                    return true;
                }
                Ok(DebugCommand::Evaluate(_expr, reply_tx)) => {
                    // For evaluate requests while paused, we can't
                    // easily evaluate in the current interpreter context
                    // without re-entrancy issues. Return a descriptive
                    // message instead.
                    let _ = reply_tx.send(Err(
                        "Expression evaluation during pause is not yet supported".to_string(),
                    ));
                    // Stay in the wait loop.
                }
                Ok(DebugCommand::Disconnect) => {
                    self.disconnected = true;
                    return false;
                }
                Err(_) => {
                    // Channel closed — DAP server gone.
                    self.disconnected = true;
                    return false;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_state_set_breakpoints() {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (event_tx, _event_rx) = mpsc::channel();
        let mut state = DebugState::new(
            "test.rs".to_string(),
            "let x = 1;".to_string(),
            cmd_rx,
            event_tx,
        );
        let verified = state.set_breakpoints("test.rs", &[1, 5, 10]);
        assert_eq!(verified, vec![1, 5, 10]);
        assert!(state.hit_breakpoint(1));
        assert!(state.hit_breakpoint(5));
        assert!(!state.hit_breakpoint(3));
        drop(cmd_tx);
    }

    #[test]
    fn debug_state_step_modes() {
        let (_cmd_tx, cmd_rx) = mpsc::channel();
        let (event_tx, _event_rx) = mpsc::channel();
        let state = DebugState::new("test.rs".to_string(), "".to_string(), cmd_rx, event_tx);
        // Default step mode is StepIn — should pause on any line.
        assert!(state.should_pause(1));
        assert!(state.should_pause(99));
    }

    #[test]
    fn debug_state_continue_mode_no_breakpoint() {
        let (_cmd_tx, cmd_rx) = mpsc::channel();
        let (event_tx, _event_rx) = mpsc::channel();
        let mut state = DebugState::new("test.rs".to_string(), "".to_string(), cmd_rx, event_tx);
        state.step_mode = StepMode::Continue;
        // No breakpoints set — should not pause.
        assert!(!state.should_pause(1));
    }

    #[test]
    fn debug_simple_program_terminates() {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();

        let source = "let x = 42;\nprintln(x);".to_string();
        let mut state = DebugState::new("test.rs".to_string(), source, cmd_rx, event_tx);
        // Set to Continue so it runs without pausing.
        state.step_mode = StepMode::Continue;

        // Run in a thread since it blocks.
        let handle = std::thread::spawn(move || {
            state.run();
        });

        // Collect events.
        let mut got_terminated = false;
        let mut got_output = false;
        while let Ok(event) = event_rx.recv() {
            match event {
                DebugEvent::Output(s) if s.contains("42") => {
                    got_output = true;
                }
                DebugEvent::Terminated => {
                    got_terminated = true;
                    break;
                }
                _ => {}
            }
        }
        drop(cmd_tx);
        handle.join().unwrap();
        assert!(got_terminated);
        assert!(got_output);
    }
}
