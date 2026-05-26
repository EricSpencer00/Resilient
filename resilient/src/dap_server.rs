//! Debug Adapter Protocol (DAP) server for Resilient.
//!
//! Speaks the DAP JSON wire protocol over stdin/stdout using the same
//! `Content-Length: N\r\n\r\n{json}` framing as LSP. A DAP client
//! (VS Code, nvim-dap, etc.) launches `rz --dap <file>` as a child
//! process and drives the session through this protocol.
//!
//! The server delegates actual program execution to `debugger::DebugState`,
//! which runs on a background thread and communicates via channels.

use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, Write};
use std::sync::mpsc;
use std::thread;

use serde_json::{Value, json};

use crate::debugger::{DebugCommand, DebugEvent, DebugFrame, DebugScope, DebugState, StopReason};

// ── DAP message framing ──────────────────────────────────────────────────────

/// Read a single DAP message from `reader`. Returns `None` on EOF.
fn read_message(reader: &mut impl BufRead) -> Option<Value> {
    // Read headers until blank line.
    let mut content_length: usize = 0;
    loop {
        let mut header = String::new();
        match reader.read_line(&mut header) {
            Ok(0) => return None, // EOF
            Err(_) => return None,
            Ok(_) => {}
        }
        let trimmed = header.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some(val) = trimmed.strip_prefix("Content-Length:")
            && let Ok(n) = val.trim().parse::<usize>()
        {
            content_length = n;
        }
    }

    if content_length == 0 {
        return None;
    }

    // Read exactly content_length bytes.
    let mut body = vec![0u8; content_length];
    if reader.read_exact(&mut body).is_err() {
        return None;
    }

    serde_json::from_slice(&body).ok()
}

/// Write a DAP message to `writer` with the Content-Length header.
fn write_message(writer: &mut impl Write, msg: &Value) -> io::Result<()> {
    let body =
        serde_json::to_string(msg).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    write!(writer, "Content-Length: {}\r\n\r\n{}", body.len(), body)?;
    writer.flush()
}

// ── Sequence counter ─────────────────────────────────────────────────────────

struct SeqCounter {
    next: i64,
}

impl SeqCounter {
    fn new() -> Self {
        SeqCounter { next: 1 }
    }

    fn next(&mut self) -> i64 {
        let seq = self.next;
        self.next += 1;
        seq
    }
}

// ── DAP server state ─────────────────────────────────────────────────────────

struct DapServer {
    seq: SeqCounter,
    /// Channel to send commands to the debug execution thread.
    cmd_tx: Option<mpsc::Sender<DebugCommand>>,
    /// Channel to receive events from the debug execution thread.
    event_rx: Option<mpsc::Receiver<DebugEvent>>,
    /// Program path received from the launch request.
    program_path: Option<String>,
    /// Pending breakpoints set before launch.
    pending_breakpoints: HashMap<String, Vec<u32>>,
    /// Last-known debug frames (updated on stop events).
    frames: Vec<DebugFrame>,
    /// Last-known debug scopes (updated on stop events).
    scopes: Vec<DebugScope>,
    /// Whether the program has been launched.
    launched: bool,
    /// Whether the program has terminated.
    terminated: bool,
}

impl DapServer {
    fn new() -> Self {
        DapServer {
            seq: SeqCounter::new(),
            cmd_tx: None,
            event_rx: None,
            program_path: None,
            pending_breakpoints: HashMap::new(),
            frames: Vec::new(),
            scopes: Vec::new(),
            launched: false,
            terminated: false,
        }
    }

    /// Build a DAP response message.
    fn response(
        &mut self,
        request_seq: i64,
        command: &str,
        success: bool,
        body: Option<Value>,
    ) -> Value {
        let mut resp = json!({
            "seq": self.seq.next(),
            "type": "response",
            "request_seq": request_seq,
            "success": success,
            "command": command,
        });
        if let Some(b) = body {
            resp["body"] = b;
        }
        resp
    }

    /// Build a DAP event message.
    fn event(&mut self, event_name: &str, body: Option<Value>) -> Value {
        let mut evt = json!({
            "seq": self.seq.next(),
            "type": "event",
            "event": event_name,
        });
        if let Some(b) = body {
            evt["body"] = b;
        }
        evt
    }

