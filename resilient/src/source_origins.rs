//! Opt-in physical parser metadata for one immutable, caller-labelled buffer.
//! This module never resolves imports, checks types, or executes a program.
//!
//! Supplemental resource/lexer policy v1 (separate from the frozen CLI vectors):
//! use the real legacy scanner plus the existing Parser, regardless of the
//! normal build's lexer selection. This is not normal Logos parity evidence.
//! After the original global preflight, require at most 128 tokens in each
//! Function-keyword region; only a Function at the same/lower delimiter depth
//! starts a new region. The prefix before the first Function is also bounded.
//! Long otherwise-valid syntax is conservatively withheld with
//! `resource-limit` / `expression-segment-tokens`; normal parsing is unchanged.
//! Output collection stops once its serialized lower bound exceeds the cap,
//! while parser/profile validation continues so no partial origins escape.
use super::{
    Lexer, Node, Parser, Token,
    span::{Pos, Span},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::Read,
    path::Path,
};

const PROFILE: &str = "physical-origins-v1";
const SOURCE_LIMIT: usize = 1024 * 1024;
const OUTPUT_LIMIT: usize = 524_288;

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Default)]
pub(super) struct LexicalStatus {
    pub(super) reason: Option<&'static str>,
}
impl LexicalStatus {
    pub(super) fn fail(&mut self, reason: &'static str) {
        self.reason.get_or_insert(reason);
    }
}

struct SourceUnit {
    text: String,
    key: String,
    digest: String,
    /// Includes the EOF boundary; every physical range uses this exact buffer.
    boundaries: Vec<usize>,
}
impl SourceUnit {
    fn new(text: String, key: String) -> Self {
        let digest = hash(text.as_bytes());
        let mut boundaries: Vec<_> = text.char_indices().map(|(i, _)| i).collect();
        boundaries.push(text.len());
        Self {
            text,
            key,
            digest,
            boundaries,
        }
    }
    fn byte(&self, offset: usize) -> Option<usize> {
        self.boundaries.get(offset).copied()
    }
    fn range(&self, start: usize, end: usize) -> Option<[usize; 2]> {
        let range = [self.byte(start)?, self.byte(end)?];
        (range[0] <= range[1]).then_some(range)
    }
    fn slice_hash(&self, range: [usize; 2]) -> String {
        hash(&self.text.as_bytes()[range[0]..range[1]])
    }
    fn envelope(&self) -> Value {
        envelope(
            Some(&self.key),
            Some(PROFILE),
            Some(&self.digest),
            Some(self.text.len()),
        )
    }
}

struct Clause {
    kind: &'static str,
    keyword: Span,
    first: Span,
    last: Span,
}
struct Declaration {
    start: Span,
    kind: &'static str,
    name: String,
    scope: Vec<Value>,
    body: Option<Span>,
    clauses: Vec<Clause>,
}

