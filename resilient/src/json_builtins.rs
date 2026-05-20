//! RES-2657: JSON serialization and deserialization builtins.
//!
//! * `to_json(value) -> string` — serialize any Value to JSON.
//! * `from_json(s) -> value` — parse a JSON string into a Value.
//!
//! Mapping:
//! - `int`    ↔ JSON number (integer)
//! - `float`  ↔ JSON number (float)
//! - `string` ↔ JSON string
//! - `bool`   ↔ JSON boolean
//! - `array`  ↔ JSON array
//! - `map`    ↔ JSON object (keys must be strings)
//! - `void`   ↔ JSON null
//! - `Option(None)` ↔ JSON null
//! - `Option(Some(x))` ↔ serialized x
//! - `Result { ok: true, payload }` ↔ `{"ok": true, "value": ...}`
//! - `Result { ok: false, payload }` ↔ `{"ok": false, "error": ...}`

use crate::{MapKey, Value};
use std::collections::HashMap;

type RResult<T> = Result<T, String>;

// ── Serialization ─────────────────────────────────────────────────────────────

/// `to_json(value) -> string`
///
/// Serializes `value` to a JSON string. Maps must have string keys.
/// Structs, tuples, and other complex types are not directly serializable.
///
/// ```text
/// to_json(42)            // == "42"
/// to_json([1, 2, 3])     // == "[1, 2, 3]"
/// to_json({"a" -> 1})    // == "{\"a\": 1}"
/// to_json(true)          // == "true"
/// to_json(void)          // error — use None for null
/// ```
pub(crate) fn builtin_to_json(args: &[Value]) -> RResult<Value> {
    match args {
        [v] => serialize_value(v).map(Value::String),
        _ => Err(format!("to_json: expected 1 argument, got {}", args.len())),
    }
}

/// RES-2328: public entry point. The previous `serialize_value`
/// recursively built fresh `String`s at every level — Array/Tuple
/// arms collected `Vec<String>` then `.join(", ")`'d, Map arms paid
/// the same Vec+join plus a per-pair `format!` for `"{}: {}"`, and
/// Result arms paid a `format!` to wrap the payload. For an N-element
/// container with K children, that's roughly O(N+K) wasted
/// `String`s + the outer Vec + the join buffer.
///
/// Route everything through `serialize_into(v, &mut String)` so the
/// entire JSON document is built into a single shared buffer.
/// Same byte output; one `String` allocation per top-level call
/// instead of one per nested value. Mirrors the direct-write pattern
/// applied to `recovers_to_bmc::node_to_smtlib2` (RES-2268),
/// `behavioral_fingerprint::node_text` (RES-2270), and
/// `lint::clause_text` (RES-2272).
fn serialize_value(v: &Value) -> RResult<String> {
    let mut out = String::new();
    serialize_into(v, &mut out)?;
    Ok(out)
}

fn serialize_into(v: &Value, out: &mut String) -> RResult<()> {
    use std::fmt::Write as _;
    match v {
        Value::Int(n) => {
            let _ = write!(out, "{}", n);
            Ok(())
        }
        Value::Float(f) => {
            if f.is_nan() {
                Err("to_json: NaN is not a valid JSON value".to_string())
            } else if f.is_infinite() {
                Err("to_json: Infinity is not a valid JSON value".to_string())
            } else if f.fract() == 0.0 && f.abs() < 1e15 {
                let _ = write!(out, "{:.1}", f);
                Ok(())
            } else {
                let _ = write!(out, "{}", f);
                Ok(())
            }
        }
        Value::Bool(b) => {
            out.push_str(if *b { "true" } else { "false" });
            Ok(())
        }
        Value::String(s) => {
            json_escape_into(s, out);
            Ok(())
        }
        Value::Array(arr) => {
            out.push('[');
            for (i, item) in arr.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                serialize_into(item, out)?;
            }
            out.push(']');
            Ok(())
        }
        Value::Map(m) => {
            // Sort keys for deterministic output
            let mut sorted: Vec<(&MapKey, &Value)> = m.iter().collect();
            sorted.sort_by_key(|(k, _)| match k {
                MapKey::Str(s) => format!("s:{s}"),
                MapKey::Int(n) => format!("i:{n}"),
                MapKey::Bool(b) => format!("b:{b}"),
            });
            out.push('{');
            for (i, (k, val)) in sorted.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                match k {
                    MapKey::Str(s) => json_escape_into(s, out),
                    MapKey::Int(n) => {
                        let _ = write!(out, "\"{}\"", n);
                    }
                    MapKey::Bool(b) => {
                        let _ = write!(out, "\"{}\"", b);
                    }
                }
                out.push_str(": ");
                serialize_into(val, out)?;
            }
            out.push('}');
            Ok(())
        }
        Value::Void => {
            out.push_str("null");
            Ok(())
        }
        Value::Option(None) => {
            out.push_str("null");
            Ok(())
        }
        Value::Option(Some(inner)) => serialize_into(inner, out),
        Value::Result { ok, payload } => {
            if *ok {
                out.push_str("{\"ok\": true, \"value\": ");
            } else {
                out.push_str("{\"ok\": false, \"error\": ");
            }
            serialize_into(payload, out)?;
            out.push('}');
            Ok(())
        }
        Value::Tuple(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                serialize_into(item, out)?;
            }
            out.push(']');
            Ok(())
        }
        other => Err(format!(
            "to_json: cannot serialize value of type {}",
            type_name(other)
        )),
    }
}