    /// Handle an initialize request.
    fn handle_initialize(&mut self, request_seq: i64) -> Vec<Value> {
        let resp = self.response(
            request_seq,
            "initialize",
            true,
            Some(json!({
                "supportsConfigurationDoneRequest": true,
                "supportsFunctionBreakpoints": false,
                "supportsConditionalBreakpoints": false,
                "supportsEvaluateForHovers": false,
                "supportsStepBack": false,
                "supportsSetVariable": false,
                "supportsRestartFrame": false,
                "supportsGotoTargetsRequest": false,
                "supportsStepInTargetsRequest": false,
                "supportsCompletionsRequest": false,
                "supportsModulesRequest": false,
                "supportsExceptionOptions": false,
                "supportsTerminateRequest": true,
            })),
        );
        let initialized_event = self.event("initialized", None);
        vec![resp, initialized_event]
    }

    /// Handle a launch request.
    fn handle_launch(&mut self, request_seq: i64, args: &Value) -> Vec<Value> {
        let program = args
            .get("program")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if program.is_empty() {
            return vec![self.response(
                request_seq,
                "launch",
                false,
                Some(json!({ "message": "No program path specified" })),
            )];
        }

        self.program_path = Some(program);
        vec![self.response(request_seq, "launch", true, None)]
    }