pub(super) struct Observer {
    source: SourceUnit,
    scope: Vec<Value>,
    active: Option<Declaration>,
    suppressed_declarations: usize,
    declarations: Vec<Value>,
    sites: Vec<Value>,
    rejected: bool,
    malformed: bool,
    output_overflow: bool,
    collected_bytes: usize,
}
impl Observer {
    fn new(source: SourceUnit) -> Self {
        Self {
            source,
            scope: Vec::new(),
            active: None,
            suppressed_declarations: 0,
            declarations: Vec::new(),
            sites: Vec::new(),
            rejected: false,
            malformed: false,
            output_overflow: false,
            collected_bytes: 0,
        }
    }
    pub(super) fn reject_profile(&mut self) {
        self.rejected = true;
    }
    pub(super) fn begin_declaration(&mut self, span: Span, kind: &'static str) {
        if self.active.is_some() {
            self.reject_profile();
            self.suppressed_declarations += 1;
            return;
        }
        if !self.output_overflow
            && serde_json::to_vec(&self.scope).expect("scope JSON").len() > OUTPUT_LIMIT
        {
            self.output_overflow = true;
        }
        self.active = Some(Declaration {
            start: span,
            kind,
            name: String::new(),
            scope: if self.output_overflow {
                Vec::new()
            } else {
                self.scope.clone()
            },
            body: None,
            clauses: Vec::new(),
        });
    }
    pub(super) fn set_declaration_name(&mut self, name: &str) {
        if self.suppressed_declarations > 0 {
            return;
        }
        if !identifier(name) {
            self.reject_profile();
        }
        if let Some(active) = &mut self.active {
            active.name = name.to_owned();
        } else {
            self.malformed = true;
        }
    }
    pub(super) fn start_body(&mut self, open: Span) {
        if self.suppressed_declarations > 0 {
            return;
        }
        if let Some(active) = &mut self.active {
            active.body = Some(open);
        } else {
            self.malformed = true;
        }
    }
    pub(super) fn abort_declaration(&mut self) {
        if self.suppressed_declarations > 0 {
            self.suppressed_declarations -= 1;
            self.malformed = true;
            return;
        }
        self.active = None;
        self.malformed = true;
    }
    pub(super) fn push_scope(&mut self, kind: &'static str, name: &str) {
        if self.active.is_some() || !identifier(name) || !matches!(kind, "module" | "impl") {
            self.reject_profile();
        }
        self.scope.push(json!({"kind":kind,"name":name}));
    }
    pub(super) fn pop_scope(&mut self, closed: bool) {
        if !closed || self.scope.pop().is_none() {
            self.malformed = true;
        }
    }
    pub(super) fn record_clause(
        &mut self,
        kind: &'static str,
        keyword: Span,
        first: Span,
        last: Span,
        parsed: bool,
    ) {
        if !parsed {
            self.malformed = true;
        }
        if self.suppressed_declarations > 0 {
            return;
        }
        if let Some(active) = &mut self.active {
            active.clauses.push(Clause {
                kind,
                keyword,
                first,
                last,
            });
        } else {
            self.malformed = true;
        }
    }
    pub(super) fn on_current_token(&mut self, token: &Token, _span: Span) {
        if matches!(
            token,
            Token::Pub
                | Token::At
                | Token::HashLeftBracket
                | Token::Fails
                | Token::RecoversTo
                | Token::Where
                | Token::Trait
                | Token::Extern
        ) {
            self.reject_profile();
        }
        if self.active.as_ref().is_some_and(|a| a.body.is_some())
            && matches!(
                token,
                Token::Function
                    | Token::Mod
                    | Token::Impl
                    | Token::Use
                    | Token::Requires
                    | Token::Ensures
            )
        {
            self.reject_profile();
        }
    }
    pub(super) fn finish_declaration(&mut self, close: Span, closed: bool) {
        if self.suppressed_declarations > 0 {
            self.suppressed_declarations -= 1;
            if !closed {
                self.malformed = true;
            }
            return;
        }
        let Some(active) = self.active.take() else {
            self.malformed = true;
            return;
        };
        let Some(body) = active.body else {
            self.malformed = true;
            return;
        };
        if !closed || active.name.is_empty() {
            self.malformed = true;
            return;
        }
        let Some(declaration_range) = self
            .source
            .range(active.start.start.offset, close.end.offset)
        else {
            self.malformed = true;
            return;
        };
        let Some(header_range) = self
            .source
            .range(active.start.start.offset, body.start.offset)
        else {
            self.malformed = true;
            return;
        };
        if header_range[1] >= declaration_range[1] {
            self.malformed = true;
            return;
        }
        if self.output_overflow {
            return;
        }
        let identity = json!({
            "schema":"resilient-physical-declaration/v1","profile":PROFILE,
            "source_key":self.source.key,"file_sha256":self.source.digest,
            "kind":active.kind,"scope":active.scope,"name":active.name,
            "fn_keyword_byte":declaration_range[0],"declaration_byte_range":declaration_range,
            "declaration_sha256":self.source.slice_hash(declaration_range),
            "header_byte_range":header_range,"header_sha256":self.source.slice_hash(header_range)
        });
        let declaration_id = identity_id("decl-sha256:", &identity);
        let mut site_ids = Vec::new();
        let mut requires = 0;
        let mut ensures = 0;
        for clause in active.clauses {
            let Some(full) = self
                .source
                .range(clause.keyword.start.offset, clause.last.end.offset)
            else {
                self.malformed = true;
                continue;
            };
            let Some(expression) = self
                .source
                .range(clause.first.start.offset, clause.last.end.offset)
            else {
                self.malformed = true;
                continue;
            };
            if full[0] < header_range[0]
                || full[1] > header_range[1]
                || expression[0] <= full[0]
                || expression[0] >= expression[1]
            {
                self.malformed = true;
                continue;
            }
            let ordinal = match clause.kind {
                "requires" => {
                    let n = requires;
                    requires += 1;
                    n
                }
                "ensures" => {
                    let n = ensures;
                    ensures += 1;
                    n
                }
                _ => {
                    self.malformed = true;
                    continue;
                }
            };
            let identity = json!({"schema":"resilient-physical-clause/v1","profile":PROFILE,
                "source_key":self.source.key,"file_sha256":self.source.digest,
                "declaration_id":declaration_id,"clause_kind":clause.kind,"ordinal":ordinal,
                "full_clause_byte_range":full,"full_clause_sha256":self.source.slice_hash(full),
                "expression_byte_range":expression,"expression_sha256":self.source.slice_hash(expression)});
            let site_id = identity_id("site-sha256:", &identity);
            site_ids.push(site_id.clone());
            let site = json!({"site_id":site_id,"identity":identity});
            self.collected_bytes += serde_json::to_vec(&site).expect("site JSON").len();
            if self.collected_bytes > OUTPUT_LIMIT {
                self.output_overflow = true;
                self.sites.clear();
                self.declarations.clear();
                return;
            }
            self.sites.push(site);
        }
        let declaration =
            json!({"declaration_id":declaration_id,"identity":identity,"site_ids":site_ids});
        self.collected_bytes += serde_json::to_vec(&declaration)
            .expect("declaration JSON")
            .len();
        if self.collected_bytes > OUTPUT_LIMIT {
            self.output_overflow = true;
            self.sites.clear();
            self.declarations.clear();
        } else {
            self.declarations.push(declaration);
        }
    }
    fn report(mut self) -> Result<Value, Error> {
        if self.malformed
            || self.active.is_some()
            || !self.scope.is_empty()
            || self.suppressed_declarations != 0
        {
            return Err(Error::parse("incomplete parser-origin event sequence"));
        }
        if self.rejected {
            return Err(Error::unsupported());
        }
        if self.output_overflow {
            return Err(Error::limit("output-bytes"));
        }
        self.declarations.sort_by_key(|d| {
            d["identity"]["fn_keyword_byte"]
                .as_u64()
                .unwrap_or(u64::MAX)
        });
        self.sites.sort_by_key(|s| {
            s["identity"]["full_clause_byte_range"][0]
                .as_u64()
                .unwrap_or(u64::MAX)
        });
        let mut ids = HashSet::new();
        if self
            .declarations
            .iter()
            .any(|d| !ids.insert(d["declaration_id"].as_str().unwrap_or_default().to_owned()))
            || self
                .sites
                .iter()
                .any(|s| !ids.insert(s["site_id"].as_str().unwrap_or_default().to_owned()))
        {
            return Err(Error::parse("duplicate physical identity"));
        }
        let mut report = self.source.envelope();
        report["disposition"] = json!("accepted");
        report["declarations"] = json!(self.declarations);
        report["sites"] = json!(self.sites);
        Ok(report)
    }
}