/// RES-2260: write the control-char escape directly via `std::fmt::
/// Write` instead of `push_str(&format!(...))`. Each control
/// character previously allocated a 6-char `String` only to be
/// immediately `push_str`'d.
///
/// RES-2328: appends directly into the serializer's shared output
/// buffer instead of returning an owned `String` per nested value.
fn json_escape_into(s: &str, out: &mut String) {
    use std::fmt::Write;
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Int(_) => "int",
        Value::Float(_) => "float",
        Value::String(_) => "string",
        Value::Bool(_) => "bool",
        Value::Array(_) => "array",
        Value::Map(_) => "map",
        Value::Set(_) => "set",
        Value::Void => "void",
        Value::Function(_) | Value::Closure { .. } | Value::Builtin { .. } => "function",
        Value::Bytes(_) => "bytes",
        Value::Struct { .. } => "struct",
        Value::Tuple(_) => "tuple",
        Value::EnumVariant { .. } => "enum",
        Value::Result { .. } => "result",
        Value::Option(_) => "option",
        Value::ActorPid(_) => "actor_pid",
        _ => "other",
    }
}

// ── Deserialization ───────────────────────────────────────────────────────────

/// `from_json(s) -> value`
///
/// Parses a JSON string into a Resilient value:
/// - JSON null → `void`
/// - JSON bool → `bool`
/// - JSON integer number → `int`
/// - JSON float number → `float`
/// - JSON string → `string`
/// - JSON array → `array`
/// - JSON object → `map` (string keys)
///
/// Returns an error for malformed JSON.
pub(crate) fn builtin_from_json(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(s)] => {
            let mut parser = JsonParser::new(s);
            let v = parser.parse_value()?;
            parser.skip_ws();
            if parser.pos < parser.src.len() {
                return Err(format!(
                    "from_json: trailing characters after JSON value at position {}",
                    parser.pos
                ));
            }
            Ok(v)
        }
        [other] => Err(format!("from_json: expected string, got {other}")),
        _ => Err(format!(
            "from_json: expected 1 argument, got {}",
            args.len()
        )),
    }
}

