# Resilient Debugger — Debug Adapter Protocol (DAP)

Resilient includes a **Debug Adapter Protocol (DAP)** server for debugging Resilient programs in editors and IDEs that support DAP (VS Code, Neovim with `nvim-dap`, etc.).

The debugger allows you to:
- Set line breakpoints
- Step through code (step over, step into, step out)
- Inspect variables and scopes
- Evaluate expressions at runtime
- View stack frames and thread information

## Launching the debugger

The Resilient DAP server is invoked via the CLI:

```bash
rz --dap
```

The server listens on stdin/stdout for DAP protocol messages. A DAP client (your editor or IDE) launches this command as a child process and communicates with it over the DAP JSON wire protocol.

### Alternative: User-friendly debug alias

For interactive use, you can also use:

```bash
rz debug <file>
```

This starts the DAP server and prints guidance to stderr. The `<file>` argument is informational; the actual program path comes from the DAP `launch` request sent by your client.

## VS Code setup

### Prerequisites

Ensure you have the Resilient VS Code extension installed. The extension provides the DAP client integration.

### Launch configuration

Create a `.vscode/launch.json` file in your Resilient project with the following configuration:

```json
{
  "version": "0.2.0",
  "configurations": [
    {
      "type": "resilient",
      "request": "launch",
      "name": "Debug Resilient Program",
      "program": "${file}",
      "rz": "rz"
    }
  ]
}
```

**Configuration fields:**

| Field | Description |
|---|---|
| `type` | Must be `"resilient"` (recognized by the Resilient VS Code extension) |
| `request` | Must be `"launch"` |
| `name` | Human-readable name for this debug configuration (appears in the VS Code launch dropdown) |
| `program` | Absolute or relative path to the `.rz` file to debug. Use `"${file}"` to debug the currently open file |
| `rz` | Path to the `rz` CLI binary (default: `"rz"` if in PATH) |

### How to debug

1. Open a `.rz` file in VS Code.
2. Set breakpoints by clicking on the line number gutter (a red dot appears).
3. Open the Run and Debug panel (Ctrl+Shift+D on Linux/Windows, Cmd+Shift+D on macOS).
4. Select "Debug Resilient Program" from the launch configuration dropdown.
5. Click the green play button to start debugging.
6. The program will run and stop at breakpoints. Use the Debug toolbar to:
   - **Continue** (F5): Resume execution
   - **Step Over** (F10): Execute the current line and pause at the next one
   - **Step Into** (F11): Step into function calls
   - **Step Out** (Shift+F11): Step out of the current function

The Debug Console shows program output and allows expression evaluation.

## Supported DAP capabilities

| Capability | Supported |
|---|---|
| **Breakpoints** | Line breakpoints (`setBreakpoints` request) |
| **Execution control** | Continue, step over, step into, step out |
| **Stack frames** | Full stack trace with source location and column info |
| **Variables** | Inspect local variables and scopes |
| **Expression evaluation** | Evaluate arbitrary expressions at runtime |
| **Threads** | Single main thread (single-threaded runtime) |
| **Stop events** | Breakpoint, step, exception |
| **Output events** | Program stdout captured and displayed |
| **Termination** | Graceful shutdown and disconnect |

## Unsupported capabilities (planned future)

The following DAP capabilities are **not yet supported**:

| Capability | Notes |
|---|---|
| Conditional breakpoints | Breakpoints with expressions; blocked on expression evaluator completeness |
| Function breakpoints | Set breakpoints by function name |
| Step back / reverse debugging | Not applicable to the current execution model |
| Hover evaluation | Evaluate expressions by hovering in the editor |
| Watch expressions | Persistent expression watches across frames |
| Set variable | Modify variable values during debugging |
| Restart frame | Restart a function and re-run it |
| Goto targets | Jump to a specific line or target |
| Completions | Auto-complete in the debug console |
| Modules request | Introspect loaded modules |
| Exception options | Customize exception handling behavior |

## DAP requests implemented

The server implements the following DAP requests:

| Request | Handler | Notes |
|---|---|---|
| `initialize` | Lines 155–179 | Handshake; reports supported capabilities |
| `launch` | Lines 181–200 | Start a debug session with a program path |
| `setBreakpoints` | Lines 202–244 | Set line breakpoints in a source file |
| `configurationDone` | Lines 246–256 | Signal readiness; begins execution |
| `continue` | Lines 289–300 | Resume execution from a breakpoint |
| `next` | Lines 302–308 | Step over (next statement) |
| `stepIn` | Lines 310–316 | Step into a function call |
| `stepOut` | Lines 318–324 | Step out of the current function |
| `threads` | Lines 326–338 | List threads (always returns one "main" thread) |
| `stackTrace` | Lines 340–365 | Retrieve the call stack |
| `scopes` | Lines 367–394 | List scopes (locals) for a frame |
| `variables` | Lines 396–436 | Inspect variables in a scope |
| `evaluate` | Lines 438–479 | Evaluate an expression during a pause |
| `disconnect` | Lines 481–489 | Close the debug session |
| `terminate` | Lines 491–497 | Terminate the running program |

All other requests return a success response to avoid stalling the client.

## DAP events emitted

The server sends the following events:

| Event | Description | Source |
|---|---|---|
| `initialized` | Sent after `initialize` to signal configuration can begin | Line 177 |
| `stopped` | Program paused; includes reason (breakpoint/step/exception) and frame info | Lines 561–581 |
| `output` | Program output (stdout) | Line 583 |
| `terminated` | Program terminated; debugging session has ended | Line 592 |

## Debugging flow

1. **Client launches the server:** `rz --dap` (started as a child process by the editor)
2. **Initialize handshake:**
   - Client sends `initialize` request
   - Server responds with capabilities + `initialized` event
3. **Configure session:**
   - Client sends `launch` request with program path
   - Client sends `setBreakpoints` for each source file with breakpoints
   - Client sends `configurationDone` to signal ready
4. **Execution:**
   - Server spawns a `DebugState` thread that runs the program
   - Events (output, stopped, terminated) stream to the client
   - Client sends continue/step requests; server relays them to the debug thread
5. **Cleanup:**
   - Client sends `disconnect` when done
   - Server releases resources and exits

## Implementation reference

The DAP server is implemented in `/resilient/src/dap_server.rs` (~810 lines):

- **Message framing** (lines 21–64): Content-Length header protocol (matching LSP)
- **Server state** (lines 84–119): Tracks breakpoints, frames, scopes, and execution state
- **Request handlers** (lines 155–497): Implement each DAP request type
- **Event loop** (lines 607–666): Main message-handling loop
- **CLI dispatch** (lines 670–695): Entry point for `rz --dap` flag

The actual program execution is delegated to `crate::debugger::DebugState`, which runs on a background thread and communicates with the DAP server via MPSC channels.

## Troubleshooting

### "No program path specified"

The `launch` request did not include a `program` field, or the field was empty. Check that your launch configuration has a valid `program` path.

### Program output not appearing

Ensure your program uses `println!` or other stdout/stderr output. The debugger captures all output and displays it in the Debug Console.

### Debugger hangs during evaluation

Expression evaluation has a 5-second timeout. If the expression is very complex or causes infinite loops, it will timeout and return an error. Ensure your expressions are simple and well-formed.

### Breakpoint not hit

- Verify the breakpoint line is in actual executable code (not comments or declarations).
- Confirm the program path matches the file being debugged.
- Check that `setBreakpoints` was called before `configurationDone`.

### Multiple threads

The current implementation supports only single-threaded debugging. Multi-threaded programs will show only the main thread.