    /// Handle setBreakpoints request.
    fn handle_set_breakpoints(&mut self, request_seq: i64, args: &Value) -> Vec<Value> {
        let source_path = args
            .get("source")
            .and_then(|s| s.get("path"))
            .and_then(|p| p.as_str())
            .unwrap_or("")
            .to_string();

        let lines: Vec<u32> = args
            .get("breakpoints")
            .and_then(|bps| bps.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|bp| bp.get("line").and_then(|l| l.as_u64()).map(|l| l as u32))
                    .collect()
            })
            .unwrap_or_default();

        // Store breakpoints. If the debug state is already running,
        // update it; otherwise save for when the session starts.
        self.pending_breakpoints
            .insert(source_path.clone(), lines.clone());

        let breakpoints: Vec<Value> = lines
            .iter()
            .map(|&line| {
                json!({
                    "id": line,
                    "verified": true,
                    "line": line,
                    "source": { "path": &source_path }
                })
            })
            .collect();

        vec![self.response(
            request_seq,
            "setBreakpoints",
            true,
            Some(json!({ "breakpoints": breakpoints })),
        )]
    }

    /// Handle configurationDone — start execution.
    fn handle_configuration_done(&mut self, request_seq: i64) -> Vec<Value> {
        let resp = self.response(request_seq, "configurationDone", true, None);

        // Now start the debug execution.
        if let Some(ref program_path) = self.program_path.clone() {
            self.start_execution(program_path);
        }

        vec![resp]
    }

    /// Start the debug execution thread.
    fn start_execution(&mut self, program_path: &str) {
        let source_text = match fs::read_to_string(program_path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to read {}: {}", program_path, e);
                return;
            }
        };

        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();

        let mut debug_state =
            DebugState::new(program_path.to_string(), source_text, cmd_rx, event_tx);

        // Apply any pending breakpoints.
        for (file, lines) in &self.pending_breakpoints {
            debug_state.set_breakpoints(file, lines);
        }

        self.cmd_tx = Some(cmd_tx);
        self.event_rx = Some(event_rx);
        self.launched = true;

        // Spawn the execution thread.
        thread::spawn(move || {
            debug_state.run();
        });
    }

    /// Handle continue request.
    fn handle_continue(&mut self, request_seq: i64) -> Vec<Value> {
        if let Some(ref tx) = self.cmd_tx {
            let _ = tx.send(DebugCommand::Continue);
        }
        vec![self.response(
            request_seq,
            "continue",
            true,
            Some(json!({ "allThreadsContinued": true })),
        )]
    }

    /// Handle next (step over) request.
    fn handle_next(&mut self, request_seq: i64) -> Vec<Value> {
        if let Some(ref tx) = self.cmd_tx {
            let _ = tx.send(DebugCommand::StepOver);
        }
        vec![self.response(request_seq, "next", true, None)]
    }

    /// Handle stepIn request.
    fn handle_step_in(&mut self, request_seq: i64) -> Vec<Value> {
        if let Some(ref tx) = self.cmd_tx {
            let _ = tx.send(DebugCommand::StepIn);
        }
        vec![self.response(request_seq, "stepIn", true, None)]
    }

    /// Handle stepOut request.
    fn handle_step_out(&mut self, request_seq: i64) -> Vec<Value> {
        if let Some(ref tx) = self.cmd_tx {
            let _ = tx.send(DebugCommand::StepOut);
        }
        vec![self.response(request_seq, "stepOut", true, None)]
    }

    /// Handle threads request.
    fn handle_threads(&mut self, request_seq: i64) -> Vec<Value> {
        vec![self.response(
            request_seq,
            "threads",
            true,
            Some(json!({
                "threads": [
                    { "id": 1, "name": "main" }
                ]
            })),
        )]
    }

    /// Handle stackTrace request.
    fn handle_stack_trace(&mut self, request_seq: i64) -> Vec<Value> {
        let stack_frames: Vec<Value> = self
            .frames
            .iter()
            .map(|f| {
                json!({
                    "id": f.id,
                    "name": f.name,
                    "source": { "path": f.file },
                    "line": f.line,
                    "column": f.column,
                })
            })
            .collect();

        vec![self.response(
            request_seq,
            "stackTrace",
            true,
            Some(json!({
                "stackFrames": stack_frames,
                "totalFrames": stack_frames.len(),
            })),
        )]
    }

    /// Handle scopes request.
    fn handle_scopes(&mut self, request_seq: i64, args: &Value) -> Vec<Value> {
        let frame_id = args.get("frameId").and_then(|v| v.as_u64()).unwrap_or(0) as u32;

        let scopes: Vec<Value> = self
            .scopes
            .iter()
            .filter(|_| {
                // Return scopes that belong to this frame.
                // Since we use frame_id as variables_reference, match on that.
                self.frames.iter().any(|f| f.id == frame_id)
            })
            .map(|s| {
                json!({
                    "name": s.name,
                    "variablesReference": s.variables_reference,
                    "expensive": false,
                })
            })
            .collect();

        vec![self.response(
            request_seq,
            "scopes",
            true,
            Some(json!({ "scopes": scopes })),
        )]
    }

    /// Handle variables request.
    fn handle_variables(&mut self, request_seq: i64, args: &Value) -> Vec<Value> {
        let var_ref = args
            .get("variablesReference")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        // Find the scope with this variables reference.
        let variables: Vec<Value> = self
            .scopes
            .iter()
            .find(|s| s.variables_reference == var_ref)
            .map(|scope| {
                let mut vars: Vec<Value> = scope
                    .variables
                    .iter()
                    .map(|(name, value)| {
                        json!({
                            "name": name,
                            "value": value,
                            "variablesReference": 0,
                        })
                    })
                    .collect();
                vars.sort_by(|a, b| {
                    a.get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .cmp(b.get("name").and_then(|v| v.as_str()).unwrap_or(""))
                });
                vars
            })
            .unwrap_or_default();

        vec![self.response(
            request_seq,
            "variables",
            true,
            Some(json!({ "variables": variables })),
        )]
    }

    /// Handle evaluate request.
    fn handle_evaluate(&mut self, request_seq: i64, args: &Value) -> Vec<Value> {
        let expression = args
            .get("expression")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if let Some(ref tx) = self.cmd_tx {
            let (reply_tx, reply_rx) = mpsc::channel();
            if tx
                .send(DebugCommand::Evaluate(expression, reply_tx))
                .is_ok()
                && let Ok(result) = reply_rx.recv_timeout(std::time::Duration::from_secs(5))
            {
                return match result {
                    Ok(val) => vec![self.response(
                        request_seq,
                        "evaluate",
                        true,
                        Some(json!({
                            "result": val,
                            "variablesReference": 0,
                        })),
                    )],
                    Err(e) => vec![self.response(
                        request_seq,
                        "evaluate",
                        false,
                        Some(json!({ "message": e })),
                    )],
                };
            }
        }

        vec![self.response(
            request_seq,
            "evaluate",
            false,
            Some(json!({ "message": "No active debug session" })),
        )]
    }

    /// Handle disconnect request.
    fn handle_disconnect(&mut self, request_seq: i64) -> Vec<Value> {
        if let Some(ref tx) = self.cmd_tx {
            let _ = tx.send(DebugCommand::Disconnect);
        }
        self.cmd_tx = None;
        self.event_rx = None;
        vec![self.response(request_seq, "disconnect", true, None)]
    }

    /// Handle terminate request.
    fn handle_terminate(&mut self, request_seq: i64) -> Vec<Value> {
        if let Some(ref tx) = self.cmd_tx {
            let _ = tx.send(DebugCommand::Disconnect);
        }
        vec![self.response(request_seq, "terminate", true, None)]
    }

    /// Dispatch a DAP request and return the response messages.
    fn dispatch(&mut self, msg: &Value) -> Vec<Value> {
        let msg_type = msg.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let command = msg.get("command").and_then(|c| c.as_str()).unwrap_or("");
        let request_seq = msg.get("seq").and_then(|s| s.as_i64()).unwrap_or(0);
        let args = msg.get("arguments").cloned().unwrap_or_else(|| json!({}));

        if msg_type != "request" {
            return Vec::new();
        }

        match command {
            "initialize" => self.handle_initialize(request_seq),
            "launch" => self.handle_launch(request_seq, &args),
            "setBreakpoints" => self.handle_set_breakpoints(request_seq, &args),
            "configurationDone" => self.handle_configuration_done(request_seq),
            "continue" => self.handle_continue(request_seq),
            "next" => self.handle_next(request_seq),
            "stepIn" => self.handle_step_in(request_seq),
            "stepOut" => self.handle_step_out(request_seq),
            "threads" => self.handle_threads(request_seq),
            "stackTrace" => self.handle_stack_trace(request_seq),
            "scopes" => self.handle_scopes(request_seq, &args),
            "variables" => self.handle_variables(request_seq, &args),
            "evaluate" => self.handle_evaluate(request_seq, &args),
            "disconnect" => self.handle_disconnect(request_seq),
            "terminate" => self.handle_terminate(request_seq),
            // Unknown commands get a success response to avoid
            // stalling the client.
            _ => vec![self.response(request_seq, command, true, None)],
        }
    }

    /// Drain any pending debug events and return DAP event messages.
    fn drain_events(&mut self) -> Vec<Value> {
        // Collect raw events from the channel first to release the
        // immutable borrow on `self.event_rx` before we call
        // `self.event()` which needs `&mut self`.
        let raw_events: Vec<DebugEvent> = {
            let rx = match self.event_rx {
                Some(ref rx) => rx,
                None => return Vec::new(),
            };
            let mut collected = Vec::new();
            while let Ok(event) = rx.try_recv() {
                collected.push(event);
            }
            collected
        };

        let mut events = Vec::new();
        for debug_event in raw_events {
            match debug_event {
                DebugEvent::Stopped {
                    reason,
                    frames,
                    scopes,
                } => {
                    self.frames = frames;
                    self.scopes = scopes;

                    let (reason_str, description, text) = match &reason {
                        StopReason::Breakpoint => {
                            ("breakpoint", "Paused on breakpoint".to_string(), None)
                        }
                        StopReason::Step => ("step", "Paused after step".to_string(), None),
                        StopReason::Exception(msg) => (
                            "exception",
                            format!("Exception: {}", msg),
                            Some(msg.clone()),
                        ),
                    };

                    let mut body = json!({
                        "reason": reason_str,
                        "description": description,
                        "threadId": 1,
                        "allThreadsStopped": true,
                    });
                    if let Some(t) = text {
                        body["text"] = json!(t);
                    }
                    events.push(self.event("stopped", Some(body)));
                }
                DebugEvent::Output(text) => {
                    events.push(self.event(
                        "output",
                        Some(json!({
                            "category": "stdout",
                            "output": text,
                        })),
                    ));
                }
                DebugEvent::Terminated => {
                    self.terminated = true;
                    events.push(self.event("terminated", None));
                }
            }
        }

        events
    }
}