/// Explicit recursive sorting also works when serde_json has preserve_order enabled.
fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<_> = map.keys().collect();
            keys.sort();
            let mut sorted = serde_json::Map::new();
            for key in keys {
                sorted.insert(key.clone(), canonical(&map[key]));
            }
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
        _ => value.clone(),
    }
}
fn identity_id(prefix: &str, identity: &Value) -> String {
    format!(
        "{prefix}{}",
        hash(&serde_json::to_vec(&canonical(identity)).expect("JSON identity"))
    )
}
fn identifier(s: &str) -> bool {
    let mut bytes = s.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
fn simple_type(s: &str) -> bool {
    identifier(s)
}
fn scalar(node: &Node) -> bool {
    let mut pending = vec![node];
    let mut visited = 0;
    while let Some(node) = pending.pop() {
        visited += 1;
        if visited > 128 {
            return false;
        }
        match node {
            Node::Identifier { name, .. } if identifier(name) => {}
            Node::IntegerLiteral { .. } | Node::BooleanLiteral { .. } => {}
            Node::PrefixExpression {
                operator, right, ..
            } if matches!(*operator, "!" | "-" | "+" | "~") => pending.push(right),
            Node::InfixExpression {
                left,
                operator,
                right,
                ..
            } if matches!(
                *operator,
                "+" | "-"
                    | "*"
                    | "/"
                    | "%"
                    | "=="
                    | "!="
                    | "<"
                    | "<="
                    | ">"
                    | ">="
                    | "&&"
                    | "||"
                    | "&"
                    | "|"
                    | "^"
                    | "<<"
                    | ">>"
            ) =>
            {
                pending.push(left);
                pending.push(right);
            }
            _ => return false,
        }
    }
    true
}
fn profile_node(node: &Node, depth: usize) -> bool {
    if depth > 128 {
        return false;
    }
    match node {
        Node::Program(nodes) => nodes.iter().all(|n| profile_node(&n.node, depth + 1)),
        Node::ModuleDecl { name, body, .. } => {
            identifier(name) && body.iter().all(|n| profile_node(n, depth + 1))
        }
        Node::Function {
            parameters,
            defaults,
            requires,
            ensures,
            return_type,
            pure,
            type_params,
            type_param_bounds,
            fails,
            recovers_to,
            is_pub,
            ..
        } => {
            !pure
                && !is_pub
                && type_params.is_empty()
                && type_param_bounds.is_empty()
                && fails.is_empty()
                && recovers_to.is_none()
                && defaults.iter().all(Option::is_none)
                && parameters
                    .iter()
                    .all(|(ty, name)| simple_type(ty) && identifier(name))
                && return_type.as_ref().is_none_or(|ty| simple_type(ty))
                && requires.iter().chain(ensures).all(scalar)
        }
        Node::ImplBlock {
            trait_name,
            struct_name,
            methods,
            associated_type_impls,
            ..
        } => {
            trait_name.is_none()
                && identifier(struct_name)
                && associated_type_impls.is_empty()
                && methods.iter().all(|m| profile_node(m, depth + 1))
        }
        Node::StructDecl {
            name,
            type_params,
            fields,
            repr_c,
            ..
        } => {
            identifier(name)
                && type_params.is_empty()
                && !repr_c
                && fields
                    .iter()
                    .all(|(ty, name)| simple_type(ty) && identifier(name))
        }
        Node::Use {
            path,
            alias,
            selectors,
            is_pub,
            ..
        } => {
            !is_pub
                && selectors.is_none()
                && valid_key(path)
                && alias.as_ref().is_none_or(|s| identifier(s))
        }
        _ => false,
    }
}

struct Error {
    code: &'static str,
    message: &'static str,
    exit: i32,
    limit: Option<&'static str>,
    lexical: Option<&'static str>,
}
impl Error {
    fn new(code: &'static str, message: &'static str, exit: i32) -> Self {
        Self {
            code,
            message,
            exit,
            limit: None,
            lexical: None,
        }
    }
    fn parse(message: &'static str) -> Self {
        Self::new("parse-error", message, 1)
    }
    fn unsupported() -> Self {
        Self::new(
            "unsupported-grammar",
            "syntax is outside physical-origins-v1",
            1,
        )
    }
    fn limit(limit: &'static str) -> Self {
        Self {
            limit: Some(limit),
            ..Self::new(
                "resource-limit",
                "physical metadata resource limit exceeded",
                1,
            )
        }
    }
}
fn valid_key(key: &str) -> bool {
    key.len() <= 1024
        && key.ends_with(".rz")
        && key.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
        })
}
fn envelope(
    key: Option<&str>,
    profile: Option<&str>,
    digest: Option<&str>,
    bytes: Option<usize>,
) -> Value {
    json!({"schema":"resilient-physical-origins/v1","profile":profile,"source_key":key,
        "key_authority":"caller-supplied-not-authenticated-path","file_sha256":digest,"bytes":bytes})
}
fn reject(mut envelope: Value, error: Error) -> (Value, i32) {
    envelope["disposition"] = json!("rejected");
    envelope["error"] = json!({"code":error.code,"message":error.message});
    if let Some(limit) = error.limit {
        envelope["error"]["limit"] = json!(limit);
    }
    if let Some(reason) = error.lexical {
        envelope["error"]["lexical_reason"] = json!(reason);
    }
    (envelope, error.exit)
}