struct JsonParser<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn new(s: &'a str) -> Self {
        Self {
            src: s.as_bytes(),
            pos: 0,
        }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.src.len() && self.src[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn consume(&mut self) -> RResult<u8> {
        if self.pos < self.src.len() {
            let b = self.src[self.pos];
            self.pos += 1;
            Ok(b)
        } else {
            Err("from_json: unexpected end of input".to_string())
        }
    }

    fn expect_byte(&mut self, expected: u8) -> RResult<()> {
        let b = self.consume()?;
        if b != expected {
            Err(format!(
                "from_json: expected '{}' at position {}, got '{}'",
                expected as char,
                self.pos - 1,
                b as char
            ))
        } else {
            Ok(())
        }
    }

    fn expect_str(&mut self, s: &[u8]) -> RResult<()> {
        for &b in s {
            self.expect_byte(b)?;
        }
        Ok(())
    }

    fn parse_value(&mut self) -> RResult<Value> {
        self.skip_ws();
        match self.peek() {
            Some(b'n') => {
                self.expect_str(b"null")?;
                Ok(Value::Void)
            }
            Some(b't') => {
                self.expect_str(b"true")?;
                Ok(Value::Bool(true))
            }
            Some(b'f') => {
                self.expect_str(b"false")?;
                Ok(Value::Bool(false))
            }
            Some(b'"') => self.parse_string().map(Value::String),
            Some(b'[') => self.parse_array(),
            Some(b'{') => self.parse_object(),
            Some(b'-') | Some(b'0'..=b'9') => self.parse_number(),
            Some(b) => Err(format!(
                "from_json: unexpected character '{}' at position {}",
                b as char, self.pos
            )),
            None => Err("from_json: unexpected end of input".to_string()),
        }
    }

    fn parse_string(&mut self) -> RResult<String> {
        self.expect_byte(b'"')?;
        // RES-1946: scan ahead for the closing unescaped `"` to get an
        // upper bound on the decoded string length. Escape sequences
        // can only shorten the output (`\n` → 1 char, `\uXXXX` → ≤ 4
        // bytes UTF-8 = up to 6 input bytes → 4 output bytes), so the
        // raw byte delta is a safe pre-size hint. Falls back to 16 if
        // the input is malformed (no closing quote on this line).
        let cap = {
            let mut i = self.pos;
            let mut bound: Option<usize> = None;
            while i < self.src.len() {
                match self.src[i] {
                    b'"' => {
                        bound = Some(i - self.pos);
                        break;
                    }
                    b'\\' => {
                        // Skip the escape char AND the next byte;
                        // `\uXXXX` is over-counted by 4 but that's
                        // a tighter upper bound than no scan at all.
                        i += 2;
                    }
                    _ => i += 1,
                }
            }
            bound.unwrap_or(16)
        };
        let mut out = String::with_capacity(cap);
        loop {
            let b = self.consume()?;
            match b {
                b'"' => return Ok(out),
                b'\\' => {
                    let esc = self.consume()?;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'b' => out.push('\x08'),
                        b'f' => out.push('\x0C'),
                        b'u' => {
                            let hex = self.take_n(4)?;
                            let n = u32::from_str_radix(&hex, 16)
                                .map_err(|_| format!("from_json: invalid \\u escape: {hex}"))?;
                            let c = char::from_u32(n).ok_or_else(|| {
                                format!("from_json: invalid unicode codepoint U+{n:04X}")
                            })?;
                            out.push(c);
                        }
                        other => {
                            return Err(format!(
                                "from_json: unknown escape \\{} at position {}",
                                other as char,
                                self.pos - 1
                            ));
                        }
                    }
                }
                b if b < 0x20 => {
                    return Err(format!(
                        "from_json: unescaped control character 0x{:02x} in string",
                        b
                    ));
                }
                b => out.push(b as char),
            }
        }
    }

    fn take_n(&mut self, n: usize) -> RResult<String> {
        if self.pos + n > self.src.len() {
            return Err("from_json: unexpected end of input in escape".to_string());
        }
        let slice = &self.src[self.pos..self.pos + n];
        self.pos += n;
        Ok(String::from_utf8_lossy(slice).into_owned())
    }

    fn parse_number(&mut self) -> RResult<Value> {
        let start = self.pos;
        let mut is_float = false;

        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        // Integer part
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
        // Fractional part
        if self.peek() == Some(b'.') {
            is_float = true;
            self.pos += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(format!(
                    "from_json: expected digits after '.' at position {}",
                    self.pos
                ));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        // Exponent
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            is_float = true;
            self.pos += 1;
            if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                self.pos += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(format!(
                    "from_json: expected digits in exponent at position {}",
                    self.pos
                ));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }

        let s = std::str::from_utf8(&self.src[start..self.pos])
            .map_err(|_| "from_json: invalid UTF-8 in number".to_string())?;

        if is_float {
            let f: f64 = s
                .parse()
                .map_err(|_| format!("from_json: invalid float: {s}"))?;
            Ok(Value::Float(f))
        } else {
            match s.parse::<i64>() {
                Ok(n) => Ok(Value::Int(n)),
                Err(_) => {
                    // Oversized integer → fall back to float
                    let f: f64 = s
                        .parse()
                        .map_err(|_| format!("from_json: integer too large: {s}"))?;
                    Ok(Value::Float(f))
                }
            }
        }
    }

    fn parse_array(&mut self) -> RResult<Value> {
        self.expect_byte(b'[')?;
        // RES-1946: typical JSON arrays hold 1-10 items; pre-size to
        // 4 to skip the default 0→4 first grow. Empty arrays
        // (immediately followed by `]`) waste 4 slots — negligible.
        let mut items = Vec::with_capacity(4);
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Value::Array(items));
        }
        loop {
            items.push(self.parse_value()?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Value::Array(items));
                }
                Some(b) => {
                    return Err(format!(
                        "from_json: expected ',' or ']' in array at position {}, got '{}'",
                        self.pos, b as char
                    ));
                }
                None => return Err("from_json: unterminated array".to_string()),
            }
        }
    }

    fn parse_object(&mut self) -> RResult<Value> {
        self.expect_byte(b'{')?;
        // RES-1946: typical JSON objects hold 2-10 entries; pre-size
        // to 4 to skip the default 0-bucket → 4-bucket rehash. Empty
        // objects waste a small bucket array — negligible.
        let mut map: HashMap<MapKey, Value> = HashMap::with_capacity(4);
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Value::Map(map));
        }
        loop {
            self.skip_ws();
            let key = self.parse_string()?;
            self.skip_ws();
            self.expect_byte(b':')?;
            let val = self.parse_value()?;
            map.insert(MapKey::Str(key), val);
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Value::Map(map));
                }
                Some(b) => {
                    return Err(format!(
                        "from_json: expected ',' or '}}' in object at position {}, got '{}'",
                        self.pos, b as char
                    ));
                }
                None => return Err("from_json: unterminated object".to_string()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::run_program;

    fn run(src: &str) -> crate::RunResult {
        run_program(src)
    }

    // ── to_json ───────────────────────────────────────────────────────────────

    #[test]
    fn to_json_primitives() {
        let r = run(r#"println(to_json(42));
println(to_json(3.14));
println(to_json(true));
println(to_json(false));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "42");
        assert_eq!(lines[1], "3.14");
        assert_eq!(lines[2], "true");
        assert_eq!(lines[3], "false");
    }

    #[test]
    fn to_json_string() {
        let r = run(r#"println(to_json("hello world"));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("\"hello world\""), "stdout: {}", r.stdout);
    }

    #[test]
    fn to_json_string_escaping() {
        let r = run(r#"println(to_json("say \"hi\""));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("\\\"hi\\\""), "stdout: {}", r.stdout);
    }

    #[test]
    fn to_json_array() {
        let r = run(r#"println(to_json([1, 2, 3]));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("[1, 2, 3]"), "stdout: {}", r.stdout);
    }

    #[test]
    fn to_json_nested_array() {
        let r = run(r#"println(to_json([[1, 2], [3, 4]]));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(
            r.stdout.contains("[[1, 2], [3, 4]]"),
            "stdout: {}",
            r.stdout
        );
    }

    #[test]
    fn to_json_map() {
        let r = run(r#"let m = {"name" -> "Alice", "age" -> 30};
let j = to_json(m);
println(contains(j, "\"name\""));
println(contains(j, "\"Alice\""));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "true");
        assert_eq!(lines[1], "true");
    }

    #[test]
    fn to_json_void_is_null() {
        // void serializes as null
        // We can't easily pass void as an argument, so test via None
        let r = run("println(to_json(None));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("null"), "stdout: {}", r.stdout);
    }

    #[test]
    fn to_json_option_some() {
        let r = run("println(to_json(Some(42)));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("42"), "stdout: {}", r.stdout);
    }

    #[test]
    fn to_json_result_ok() {
        let r = run(r#"println(to_json(Ok(42)));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("\"ok\": true"), "stdout: {}", r.stdout);
        assert!(r.stdout.contains("42"), "stdout: {}", r.stdout);
    }

    #[test]
    fn to_json_result_err() {
        let r = run(r#"println(to_json(Err("oops")));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("\"ok\": false"), "stdout: {}", r.stdout);
        assert!(r.stdout.contains("\"error\""), "stdout: {}", r.stdout);
    }

    // ── from_json ─────────────────────────────────────────────────────────────

    #[test]
    fn from_json_int() {
        let r = run(r#"let v = from_json("42");
println(v);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("42"), "stdout: {}", r.stdout);
    }

    #[test]
    fn from_json_float() {
        let r = run(r#"let v = from_json("3.14");
println(v);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("3.14"), "stdout: {}", r.stdout);
    }

    #[test]
    fn from_json_bool() {
        let r = run(r#"println(from_json("true"));
println(from_json("false"));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "true");
        assert_eq!(lines[1], "false");
    }

    #[test]
    fn from_json_string() {
        let r = run(r#"let v = from_json("\"hello\"");
println(v);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("hello"), "stdout: {}", r.stdout);
    }

    #[test]
    fn from_json_null_is_void() {
        let r = run(r#"let v = from_json("null");
println(type_of(v));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("void"), "stdout: {}", r.stdout);
    }

    #[test]
    fn from_json_array() {
        let r = run(r#"let v = from_json("[1, 2, 3]");
println(type_of(v));
println(len(v));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "array");
        assert_eq!(lines[1], "3");
    }

    #[test]
    fn from_json_object() {
        // Use \{ to get a literal { in a Resilient string (not interpolation).
        let r = run(r#"let json_str = "\{\"name\": \"Alice\", \"age\": 30}";
let v = from_json(json_str);
println(type_of(v));
println(v["name"]);
println(v["age"]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "map");
        assert_eq!(lines[1], "Alice");
        assert_eq!(lines[2], "30");
    }

    #[test]
    fn from_json_nested() {
        let r = run(r#"let json_str = "\{\"items\": [1, 2, 3]}";
let v = from_json(json_str);
let items = v["items"];
println(len(items));
println(items[0]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "3");
        assert_eq!(lines[1], "1");
    }

    #[test]
    fn from_json_error_malformed() {
        let r = run(r#"let v = from_json("\{bad json}");"#);
        assert!(!r.ok, "expected parse error for malformed JSON");
    }

    #[test]
    fn from_json_error_trailing() {
        let r = run(r#"let v = from_json("42 extra");"#);
        assert!(!r.ok, "expected error for trailing content");
    }

    // ── roundtrip ─────────────────────────────────────────────────────────────

    #[test]
    fn roundtrip_array_of_ints() {
        let r = run(r#"let arr = [10, 20, 30];
let j = to_json(arr);
let arr2 = from_json(j);
println(arr2[0]);
println(arr2[2]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "10");
        assert_eq!(lines[1], "30");
    }

    #[test]
    fn roundtrip_map() {
        let r = run(r#"let m = {"x" -> 1, "y" -> 2};
let j = to_json(m);
let m2 = from_json(j);
println(m2["x"]);
println(m2["y"]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "1");
        assert_eq!(lines[1], "2");
    }

    #[test]
    fn roundtrip_nested_structure() {
        let r = run(
            r#"let data = {"users" -> [{"name" -> "Bob", "score" -> 95}, {"name" -> "Alice", "score" -> 87}]};
let j = to_json(data);
let data2 = from_json(j);
let users = data2["users"];
println(len(users));"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('2'), "stdout: {}", r.stdout);
    }

    #[test]
    fn from_json_whitespace_tolerance() {
        let r = run(r#"let json_str = "  \{  \"a\" :  1  }  ";
let v = from_json(json_str);
println(v["a"]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('1'), "stdout: {}", r.stdout);
    }

    #[test]
    fn from_json_empty_array() {
        let r = run(r#"let v = from_json("[]");
println(len(v));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('0'), "stdout: {}", r.stdout);
    }

    #[test]
    fn from_json_empty_object() {
        let r = run(r#"let v = from_json("\{}");
println(type_of(v));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("map"), "stdout: {}", r.stdout);
    }
}
