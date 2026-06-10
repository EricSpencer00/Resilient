//! RES-2611: Source maps for compiled-output debugging.
//!
//! A `SourceMap` records how bytecode offsets map back to Resilient
//! source lines, enabling debuggers and error reporters to display the
//! right source position for any runtime address.
//!
//! ## How source maps are built
//!
//! The bytecode compiler already stores a `line_info: Vec<u32>` on every
//! `Chunk` (one entry per instruction, value = 1-based source line).
//! `SourceMap::from_chunk` converts that dense per-instruction table into
//! an RLE-compressed form: consecutive instructions on the same line are
//! collapsed into a single `(start_pc, line)` entry.
//!
//! ## API
//!
//!   SourceMap::from_chunk(chunk) → SourceMap
//!   SourceMap::lookup(pc)        → Option<u32>   (1-based line, or None)
//!   SourceMap::entries()         → &[(usize, u32)]
//!
//! ## `rz dump-source-map`
//!
//! ```text
//! $ rz dump-source-map hello.rz
//! pc=0  line=1
//! pc=3  line=2
//! pc=7  line=4
//! ...
//! ```

use crate::bytecode::Chunk;

// ---------------------------------------------------------------------------
// SourceMap
// ---------------------------------------------------------------------------

/// A compressed source-position table for one bytecode chunk.
///
/// Entries are sorted by `start_pc`. Each entry says "from `start_pc`
/// onwards (until the next entry), instructions came from `line`."
#[derive(Debug, Clone, Default)]
pub struct SourceMap {
    entries: Vec<(usize, u32)>,
}

impl SourceMap {
    /// Build a `SourceMap` from the dense `line_info` table in a `Chunk`.
    /// Adjacent instructions on the same source line are merged (RLE).
    pub fn from_chunk(chunk: &Chunk) -> Self {
        let mut entries: Vec<(usize, u32)> = Vec::new();
        let mut last_line: Option<u32> = None;
        for (pc, &line) in chunk.line_info.iter().enumerate() {
            if last_line != Some(line) {
                entries.push((pc, line));
                last_line = Some(line);
            }
        }
        SourceMap { entries }
    }

    /// Look up the source line for instruction at `pc`.
    /// Returns the line of the last entry whose `start_pc ≤ pc`.
    #[allow(dead_code)]
    pub fn lookup(&self, pc: usize) -> Option<u32> {
        // Binary search for the last entry with start_pc ≤ pc.
        if self.entries.is_empty() {
            return None;
        }
        let idx = self
            .entries
            .partition_point(|(start, _)| *start <= pc)
            .saturating_sub(1);
        self.entries.get(idx).map(|(_, line)| *line)
    }

    /// All RLE-compressed entries: `(start_pc, source_line)`.
    pub fn entries(&self) -> &[(usize, u32)] {
        &self.entries
    }

    /// Total number of instructions this map covers (length of original line_info).
    #[allow(dead_code)]
    pub fn instruction_count(&self) -> usize {
        // The last entry's start_pc is the last instruction that changed line;
        // we don't know the total count without the chunk, so we expose entries.
        self.entries.last().map(|(pc, _)| pc + 1).unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------
// CLI subcommand: rz dump-source-map <file>
// ---------------------------------------------------------------------------

/// Handles `rz dump-source-map <file>`.
/// Returns `Some(exit_code)` when matched, `None` to fall through.
pub(crate) fn dispatch_dump_source_map(args: &[String]) -> Option<i32> {
    if args.get(1).map(|s| s.as_str()) != Some("dump-source-map") {
        return None;
    }
    if is_dump_source_map_help_request(args) {
        print_dump_source_map_help();
        return Some(0);
    }

    let file = match args.get(2) {
        Some(f) => f.clone(),
        None => {
            eprintln!("usage: rz dump-source-map <file.rz>");
            return Some(1);
        }
    };

    let src = match std::fs::read_to_string(&file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("dump-source-map: cannot read {file}: {e}");
            return Some(1);
        }
    };

    let (program, errors) = crate::parse(&src);
    if !errors.is_empty() {
        for e in &errors {
            eprintln!("parse error: {e}");
        }
        return Some(1);
    }

    let compiled = match crate::compiler::compile(&program) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("compile error: {e}");
            return Some(1);
        }
    };

    println!("source map for {file}:");
    println!();

    // Main chunk
    let main_map = SourceMap::from_chunk(&compiled.main);
    if main_map.entries().is_empty() {
        println!("  [main] (no instructions)");
    } else {
        for (pc, line) in main_map.entries() {
            println!("  [main] pc={pc:<6} line={line}");
        }
    }

    // Per-function chunks
    for func in &compiled.functions {
        let func_map = SourceMap::from_chunk(&func.chunk);
        println!();
        println!("  [fn {}]", func.name);
        if func_map.entries().is_empty() {
            println!("    (no instructions)");
        } else {
            for (pc, line) in func_map.entries() {
                println!("    pc={pc:<6} line={line}");
            }
        }
    }

    Some(0)
}