/// Stable-filesystem reader: final symlinks/nonregular inputs are rejected before
/// open. Concurrent replacement and intermediate symlink containment are excluded.
fn read_source(path: &Path) -> Result<Vec<u8>, Error> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| Error::new("input-error", "cannot inspect input", 2))?;
    if metadata.file_type().is_symlink() {
        return Err(Error::new(
            "input-symlink",
            "final input symlink is unsupported",
            2,
        ));
    }
    if !metadata.is_file() {
        return Err(Error::new(
            "input-not-regular",
            "input must be a regular file",
            2,
        ));
    }
    if metadata.len() > SOURCE_LIMIT as u64 {
        return Err(Error::limit("source-bytes"));
    }
    let file = File::open(path).map_err(|_| Error::new("input-error", "cannot open input", 2))?;
    let opened = file
        .metadata()
        .map_err(|_| Error::new("input-error", "cannot inspect opened input", 2))?;
    if !opened.is_file() {
        return Err(Error::new(
            "input-not-regular",
            "opened input must be regular",
            2,
        ));
    }
    if opened.len() > SOURCE_LIMIT as u64 {
        return Err(Error::limit("source-bytes"));
    }
    let mut bytes = Vec::new();
    file.take((SOURCE_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::new("input-error", "cannot read input", 2))?;
    if bytes.len() > SOURCE_LIMIT {
        return Err(Error::limit("source-bytes"));
    }
    Ok(bytes)
}

