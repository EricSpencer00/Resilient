use std::num::IntErrorKind;

/// Zero is also the lexers' recovery value. Recheck its physical spelling
/// before it can become an executable AST constant, including pre-scanned
/// source-origin tokens and the optional Logos token stream.
pub(crate) fn recovered_overflow(
    source: &[char],
    start: usize,
    end: usize,
    line: usize,
    column: usize,
) -> Option<String> {
    let literal = source.get(start..end)?;
    let (radix, digits) = match literal {
        ['0', 'x' | 'X', rest @ ..] => (16, rest),
        ['0', 'o' | 'O', rest @ ..] => (8, rest),
        ['0', 'b' | 'B', rest @ ..] => (2, rest),
        _ => (10, literal),
    };
    // Valid zero spellings are common; avoid allocating or reparsing them.
    if digits.iter().all(|c| matches!(c, '0' | '_')) {
        return None;
    }
    let cleaned: String = digits.iter().filter(|&&c| c != '_').collect();
    let error = i64::from_str_radix(&cleaned, radix).err()?;
    if error.kind() != &IntErrorKind::PosOverflow {
        return None;
    }
    Some(crate::format_integer_literal_overflow_error(
        line,
        column,
        &literal.iter().collect::<String>(),
    ))
}