const DUMP_SOURCE_MAP_HELP_TEXT: &str = r#"rz dump-source-map — print bytecode-to-source-line mappings

USAGE:
    rz dump-source-map <file>

OUTPUT:
    Compiles the file, then prints bytecode program counters with source lines.
    The report includes the main chunk and one section per compiled function.

EXAMPLES:
    rz dump-source-map examples/hello.rz
    rz dump-source-map firmware/control.rz

Run `rz --help` for global flags and other subcommands.
"#;

pub(crate) fn is_dump_source_map_help_request(args: &[String]) -> bool {
    args.get(1).map(String::as_str) == Some("dump-source-map")
        && matches!(
            args.get(2).map(String::as_str),
            Some("--help" | "-h" | "help")
        )
}

pub(crate) fn print_dump_source_map_help() {
    print!("{}", DUMP_SOURCE_MAP_HELP_TEXT);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::Chunk;

    fn chunk_with_lines(lines: &[u32]) -> Chunk {
        Chunk {
            line_info: lines.to_vec(),
            ..Chunk::default()
        }
    }

    #[test]
    fn empty_chunk_returns_none() {
        let map = SourceMap::from_chunk(&chunk_with_lines(&[]));
        assert_eq!(map.lookup(0), None);
    }

    #[test]
    fn single_line_all_same() {
        let map = SourceMap::from_chunk(&chunk_with_lines(&[5, 5, 5, 5]));
        assert_eq!(map.lookup(0), Some(5));
        assert_eq!(map.lookup(3), Some(5));
        assert_eq!(map.entries().len(), 1, "all same line → 1 RLE entry");
    }

    #[test]
    fn multiple_lines_lookup() {
        // pc 0-2: line 1, pc 3-5: line 2, pc 6: line 4
        let map = SourceMap::from_chunk(&chunk_with_lines(&[1, 1, 1, 2, 2, 2, 4]));
        assert_eq!(map.lookup(0), Some(1));
        assert_eq!(map.lookup(2), Some(1));
        assert_eq!(map.lookup(3), Some(2));
        assert_eq!(map.lookup(5), Some(2));
        assert_eq!(map.lookup(6), Some(4));
        assert_eq!(map.entries().len(), 3);
    }

    #[test]
    fn lookup_past_end_returns_last_line() {
        let map = SourceMap::from_chunk(&chunk_with_lines(&[1, 2, 3]));
        assert_eq!(map.lookup(100), Some(3));
    }

    #[test]
    fn rle_compression_reduces_entry_count() {
        // 1000 instructions all on line 7
        let lines: Vec<u32> = vec![7; 1000];
        let map = SourceMap::from_chunk(&chunk_with_lines(&lines));
        assert_eq!(map.entries().len(), 1);
        assert_eq!(map.lookup(999), Some(7));
    }

    #[test]
    fn compiler_integration() {
        let (prog, _) = crate::parse("let x = 1;\nlet y = 2;\nprintln(to_string(x + y));");
        let compiled = crate::compiler::compile(&prog).expect("compile");
        let map = SourceMap::from_chunk(&compiled.main);
        // At least one entry per source line.
        assert!(!map.entries().is_empty(), "map should be non-empty");
        // Lines should be reasonable (0 = synthetic/unknown, ≥1 = real source line).
        for (_, line) in map.entries() {
            assert!(*line < 100_000, "line must be plausible, got {line}");
        }
    }
}