// ── Public entry point ───────────────────────────────────────────────────────

/// Run the DAP server on stdin/stdout. Called from `run_cli()` when
/// `--dap` is passed.
pub fn run() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = io::BufReader::new(stdin.lock());
    let mut writer = io::BufWriter::new(stdout.lock());

    let mut server = DapServer::new();

    loop {
        // First, drain any pending events from the debug thread.
        for evt in server.drain_events() {
            if write_message(&mut writer, &evt).is_err() {
                return;
            }
        }

        // If the program has terminated and there is no active session,
        // we can still accept disconnect/terminate requests.

        // Try to read a message with a short timeout. We use a non-blocking
        // approach: check if there's data available, process it, then
        // loop back to drain events.
        //
        // For simplicity and correctness, we use a blocking read here.
        // The DAP client drives the conversation, so we always have a
        // pending request or the session is done.
        let msg = match read_message(&mut reader) {
            Some(m) => m,
            None => break, // EOF — client disconnected.
        };

        let responses = server.dispatch(&msg);
        for resp in &responses {
            if write_message(&mut writer, resp).is_err() {
                return;
            }
        }

        // After handling a request that might have triggered execution
        // (configurationDone, continue, step*), give the debug thread
        // a moment to produce events, then drain them.
        let command = msg.get("command").and_then(|c| c.as_str()).unwrap_or("");
        if matches!(
            command,
            "configurationDone" | "continue" | "next" | "stepIn" | "stepOut"
        ) {
            // Brief yield to let the debug thread run.
            std::thread::sleep(std::time::Duration::from_millis(10));
            for evt in server.drain_events() {
                if write_message(&mut writer, &evt).is_err() {
                    return;
                }
            }
        }

        if command == "disconnect" {
            break;
        }
    }
}