fn produce(source: SourceUnit) -> Result<Value, Error> {
    // The physical profile explicitly uses the real legacy scanner even when
    // normal compilation selects Logos; no cross-lexer parity is claimed.
    let mut lexer = Lexer::new_for_origins(&source.text);
    let mut status = LexicalStatus::default();
    let mut tokens = Vec::new();
    let mut depth = 0usize;
    let mut delimiters = Vec::new();
    let mut mismatched_delimiter = false;
    // Supplemental metadata shape policy v1. A nested Function token cannot
    // reset the enclosing region, including closures and function-type syntax.
    let mut region_depth = None;
    let mut region_tokens = 0usize;
    let mut shape_exceeded = false;
    let mut functions = 0usize;
    let mut clauses = 0usize;
    let mut unsupported_preparse = source.text.starts_with("#!");
    loop {
        let token = lexer.next_token_observed(Some(&mut status));
        let span = Span::new(
            Pos::new(
                lexer.last_token_line,
                lexer.last_token_column,
                lexer.last_token_offset,
            ),
            Pos::new(lexer.line, lexer.column, lexer.position),
        );
        if token == Token::Eof {
            // EOF has a synthetic end one past char-count; never use that end.
            let offset = source.boundaries.len() - 1;
            let pos = Pos::new(lexer.line, lexer.column, offset);
            tokens.push((token, Span::new(pos, pos)));
            break;
        }
        if tokens.len() >= 65_536 {
            return Err(Error::limit("tokens"));
        }
        // Attribute parsing can mutate global registries. Unsupported metadata
        // syntax must not enter that parser path, even on a rejected unit.
        unsupported_preparse |= matches!(token, Token::At | Token::HashLeftBracket);
        if token == Token::Function && region_depth.is_none_or(|base| depth <= base) {
            region_depth = Some(depth);
            region_tokens = 0;
        }
        region_tokens += 1;
        shape_exceeded |= region_tokens > 128;
        if matches!(
            token,
            Token::LeftBrace
                | Token::LeftParen
                | Token::LeftBracket
                | Token::HashLeftBrace
                | Token::HashLeftBracket
        ) {
            depth += 1;
            delimiters.push(match token {
                Token::LeftParen => 0,
                Token::LeftBracket | Token::HashLeftBracket => 1,
                _ => 2,
            });
            if depth > 128 {
                return Err(Error::limit("delimiter-depth"));
            }
        } else if matches!(
            token,
            Token::RightBrace | Token::RightParen | Token::RightBracket
        ) {
            depth = depth.saturating_sub(1);
            let expected = match token {
                Token::RightParen => 0,
                Token::RightBracket => 1,
                _ => 2,
            };
            if delimiters.pop() != Some(expected) {
                mismatched_delimiter = true;
            }
        }
        if token == Token::Function {
            functions += 1;
            if functions > 4096 {
                return Err(Error::limit("function-keywords"));
            }
        }
        if matches!(token, Token::Requires | Token::Ensures) {
            clauses += 1;
            if clauses > 8192 {
                return Err(Error::limit("contract-keywords"));
            }
        }
        if source.range(span.start.offset, span.end.offset).is_none() {
            return Err(Error::parse("nonphysical lexer extent"));
        }
        tokens.push((token, span));
    }
    if let Some(reason) = status.reason {
        return Err(Error {
            lexical: Some(reason),
            ..Error::parse("incomplete physical lexeme")
        });
    }
    // Some normal parser recovery paths synthesize nodes at EOF without a
    // diagnostic. Physical provenance requires actual closed token delimiters.
    if mismatched_delimiter || !delimiters.is_empty() {
        return Err(Error::parse("unclosed or mismatched physical delimiter"));
    }
    // Evaluate only after the original global token/depth/keyword preflight,
    // so its frozen resource-precedence requirements remain unchanged.
    if shape_exceeded {
        return Err(Error::limit("expression-segment-tokens"));
    }
    if unsupported_preparse {
        return Err(Error::unsupported());
    }
    let mut parser = Parser::new_with_origins(lexer, tokens, Observer::new(source));
    let program = parser.parse_program();
    if !parser.errors.is_empty() {
        return Err(Error::parse("parser rejected physical source"));
    }
    let mut observer = parser
        .origins
        .take()
        .ok_or_else(|| Error::parse("missing parser observer"))?;
    if !profile_node(&program, 0) {
        observer.reject_profile();
    }
    observer.report()
}

