//! RES-2794: error stack traces with source locations.
//!
//! Maintains a call stack on the interpreter and formats stack traces
//! when runtime errors occur. Each frame records the function name and
//! the source position of the call site.

use crate::span::Span;

const DEFAULT_STACKTRACE_DEPTH: usize = 64;

#[derive(Debug, Clone)]
pub struct StackFrame {
    pub fn_name: String,
    pub call_span: Span,
}

pub fn format_stack_trace(frames: &[StackFrame], source_path: &str) -> String {
    format_stack_trace_with_limit(frames, source_path, stacktrace_depth_limit())
}

pub(crate) fn format_stack_trace_with_limit(
    frames: &[StackFrame],
    source_path: &str,
    max_depth: usize,
) -> String {
    let frames = visible_frames(frames, max_depth);
    if frames.is_empty() {
        return String::new();
    }
    let mut lines = Vec::with_capacity(frames.len() + 1);
    lines.push("stack trace (most recent call last):".to_string());
    let source_path = display_source_path(source_path);
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
    builtin_stacktrace_with_limit(frames, source_path, stacktrace_depth_limit())
}

pub(crate) fn builtin_stacktrace_with_limit(
    frames: &[StackFrame],
    source_path: &str,
    max_depth: usize,
) -> Vec<String> {
    let source_path = display_source_path(source_path);
    visible_frames(frames, max_depth)
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

fn visible_frames(frames: &[StackFrame], max_depth: usize) -> &[StackFrame] {
    if frames.len() <= max_depth {
        frames
    } else {
        &frames[frames.len() - max_depth..]
    }
}

fn stacktrace_depth_limit() -> usize {
    std::env::var("RZ_STACKTRACE_DEPTH")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_STACKTRACE_DEPTH)
}

fn display_source_path(source_path: &str) -> String {
    std::path::Path::new(source_path)
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| source_path.to_string())
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
    fn builtin_stacktrace_truncates_to_limit() {
        let frames = vec![
            frame("a", 1, 1),
            frame("b", 2, 2),
            frame("c", 3, 3),
            frame("d", 4, 4),
        ];
        let result = builtin_stacktrace_with_limit(&frames, "test.rz", 2);
        assert_eq!(result, vec!["c at test.rz:3:3", "d at test.rz:4:4"]);

        let trace = format_stack_trace_with_limit(&frames, "test.rz", 2);
        let lines: Vec<&str> = trace.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[1].contains("at c (test.rz:3:3)"));
        assert!(lines[2].contains("at d (test.rz:4:4)"));
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
        assert!(
            combined.contains("at inner (<input>:") || combined.contains("at middle (<input>:"),
            "error stack trace should include source locations, got: {:?}",
            r.errors
        );
    }

    #[test]
    fn builtin_stacktrace_includes_source_locations() {
        let r = crate::run_program(
            r#"
fn inner_trace() {
    let trace = stacktrace()
    println("trace length: " + to_string(len(trace)))
    for frame in trace {
        println(frame)
    }
}

fn middle_trace() {
    inner_trace()
}

fn outer_trace() {
    middle_trace()
}

outer_trace()
"#,
        );
        assert!(r.ok, "stacktrace example should run successfully");
        assert!(r.stdout.contains("trace length: 3"), "got: {}", r.stdout);
        assert!(
            r.stdout.contains("outer_trace at <input>:")
                && r.stdout.contains("middle_trace at <input>:")
                && r.stdout.contains("inner_trace at <input>:"),
            "stacktrace() should include file:line:col in each frame, got: {}",
            r.stdout
        );
    }
}