/// Dispatch the `--dap` CLI flag. Returns `Some(exit_code)` if the flag
/// was present and handled, `None` to fall through to the normal CLI.
pub fn dispatch_dap(args: &[String]) -> Option<i32> {
    // Look for `--dap <file>` or `debug <file>` subcommand.
    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--dap" {
            // `--dap` starts the DAP server on stdio. The program path
            // comes from the DAP launch request, not the CLI.
            run();
            return Some(0);
        }
        if arg == "debug" && i + 1 < args.len() {
            // `rz debug <file>` is a user-friendly alias that starts the
            // DAP server. The file argument is printed as guidance but the
            // actual program path comes from the launch request.
            eprintln!(
                "Starting DAP server for {}. Connect a DAP client to stdin/stdout.",
                &args[i + 1]
            );
            run();
            return Some(0);
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seq_counter_increments() {
        let mut seq = SeqCounter::new();
        assert_eq!(seq.next(), 1);
        assert_eq!(seq.next(), 2);
        assert_eq!(seq.next(), 3);
    }

    #[test]
    fn dap_server_initialize_response() {
        let mut server = DapServer::new();
        let messages = server.handle_initialize(1);
        assert_eq!(messages.len(), 2);

        // First message is the response.
        assert_eq!(messages[0]["type"], "response");
        assert_eq!(messages[0]["command"], "initialize");
        assert_eq!(messages[0]["success"], true);
        assert_eq!(
            messages[0]["body"]["supportsConfigurationDoneRequest"],
            true
        );

        // Second message is the initialized event.
        assert_eq!(messages[1]["type"], "event");
        assert_eq!(messages[1]["event"], "initialized");
    }

    #[test]
    fn dap_server_launch_requires_program() {
        let mut server = DapServer::new();
        let messages = server.handle_launch(2, &json!({}));
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["success"], false);
    }

    #[test]
    fn dap_server_launch_with_program() {
        let mut server = DapServer::new();
        let messages = server.handle_launch(2, &json!({ "program": "/tmp/test.rs" }));
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["success"], true);
        assert_eq!(server.program_path, Some("/tmp/test.rs".to_string()));
    }

    #[test]
    fn dap_server_threads_response() {
        let mut server = DapServer::new();
        let messages = server.handle_threads(3);
        assert_eq!(messages.len(), 1);
        let threads = messages[0]["body"]["threads"].as_array().unwrap();
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0]["id"], 1);
        assert_eq!(threads[0]["name"], "main");
    }

    #[test]
    fn dap_server_set_breakpoints() {
        let mut server = DapServer::new();
        let args = json!({
            "source": { "path": "/tmp/test.rs" },
            "breakpoints": [
                { "line": 5 },
                { "line": 10 },
            ]
        });
        let messages = server.handle_set_breakpoints(4, &args);
        assert_eq!(messages.len(), 1);
        let bps = messages[0]["body"]["breakpoints"].as_array().unwrap();
        assert_eq!(bps.len(), 2);
        assert_eq!(bps[0]["line"], 5);
        assert_eq!(bps[1]["line"], 10);
    }

    #[test]
    fn dap_server_dispatch_unknown_command() {
        let mut server = DapServer::new();
        let msg = json!({
            "seq": 1,
            "type": "request",
            "command": "unknownCommand",
        });
        let messages = server.dispatch(&msg);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["success"], true);
        assert_eq!(messages[0]["command"], "unknownCommand");
    }

    #[test]
    fn write_and_read_message_roundtrip() {
        let msg = json!({
            "seq": 1,
            "type": "request",
            "command": "initialize",
        });
        let mut buffer = Vec::new();
        write_message(&mut buffer, &msg).unwrap();

        let mut reader = io::BufReader::new(&buffer[..]);
        let parsed = read_message(&mut reader).unwrap();
        assert_eq!(parsed["seq"], 1);
        assert_eq!(parsed["command"], "initialize");
    }

    #[test]
    fn dispatch_dap_flag_not_present() {
        let args = vec!["rz".to_string(), "test.rs".to_string()];
        assert!(dispatch_dap(&args).is_none());
    }
}