pub(super) fn dispatch(args: &[String]) -> Option<i32> {
    if args.get(1).map(String::as_str) != Some("source-origins") {
        return None;
    }
    let mut key: Option<&str> = None;
    let mut profile: Option<&str> = None;
    let mut key_count = 0;
    let mut profile_count = 0;
    let mut path = None;
    let mut invalid = false;
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--source-key" | "--profile" => {
                let is_key = args[i] == "--source-key";
                if is_key {
                    key_count += 1;
                } else {
                    profile_count += 1;
                }
                i += 1;
                if let Some(value) = args.get(i).filter(|s| !s.starts_with("--")) {
                    if is_key {
                        key = Some(value);
                    } else {
                        profile = Some(value);
                    }
                } else {
                    invalid = true;
                    i = i.saturating_sub(1);
                }
            }
            value if !value.starts_with('-') && path.is_none() => path = Some(value),
            _ => invalid = true,
        }
        i += 1;
    }
    if key_count != 1 {
        key = None;
    }
    if profile_count != 1 {
        profile = None;
    }
    let mut base = envelope(key, profile, None, None);
    let result = if invalid || path.is_none() || key.is_none() || profile.is_none() {
        Err(Error::new(
            "invalid-cli",
            "one input, source-key and profile are required",
            2,
        ))
    } else if !valid_key(key.unwrap_or_default()) {
        Err(Error::new(
            "invalid-source-key",
            "source-key is not a canonical logical .rz key",
            2,
        ))
    } else if profile != Some(PROFILE) {
        Err(Error::new(
            "unsupported-profile",
            "unsupported physical origins profile",
            2,
        ))
    } else {
        read_source(Path::new(path.unwrap_or_default())).and_then(|bytes| {
            base["file_sha256"] = json!(hash(&bytes));
            base["bytes"] = json!(bytes.len());
            let text = String::from_utf8(bytes)
                .map_err(|_| Error::new("invalid-utf8", "input is not UTF-8", 2))?;
            produce(SourceUnit::new(text, key.unwrap_or_default().to_owned()))
        })
    };
    let (report, exit) = match result {
        Ok(report) => {
            if serde_json::to_vec(&report).expect("origin JSON").len() + 1 > OUTPUT_LIMIT {
                reject(base, Error::limit("output-bytes"))
            } else {
                (report, 0)
            }
        }
        Err(error) => reject(base, error),
    };
    println!("{}", serde_json::to_string(&report).expect("origin JSON"));
    Some(exit)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Supplemental resource policy v1, frozen before the corresponding guard:
    // metadata-only withholding of long valid syntax, never a normal parse limit.
    fn expect_shape_limit(text: String) {
        let error = match produce(SourceUnit::new(text, "fixtures/shape.rz".into())) {
            Err(error) => error,
            Ok(_) => panic!("unsafe expression shape received accepted origins"),
        };
        assert_eq!(error.code, "resource-limit");
        assert_eq!(error.limit, Some("expression-segment-tokens"));
    }

    #[test]
    fn metadata_prefix_and_left_chain_are_bounded_before_parser() {
        expect_shape_limit(format!("fn f() requires {}true {{}}", "!".repeat(2000)));
        expect_shape_limit(format!("fn f(int x) ensures {}x {{}}", "x + ".repeat(1000)));
    }

    #[test]
    fn mixed_parentheses_and_prefixes_cannot_multiply_stack_budget() {
        let open = format!("{}(", "!".repeat(30));
        expect_shape_limit(format!(
            "fn f() requires {}true{} {{}}",
            open.repeat(20),
            ")".repeat(20)
        ));
    }

    #[test]
    fn nested_function_keywords_do_not_reset_outer_budget() {
        expect_shape_limit(format!(
            "fn outer() {{ fn inner() {{ {} }} }}",
            "0;".repeat(100)
        ));
    }

    #[test]
    fn unclosed_physical_fn_module_and_impl_never_receive_origins() {
        for source in ["fn f() {", "mod m { fn f() {}", "impl Cell { fn f(self) {}"] {
            let result = produce(SourceUnit::new(
                source.into(),
                "fixtures/unclosed.rz".into(),
            ));
            assert!(matches!(
                result,
                Err(Error {
                    code: "parse-error",
                    ..
                })
            ));
        }
    }

    #[test]
    fn large_shared_scope_stops_materializing_origin_records() {
        let source = format!(
            "mod {} {{ {} }}",
            "m".repeat(800_000),
            "fn f() {} ".repeat(20)
        );
        let result = produce(SourceUnit::new(source, "fixtures/large-scope.rz".into()));
        assert!(matches!(
            result,
            Err(Error {
                limit: Some("output-bytes"),
                ..
            })
        ));
    }
}
