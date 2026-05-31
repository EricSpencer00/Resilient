//! RES-2794: error stack traces with source locations.
//!
//! Maintains a call stack on the interpreter and formats stack traces
//! when runtime errors occur. Each frame records the function name and
//! the source position of the call site.

use crate::span::Span;

#[derive(Debug, Clone)]
pub struct StackFrame {
    pub fn_name: String,
    pub call_span: Span,
}

pub fn format_stack_trace(frames: &[StackFrame], source_path: &str) -> String {
    if frames.is_empty() {
        return String::new();
    }
    let mut lines = Vec::with_capacity(frames.len() + 1);
    lines.push("stack trace (most recent call last):".to_string());
    for frame in frames {
        let loc = if frame.call_span.start.line > 0 {
            format!(
                "{}:{}:{}",
                source_path, frame.call_span.start.line, frame.call_span.start.column
            )
        } else {
            source_path.to_string()
        };
        lines.push(format!("  at {} ({})", frame.fn_name, loc));
    }
    lines.join("\n")
}

pub fn builtin_stacktrace(frames: &[StackFrame], source_path: &str) -> Vec<String> {
    frames
        .iter()
        .map(|f| {
            if f.call_span.start.line > 0 {
                format!(
                    "{} at {}:{}:{}",
                    f.fn_name, source_path, f.call_span.start.line, f.call_span.start.column
                )
            } else {
                f.fn_name.clone()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::Pos;

    fn frame(name: &str, line: usize, col: usize) -> StackFrame {
        StackFrame {
            fn_name: name.to_string(),
            call_span: Span {
                start: Pos {
                    line,
                    column: col,
                    offset: 0,
                },
                end: Pos {
                    line,
                    column: col,
                    offset: 0,
                },
            },
        }
    }

    #[test]
    fn format_empty_stack() {
        assert_eq!(format_stack_trace(&[], "test.rz"), "");
    }

    #[test]
    fn format_single_frame() {
        let frames = vec![frame("main", 5, 1)];
        let trace = format_stack_trace(&frames, "test.rz");
        assert!(trace.contains("most recent call last"));
        assert!(trace.contains("at main (test.rz:5:1)"));
    }

    #[test]
    fn format_multi_frame() {
        let frames = vec![
            frame("main", 10, 5),
            frame("process", 20, 3),
            frame("validate", 30, 7),
        ];
        let trace = format_stack_trace(&frames, "app.rz");
        let lines: Vec<&str> = trace.lines().collect();
        assert_eq!(lines.len(), 4);
        assert!(lines[1].contains("main"));
        assert!(lines[2].contains("process"));
        assert!(lines[3].contains("validate"));
    }

    #[test]
    fn builtin_stacktrace_returns_strings() {
        let frames = vec![frame("foo", 1, 1), frame("bar", 2, 3)];
        let result = builtin_stacktrace(&frames, "test.rz");
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "foo at test.rz:1:1");
        assert_eq!(result[1], "bar at test.rz:2:3");
    }

    #[test]
    fn end_to_end_stack_trace_in_error() {
        let r = crate::run_program(
            r#"
fn inner() -> int {
    return 1 / 0
}

fn middle() -> int {
    return inner()
}

fn outer() -> int {
    return middle()
}

outer()
"#,
        );
        assert!(!r.ok, "should error on division by zero");
        let combined = r.errors.join(" ");
        assert!(
            combined.contains("stack trace") || combined.contains("inner"),
            "error should include stack trace, got: {:?}",
            r.errors
        );
    }
}
