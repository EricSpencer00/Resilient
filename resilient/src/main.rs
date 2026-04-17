use std::cell::RefCell;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io;
use std::io::Write;
use std::path::Path;
use std::rc::Rc;

// Import modules
mod typechecker;
mod parser;
mod repl;

use rustyline::error::ReadlineError;
use rustyline::{DefaultEditor, Result as RustylineResult};

// Token types for our lexer
#[derive(Debug, Clone, PartialEq)]
enum Token {
    // Keywords
    Function,
    Let,
    Live,
    Assert,
    If,
    Else,
    Return,
    Static,
    While,
    For,
    In,
    Requires,
    Ensures,
    Invariant,
    Struct,
    New,
    Dot,
    Match,
    FatArrow,
    Arrow,
    Underscore,
    Question,
    
    // Literals
    Identifier(String),
    IntLiteral(i64),
    FloatLiteral(f64),
    StringLiteral(String),
    BoolLiteral(bool),
    
    // Operators
    Plus,
    Minus,
    Multiply,
    Divide,
    Modulo,
    Assign,
    Equal,
    NotEqual,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    ShiftLeft,
    ShiftRight,
    Greater,
    Less,
    GreaterEqual,
    LessEqual,
    
    // Delimiters
    LeftParen,
    RightParen,
    LeftBrace,
    RightBrace,
    LeftBracket,
    RightBracket,
    Comma,
    Semicolon,
    Colon,
    
    /// Prefix logical-not.
    Bang,

    // Other
    Eof,
    /// A character the lexer did not recognize. Emitted instead of
    /// panicking so the parser can report a graceful diagnostic. The
    /// `char` payload is the offending character, for the error message.
    Unknown(char),
}

// Lexer for tokenizing Resilient source code
struct Lexer {
    input: Vec<char>,
    position: usize,
    read_position: usize,
    ch: char,
    /// 1-indexed current line, advanced each time we consume a '\n'.
    line: usize,
    /// 1-indexed column at the current `ch`.
    column: usize,
    /// Line/column snapshotted at the START of the most recently
    /// emitted token — so external code can ask "where did this
    /// token begin?".
    last_token_line: usize,
    last_token_column: usize,
}

impl Lexer {
    fn new(input: String) -> Self {
        let mut lexer = Lexer {
            input: input.chars().collect(),
            position: 0,
            read_position: 0,
            ch: '\0',
            line: 1,
            column: 0,
            last_token_line: 1,
            last_token_column: 1,
        };
        lexer.read_char();
        lexer
    }

    fn read_char(&mut self) {
        if self.ch == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        if self.read_position >= self.input.len() {
            self.ch = '\0';
        } else {
            self.ch = self.input[self.read_position];
        }
        self.position = self.read_position;
        self.read_position += 1;
    }
    
    fn peek_char(&self) -> char {
        if self.read_position >= self.input.len() {
            '\0'
        } else {
            self.input[self.read_position]
        }
    }
    
    fn next_token(&mut self) -> Token {
        self.skip_whitespace();
        // Capture where this token STARTS so `Parser` can attribute
        // errors to the correct file:line:col.
        self.last_token_line = self.line;
        self.last_token_column = self.column;
        
        let token = match self.ch {
            '=' => {
                if self.peek_char() == '=' {
                    self.read_char();
                    Token::Equal
                } else if self.peek_char() == '>' {
                    self.read_char();
                    Token::FatArrow
                } else {
                    Token::Assign
                }
            },
            '+' => Token::Plus,
            '-' => {
                if self.peek_char() == '>' {
                    self.read_char();
                    Token::Arrow
                } else {
                    Token::Minus
                }
            },
            '*' => Token::Multiply,
            '%' => Token::Modulo,
            '&' => {
                if self.peek_char() == '&' {
                    self.read_char();
                    Token::And
                } else {
                    Token::BitAnd
                }
            },
            '|' => {
                if self.peek_char() == '|' {
                    self.read_char();
                    Token::Or
                } else {
                    Token::BitOr
                }
            },
            '^' => Token::BitXor,
            '/' => {
                if self.peek_char() == '/' {
                    // Line comment: skip to newline.
                    while self.ch != '\n' && self.ch != '\0' {
                        self.read_char();
                    }
                    return self.next_token();
                } else if self.peek_char() == '*' {
                    // Block comment: skip to '*/' (non-nesting).
                    self.read_char(); // consume first '*'
                    self.read_char(); // advance past it
                    loop {
                        if self.ch == '\0' {
                            // Unterminated block comment — record a lexer
                            // error and stop; return Eof so parser stops.
                            return Token::Unknown('*');
                        }
                        if self.ch == '*' && self.peek_char() == '/' {
                            self.read_char(); // '*'
                            self.read_char(); // '/'
                            break;
                        }
                        self.read_char();
                    }
                    return self.next_token();
                } else {
                    Token::Divide
                }
            },
            '>' => {
                if self.peek_char() == '=' {
                    self.read_char();
                    Token::GreaterEqual
                } else if self.peek_char() == '>' {
                    self.read_char();
                    Token::ShiftRight
                } else {
                    Token::Greater
                }
            },
            '<' => {
                if self.peek_char() == '=' {
                    self.read_char();
                    Token::LessEqual
                } else if self.peek_char() == '<' {
                    self.read_char();
                    Token::ShiftLeft
                } else {
                    Token::Less
                }
            },
            '!' => {
                if self.peek_char() == '=' {
                    self.read_char();
                    Token::NotEqual
                } else {
                    Token::Bang
                }
            },
            '(' => Token::LeftParen,
            ')' => Token::RightParen,
            '{' => Token::LeftBrace,
            '}' => Token::RightBrace,
            '[' => Token::LeftBracket,
            ']' => Token::RightBracket,
            // RES-038: `.` is now a real token (field access). Numeric
            // literals are still fine because read_number consumes `.`
            // before the tokenizer can dispatch here — digit check
            // comes first in next_token's fall-through arm.
            '.' => Token::Dot,
            '?' => Token::Question,
            ',' => Token::Comma,
            ';' => Token::Semicolon,
            ':' => Token::Colon,
            '"' => {
                self.read_char();
                let str_value = self.read_string();
                Token::StringLiteral(str_value)
            },
            '\0' => Token::Eof,
            _ => {
                if self.is_letter(self.ch) {
                    // read_identifier() leaves self.ch at the first character
                    // AFTER the identifier, so we early-return without the
                    // trailing read_char() to avoid swallowing it.
                    let ident = self.read_identifier();
                    return match ident.as_str() {
                        "fn" => Token::Function,
                        "let" => Token::Let,
                        "live" => Token::Live,
                        "assert" => Token::Assert,
                        "if" => Token::If,
                        "else" => Token::Else,
                        "return" => Token::Return,
                        "static" => Token::Static,
                        "while" => Token::While,
                        "for" => Token::For,
                        "in" => Token::In,
                        "requires" => Token::Requires,
                        "ensures" => Token::Ensures,
                        "invariant" => Token::Invariant,
                        "struct" => Token::Struct,
                        "new" => Token::New,
                        "match" => Token::Match,
                        "_" => Token::Underscore,
                        "true" => Token::BoolLiteral(true),
                        "false" => Token::BoolLiteral(false),
                        _ => Token::Identifier(ident),
                    };
                } else if self.is_digit(self.ch) {
                    return self.read_number();
                } else {
                    // Unknown character: emit a token the parser can
                    // route through `record_error` and keep going.
                    let unknown = self.ch;
                    self.read_char();
                    return Token::Unknown(unknown);
                }
            }
        };

        self.read_char();
        token
    }
    
    fn read_identifier(&mut self) -> String {
        let position = self.position;
        while self.is_letter(self.ch) || self.is_digit(self.ch) {
            self.read_char();
        }
        self.input[position..self.position].iter().collect()
    }
    
    fn read_number(&mut self) -> Token {
        // Hex (0x...) and binary (0b...) integer literals first.
        if self.ch == '0' && (self.peek_char() == 'x' || self.peek_char() == 'X') {
            return self.read_radix_number(16, "0x");
        }
        if self.ch == '0' && (self.peek_char() == 'b' || self.peek_char() == 'B') {
            return self.read_radix_number(2, "0b");
        }

        let position = self.position;
        let mut is_float = false;

        while self.is_digit(self.ch) || self.ch == '.' {
            if self.ch == '.' {
                is_float = true;
            }
            self.read_char();
        }

        let number_str: String = self.input[position..self.position].iter().collect();

        if is_float {
            Token::FloatLiteral(number_str.parse::<f64>().unwrap_or(0.0))
        } else {
            Token::IntLiteral(number_str.parse::<i64>().unwrap_or(0))
        }
    }

    /// Consume a `0xHH..` or `0bBB..` integer literal. `prefix` is the
    /// two-character start marker already verified by the caller.
    fn read_radix_number(&mut self, radix: u32, prefix: &str) -> Token {
        // Skip the two-char prefix.
        self.read_char();
        self.read_char();
        let position = self.position;
        let is_valid_digit = |ch: char, r: u32| ch.is_digit(r) || ch == '_';
        while is_valid_digit(self.ch, radix) {
            self.read_char();
        }
        let raw: String = self.input[position..self.position].iter().collect();
        let cleaned = raw.replace('_', "");
        if cleaned.is_empty() {
            // Malformed literal like bare `0x` — best-effort: emit 0.
            // Parser already surfaces these via its own diagnostics if
            // they appear in unexpected positions.
            return Token::IntLiteral(0);
        }
        match i64::from_str_radix(&cleaned, radix) {
            Ok(n) => Token::IntLiteral(n),
            Err(_) => {
                // Overflow or invalid — fall back to 0 and let the
                // parser (or runtime) catch anomalies. A real language
                // would report this through the diagnostics pipeline;
                // once the lexer gains a diagnostic channel (G5), this
                // branch should use it. For now: note the prefix in
                // the returned string representation of a dummy token.
                let _ = prefix;
                Token::IntLiteral(0)
            }
        }
    }
    
    fn read_string(&mut self) -> String {
        let _position = self.position;
        let mut result = String::new();
        
        while self.ch != '"' && self.ch != '\0' {
            // Handle escape sequences
            if self.ch == '\\' && self.read_position < self.input.len() {
                self.read_char(); // Skip the backslash
                
                // Process escape sequence
                match self.ch {
                    'n' => result.push('\n'),
                    't' => result.push('\t'),
                    'r' => result.push('\r'),
                    '\\' => result.push('\\'),
                    '"' => result.push('"'),
                    _ => {
                        // Invalid escape sequence, treat as literal
                        result.push('\\');
                        result.push(self.ch);
                    }
                }
            } else {
                result.push(self.ch);
            }
            
            self.read_char();
        }
        
        result
    }
    
    fn is_letter(&self, ch: char) -> bool {
        ch.is_alphabetic() || ch == '_'
    }
    
    fn is_digit(&self, ch: char) -> bool {
        ch.is_ascii_digit()
    }
    
    fn skip_whitespace(&mut self) {
        while self.ch.is_whitespace() {
            self.read_char();
        }
    }
}

/// RES-039: patterns for `match` arms.
#[derive(Debug, Clone)]
enum Pattern {
    /// Matches a literal int, float, string, or bool.
    Literal(Node),
    /// Binds the scrutinee to an identifier; always matches.
    Identifier(String),
    /// Matches anything without binding (`_`).
    Wildcard,
}

// AST nodes for our parser
#[derive(Debug, Clone)]
enum Node {
    Program(Vec<Node>),
    Function {
        name: String,
        parameters: Vec<(String, String)>, // (type, name)
        body: Box<Node>,
        /// RES-035: pre-condition clauses, checked on entry. Each is a
        /// boolean expression over the parameters.
        requires: Vec<Node>,
        /// RES-035: post-condition clauses, checked on exit. The
        /// special identifier `result` is bound to the return value
        /// inside each clause's env.
        ensures: Vec<Node>,
        /// RES-052: optional `-> TYPE` return-type annotation. Advisory.
        #[allow(dead_code)]
        return_type: Option<String>,
    },
    LiveBlock {
        body: Box<Node>,
        /// RES-036: zero or more invariant expressions checked after
        /// every iteration of the body. A failing invariant triggers
        /// the same retry path as a body-level error.
        invariants: Vec<Node>,
    },
    Assert {
        condition: Box<Node>,
        message: Option<Box<Node>>,
    },
    Block(Vec<Node>),
    LetStatement {
        name: String,
        value: Box<Node>,
        /// RES-052: optional type annotation, e.g. `let x: int = 0;`.
        /// Advisory today; enforced in RES-053.
        #[allow(dead_code)]
        type_annot: Option<String>,
    },
    /// RES-013: `static let NAME = EXPR;` — like let, but stored in a
    /// per-interpreter statics map so the binding survives across
    /// function calls. First evaluation sets the value; subsequent
    /// evaluations are no-ops.
    StaticLet {
        name: String,
        value: Box<Node>,
    },
    /// RES-017: re-bind an existing variable. Fails at runtime if the
    /// name has not been declared with `let` or `static let`.
    Assignment {
        name: String,
        value: Box<Node>,
    },
    ReturnStatement {
        /// `None` for a bare `return;`
        value: Option<Box<Node>>,
    },
    IfStatement {
        condition: Box<Node>,
        consequence: Box<Node>,
        alternative: Option<Box<Node>>,
    },
    /// RES-023: `while COND { BODY }`. Body re-evaluated until COND is falsy.
    WhileStatement {
        condition: Box<Node>,
        body: Box<Node>,
    },
    /// RES-037: `for NAME in EXPR { BODY }`. `EXPR` must evaluate to an
    /// array; `NAME` is bound to each element in order.
    ForInStatement {
        name: String,
        iterable: Box<Node>,
        body: Box<Node>,
    },
    ExpressionStatement(Box<Node>),
    Identifier(String),
    IntegerLiteral(i64),
    FloatLiteral(f64),
    StringLiteral(String),
    BooleanLiteral(bool),
    PrefixExpression {
        operator: String,
        right: Box<Node>,
    },
    InfixExpression {
        left: Box<Node>,
        operator: String,
        right: Box<Node>,
    },
    CallExpression {
        function: Box<Node>,
        arguments: Vec<Node>,
    },
    /// RES-041: `expr?` — if the operand is `Ok(v)`, evaluate to `v`;
    /// if `Err(e)`, return `Err(e)` from the enclosing function.
    TryExpression(Box<Node>),
    /// RES-042: anonymous fn expression. Unlike `Node::Function`, this
    /// node is not bound to a name — it evaluates to a `Value::Function`
    /// directly. Captures its defining env by value, matching existing
    /// named-fn semantics.
    FunctionLiteral {
        parameters: Vec<(String, String)>,
        body: Box<Node>,
        requires: Vec<Node>,
        ensures: Vec<Node>,
        #[allow(dead_code)]
        return_type: Option<String>,
    },
    /// RES-039: `match SCRUTINEE { PATTERN => EXPR, ... }` expression.
    Match {
        scrutinee: Box<Node>,
        arms: Vec<(Pattern, Node)>,
    },
    /// RES-038: `struct NAME { TYPE FIELD, ... }` declaration. Fields
    /// are carried but currently unused at runtime — the typechecker
    /// (G7) will register them in a struct table to verify literal
    /// construction.
    #[allow(dead_code)]
    StructDecl {
        name: String,
        fields: Vec<(String, String)>, // (type, field_name)
    },
    /// RES-038: `NAME { field: expr, ... }` struct literal.
    StructLiteral {
        name: String,
        fields: Vec<(String, Node)>,
    },
    /// RES-038: `target.field` read.
    FieldAccess {
        target: Box<Node>,
        field: String,
    },
    /// RES-038: `target.field = expr` write.
    FieldAssignment {
        target: Box<Node>,
        field: String,
        value: Box<Node>,
    },
    /// RES-032: `[e1, e2, e3]` array literal.
    ArrayLiteral(Vec<Node>),
    /// RES-032: `a[i]` read.
    IndexExpression {
        target: Box<Node>,
        index: Box<Node>,
    },
    /// RES-032: `a[i] = expr` write.
    IndexAssignment {
        target: Box<Node>,
        index: Box<Node>,
        value: Box<Node>,
    },
}

// Parser for creating AST from tokens
struct Parser {
    lexer: Lexer,
    current_token: Token,
    peek_token: Token,
    /// Source position (line, column) of `current_token`. 1-indexed.
    current_line: usize,
    current_column: usize,
    /// Source position of `peek_token`.
    peek_line: usize,
    peek_column: usize,
    errors: Vec<String>,
}

impl Parser {
    fn new(lexer: Lexer) -> Self {
        let mut parser = Parser {
            lexer,
            current_token: Token::Eof,
            peek_token: Token::Eof,
            current_line: 1,
            current_column: 1,
            peek_line: 1,
            peek_column: 1,
            errors: Vec::new(),
        };

        parser.next_token();
        parser.next_token();
        parser
    }

    /// Record an error, prefixing with the start of `current_token`
    /// so users see `line:col: Parser error: ...`.
    fn record_error(&mut self, msg: String) {
        let full = format!(
            "{}:{}: {}",
            self.current_line, self.current_column, msg
        );
        eprintln!("\x1B[31mParser error: {}\x1B[0m", full);
        self.errors.push(full);
    }

    fn next_token(&mut self) {
        self.current_token = self.peek_token.clone();
        self.current_line = self.peek_line;
        self.current_column = self.peek_column;
        self.peek_token = self.lexer.next_token();
        self.peek_line = self.lexer.last_token_line;
        self.peek_column = self.lexer.last_token_column;
    }
    
    fn parse_program(&mut self) -> Node {
        let mut program = Vec::new();
        
        while self.current_token != Token::Eof {
            if let Some(statement) = self.parse_statement() {
                program.push(statement);
            }
            self.next_token();
        }
        
        Node::Program(program)
    }
    
    fn parse_statement(&mut self) -> Option<Node> {
        match self.current_token {
            Token::Function => Some(self.parse_function()),
            Token::Struct => Some(self.parse_struct_decl()),
            Token::Let => Some(self.parse_let_statement()),
            Token::Static => Some(self.parse_static_let_statement()),
            Token::Return => Some(self.parse_return_statement()),
            Token::Live => Some(self.parse_live_block()),
            Token::Assert => Some(self.parse_assert()),
            Token::If => Some(self.parse_if_statement()),
            Token::While => Some(self.parse_while_statement()),
            Token::For => Some(self.parse_for_in_statement()),
            Token::Unknown(ch) => {
                self.record_error(format!("Unexpected character '{}'", ch));
                None
            }
            // Assignment: `IDENT = EXPR;` — disambiguated from an
            // expression statement by looking ahead for `=`.
            Token::Identifier(_) if self.peek_token == Token::Assign => {
                Some(self.parse_assignment())
            }
            // Index / field assignment: `IDENT[...] = EXPR;` or
            // `IDENT.field.more = EXPR;`. We let the expression parser
            // build the full LHS, then disambiguate at the `=`.
            Token::Identifier(_)
                if self.peek_token == Token::LeftBracket
                    || self.peek_token == Token::Dot =>
            {
                Some(self.parse_maybe_index_assignment())
            }
            _ => self.parse_expression_statement(),
        }
    }

    /// Parse either `IDENT[...] = EXPR;` (index assignment) or fall
    /// through to a plain expression statement if no `=` follows the
    /// index. Entered with current_token = the leading Identifier.
    fn parse_maybe_index_assignment(&mut self) -> Node {
        // Parse the index expression (which consumes IDENT, [, index, ]).
        let lhs = self
            .parse_expression(0)
            .unwrap_or(Node::IntegerLiteral(0));
        // If this is an assignment, peek should be `=`.
        if self.peek_token == Token::Assign {
            self.next_token(); // move onto '='
            self.next_token(); // skip '=' to first token of RHS
            let value = self.parse_expression(0).unwrap_or(Node::IntegerLiteral(0));
            if self.peek_token == Token::Semicolon {
                self.next_token();
            }
            // Destructure the LHS to pick the right assignment shape.
            match lhs {
                Node::IndexExpression { target, index } => Node::IndexAssignment {
                    target,
                    index,
                    value: Box::new(value),
                },
                Node::FieldAccess { target, field } => Node::FieldAssignment {
                    target,
                    field,
                    value: Box::new(value),
                },
                _ => Node::ExpressionStatement(Box::new(lhs)),
            }
        } else {
            if self.peek_token == Token::Semicolon {
                self.next_token();
            }
            Node::ExpressionStatement(Box::new(lhs))
        }
    }

    fn parse_assignment(&mut self) -> Node {
        let name = match &self.current_token {
            Token::Identifier(n) => n.clone(),
            _ => unreachable!("parse_assignment only dispatched for Identifier"),
        };
        self.next_token(); // move onto '='
        self.next_token(); // skip '=' to first token of RHS
        let value = self.parse_expression(0).unwrap_or(Node::IntegerLiteral(0));
        if self.peek_token == Token::Semicolon {
            self.next_token();
        }
        Node::Assignment {
            name,
            value: Box::new(value),
        }
    }
    
    fn parse_function(&mut self) -> Node {
        self.next_token(); // Skip 'fn'
        
        let name = match &self.current_token {
            Token::Identifier(name) => name.clone(),
            _ => {
                let tok = self.current_token.clone();
                self.record_error(format!("Expected identifier after 'fn', found {:?}", tok));
                // Return a placeholder to allow parsing to continue
                String::from("error_function")
            },
        };
        
        self.next_token(); // Skip name
        
        // Check if we have a left parenthesis as expected
        if self.current_token != Token::LeftParen {
            // For better error messages, provide more context
            if name == "main" {
                self.record_error(format!("Expected '(' after function name '{}'. Functions in Resilient must have parameters, even if unused. Try: fn main(int dummy) {{ ... }}", name));
            } else {
                self.record_error(format!("Expected '(' after function name '{}'", name));
            }
            
            // Try to recover by skipping to the opening brace
            while self.current_token != Token::LeftBrace && self.current_token != Token::Eof {
                self.next_token();
            }
            
            if self.current_token == Token::Eof {
                return Node::Function {
                    name,
                    parameters: Vec::new(),
                    body: Box::new(Node::Block(Vec::new())),
                    requires: Vec::new(),
                    ensures: Vec::new(),
                    return_type: None,
                };
            }

            let body = self.parse_block_statement();
            return Node::Function {
                name,
                parameters: Vec::new(),
                body: Box::new(body),
                requires: Vec::new(),
                ensures: Vec::new(),
                return_type: None,
            };
        }

        self.next_token(); // Skip '('

        let parameters = self.parse_function_parameters();

        // RES-052: optional `-> TYPE` return type, BEFORE contracts.
        let return_type = self.parse_optional_return_type();

        // RES-035: between the parameter list and the body, accept any
        // number of `requires EXPR` and `ensures EXPR` clauses, in any
        // order. Each clause parses as a single expression.
        let (requires, ensures) = self.parse_function_contracts();

        if self.current_token != Token::LeftBrace {
            self.record_error(format!("Expected '{{' after function parameters for '{}'", name));
            // Try to recover by skipping to the opening brace
            while self.current_token != Token::LeftBrace && self.current_token != Token::Eof {
                self.next_token();
            }

            if self.current_token == Token::Eof {
                return Node::Function {
                    name,
                    parameters,
                    body: Box::new(Node::Block(Vec::new())),
                    requires,
                    ensures,
                    return_type,
                };
            }
        }

        let body = self.parse_block_statement();

        Node::Function {
            name,
            parameters,
            body: Box::new(body),
            requires,
            ensures,
            return_type,
        }
    }

    /// Parse an optional `-> TYPE`. If present, current_token advances
    /// past the type identifier. If absent, no tokens are consumed.
    fn parse_optional_return_type(&mut self) -> Option<String> {
        if self.current_token != Token::Arrow {
            return None;
        }
        self.next_token(); // skip '->'
        let ty = match &self.current_token {
            Token::Identifier(t) => Some(t.clone()),
            _ => {
                let tok = self.current_token.clone();
                self.record_error(format!("Expected type name after '->', found {:?}", tok));
                None
            }
        };
        self.next_token(); // skip type identifier
        ty
    }

    /// Parse zero or more `requires EXPR` / `ensures EXPR` clauses. On
    /// entry current_token is whatever followed the parameter list's
    /// `)`; on exit it's the `{` that starts the body (or whatever
    /// caused parsing to give up).
    fn parse_function_contracts(&mut self) -> (Vec<Node>, Vec<Node>) {
        let mut requires = Vec::new();
        let mut ensures = Vec::new();
        loop {
            match self.current_token {
                Token::Requires => {
                    self.next_token(); // skip `requires`
                    let expr = self.parse_expression(0).unwrap_or(Node::BooleanLiteral(true));
                    self.next_token(); // move past last token of expression
                    requires.push(expr);
                }
                Token::Ensures => {
                    self.next_token();
                    let expr = self.parse_expression(0).unwrap_or(Node::BooleanLiteral(true));
                    self.next_token();
                    ensures.push(expr);
                }
                _ => break,
            }
        }
        (requires, ensures)
    }
    
    fn parse_function_parameters(&mut self) -> Vec<(String, String)> {
        let mut parameters = Vec::new();
        
        if self.current_token == Token::RightParen {
            self.next_token(); // Skip ')'
            return parameters;
        }
        
        while self.current_token != Token::RightParen {
            let param_type = match &self.current_token {
                Token::Identifier(typ) => typ.clone(),
                _ => {
                    let tok = self.current_token.clone();
                    self.record_error(format!("Expected parameter type, found {:?}", tok));
                    // Recover: bail out of the loop; caller will see RightParen
                    // or Eof and stop.
                    break;
                }
            };

            self.next_token(); // Skip type

            let param_name = match &self.current_token {
                Token::Identifier(name) => name.clone(),
                _ => {
                    let tok = self.current_token.clone();
                    self.record_error(format!("Expected parameter name, found {:?}", tok));
                    break;
                }
            };

            parameters.push((param_type, param_name));

            self.next_token(); // Skip name

            if self.current_token == Token::Comma {
                self.next_token(); // Skip ','
            } else if self.current_token != Token::RightParen {
                let tok = self.current_token.clone();
                self.record_error(format!(
                    "Expected ',' or ')' after parameter, found {:?}",
                    tok
                ));
                break;
            }
        }
        
        self.next_token(); // Skip ')'
        parameters
    }
    
    fn parse_block_statement(&mut self) -> Node {
        let mut statements = Vec::new();
        
        self.next_token(); // Skip '{'
        
        while self.current_token != Token::RightBrace && self.current_token != Token::Eof {
            if let Some(stmt) = self.parse_statement() {
                statements.push(stmt);
            }
            self.next_token();
        }
        
        Node::Block(statements)
    }
    
    /// `static let NAME = EXPR;` — parsed into a StaticLet node. The
    /// implementation just reuses parse_let_statement after consuming
    /// the `static` keyword and enforcing that `let` follows.
    /// `for NAME in EXPR { BODY }`
    fn parse_for_in_statement(&mut self) -> Node {
        self.next_token(); // Skip 'for'
        let name = match &self.current_token {
            Token::Identifier(n) => n.clone(),
            _ => {
                let tok = self.current_token.clone();
                self.record_error(format!("Expected identifier after 'for', found {:?}", tok));
                String::new()
            }
        };
        self.next_token(); // skip name
        if self.current_token != Token::In {
            let tok = self.current_token.clone();
            self.record_error(format!("Expected 'in' after 'for {}', found {:?}", name, tok));
            return Node::ForInStatement {
                name,
                iterable: Box::new(Node::ArrayLiteral(Vec::new())),
                body: Box::new(Node::Block(Vec::new())),
            };
        }
        self.next_token(); // skip 'in'
        let iterable = self.parse_expression(0).unwrap_or(Node::ArrayLiteral(Vec::new()));
        self.next_token(); // advance past the expression's tail (RES-014 invariant)

        if self.current_token != Token::LeftBrace {
            let tok = self.current_token.clone();
            self.record_error(format!("Expected '{{' after for-iterable, found {:?}", tok));
            return Node::ForInStatement {
                name,
                iterable: Box::new(iterable),
                body: Box::new(Node::Block(Vec::new())),
            };
        }
        let body = self.parse_block_statement();
        Node::ForInStatement {
            name,
            iterable: Box::new(iterable),
            body: Box::new(body),
        }
    }

    /// `while COND { BODY }` — same parsing shape as `if` (both `while (c)`
    /// and `while c` forms), minus the `else` branch.
    fn parse_while_statement(&mut self) -> Node {
        self.next_token(); // Skip 'while'

        let condition = if self.current_token == Token::LeftParen {
            self.next_token();
            let expr = self.parse_expression(0).unwrap_or(Node::BooleanLiteral(false));
            self.next_token();
            if self.current_token != Token::RightParen {
                let tok = self.current_token.clone();
                self.record_error(format!("Expected ')' after while condition, found {:?}", tok));
            } else {
                self.next_token();
            }
            expr
        } else {
            let expr = self.parse_expression(0).unwrap_or(Node::BooleanLiteral(false));
            self.next_token();
            expr
        };

        if self.current_token != Token::LeftBrace {
            let tok = self.current_token.clone();
            self.record_error(format!("Expected '{{' after while condition, found {:?}", tok));
            return Node::WhileStatement {
                condition: Box::new(condition),
                body: Box::new(Node::Block(Vec::new())),
            };
        }

        let body = self.parse_block_statement();
        Node::WhileStatement {
            condition: Box::new(condition),
            body: Box::new(body),
        }
    }

    fn parse_static_let_statement(&mut self) -> Node {
        self.next_token(); // Skip 'static'
        if self.current_token != Token::Let {
            let tok = self.current_token.clone();
            self.record_error(format!("Expected 'let' after 'static', found {:?}", tok));
            return Node::StaticLet {
                name: String::new(),
                value: Box::new(Node::IntegerLiteral(0)),
            };
        }
        // Delegate to parse_let_statement and re-wrap. parse_let_statement
        // returns a Node::LetStatement.
        let inner = self.parse_let_statement();
        match inner {
            Node::LetStatement { name, value, .. } => Node::StaticLet { name, value },
            other => other, // error paths return a degenerate LetStatement
        }
    }

    fn parse_let_statement(&mut self) -> Node {
        self.next_token(); // Skip 'let'

        let name = match &self.current_token {
            Token::Identifier(name) => name.clone(),
            _ => {
                let tok = self.current_token.clone();
                self.record_error(format!("Expected identifier after 'let', found {:?}", tok));
                return Node::LetStatement {
                    name: String::new(),
                    value: Box::new(Node::IntegerLiteral(0)),
                    type_annot: None,
                };
            }
        };

        self.next_token(); // Skip name

        // RES-052: optional `: TYPE` annotation.
        let type_annot = if self.current_token == Token::Colon {
            self.next_token(); // skip ':'
            let ty = match &self.current_token {
                Token::Identifier(t) => Some(t.clone()),
                _ => {
                    let tok = self.current_token.clone();
                    self.record_error(format!("Expected type name after ':', found {:?}", tok));
                    None
                }
            };
            self.next_token(); // skip type
            ty
        } else {
            None
        };

        if self.current_token != Token::Assign {
            let tok = self.current_token.clone();
            self.record_error(format!(
                "Expected '=' after identifier '{}' in let statement, found {:?}",
                name, tok
            ));
            return Node::LetStatement {
                name,
                value: Box::new(Node::IntegerLiteral(0)),
                type_annot,
            };
        }

        self.next_token(); // Skip '='

        let value = self.parse_expression(0).unwrap();

        if self.peek_token == Token::Semicolon {
            self.next_token(); // Skip to semicolon
        }

        Node::LetStatement {
            name,
            value: Box::new(value),
            type_annot,
        }
    }
    
    fn parse_return_statement(&mut self) -> Node {
        self.next_token(); // Skip 'return'

        // Bare `return;` or `return}` → no expression.
        if matches!(
            self.current_token,
            Token::Semicolon | Token::RightBrace | Token::Eof
        ) {
            return Node::ReturnStatement { value: None };
        }

        let value = match self.parse_expression(0) {
            Some(expr) => Some(Box::new(expr)),
            None => {
                self.record_error(
                    "Expected expression after 'return' (or write 'return;' for no value)".to_string()
                );
                None
            }
        };

        if self.peek_token == Token::Semicolon {
            self.next_token(); // Skip to semicolon
        }

        Node::ReturnStatement { value }
    }
    
    fn parse_live_block(&mut self) -> Node {
        self.next_token(); // Skip 'live'

        // RES-036: zero or more `invariant EXPR` clauses between `live`
        // and `{`.
        let mut invariants = Vec::new();
        while self.current_token == Token::Invariant {
            self.next_token(); // skip `invariant`
            let expr = self.parse_expression(0).unwrap_or(Node::BooleanLiteral(true));
            self.next_token(); // move past last token of the expression
            invariants.push(expr);
        }

        if self.current_token != Token::LeftBrace {
            let tok = self.current_token.clone();
            self.record_error(format!("Expected '{{' after 'live', found {:?}", tok));
            return Node::LiveBlock {
                body: Box::new(Node::Block(Vec::new())),
                invariants,
            };
        }

        let body = self.parse_block_statement();

        Node::LiveBlock {
            body: Box::new(body),
            invariants,
        }
    }
    
    fn parse_assert(&mut self) -> Node {
        self.next_token(); // Skip 'assert'
        
        if self.current_token != Token::LeftParen {
            let tok = self.current_token.clone();
            self.record_error(format!("Expected '(' after 'assert', found {:?}", tok));
            return Node::Assert {
                condition: Box::new(Node::BooleanLiteral(true)),
                message: None,
            };
        }

        self.next_token(); // Skip '('

        let condition = self.parse_expression(0).unwrap_or(Node::BooleanLiteral(true));
        self.next_token(); // RES-014: advance past last token of expression

        let message = if self.current_token == Token::Comma {
            self.next_token(); // Skip ','
            let msg = self.parse_expression(0).unwrap_or(Node::StringLiteral(String::new()));
            self.next_token(); // advance past last token of message expression
            Some(Box::new(msg))
        } else {
            None
        };

        if self.current_token != Token::RightParen {
            let tok = self.current_token.clone();
            self.record_error(format!(
                "Expected ')' after assert condition, found {:?}",
                tok
            ));
        }
        
        Node::Assert {
            condition: Box::new(condition),
            message,
        }
    }
    
    fn parse_if_statement(&mut self) -> Node {
        self.next_token(); // Skip 'if'

        // Handle both `if (condition)` and `if condition` forms.
        //
        // RES-014 invariant note: `parse_expression` leaves `current_token`
        // pointing at the *last token it consumed*. So after the call we
        // must advance once to move past the expression's tail.
        let condition = if self.current_token == Token::LeftParen {
            self.next_token(); // Skip '('
            let expr = self.parse_expression(0).unwrap_or(Node::BooleanLiteral(false));
            self.next_token(); // Advance past last-token-of-expression

            if self.current_token != Token::RightParen {
                let tok = self.current_token.clone();
                self.record_error(format!(
                    "Expected ')' after if condition, found {:?}",
                    tok
                ));
            } else {
                self.next_token(); // Skip ')'
            }
            expr
        } else {
            let expr = self.parse_expression(0).unwrap_or(Node::BooleanLiteral(false));
            self.next_token(); // Advance past last-token-of-expression
            expr
        };

        if self.current_token != Token::LeftBrace {
            let tok = self.current_token.clone();
            self.record_error(format!(
                "Expected '{{' after if condition, found {:?}",
                tok
            ));
            // Recover by returning a skeleton `if` with an empty body so
            // the rest of the file can still be parsed.
            return Node::IfStatement {
                condition: Box::new(condition),
                consequence: Box::new(Node::Block(Vec::new())),
                alternative: None,
            };
        }

        let consequence = self.parse_block_statement();

        let alternative = if self.peek_token == Token::Else {
            self.next_token(); // Move to 'else'
            self.next_token(); // Skip 'else'

            if self.current_token != Token::LeftBrace {
                let tok = self.current_token.clone();
                self.record_error(format!("Expected '{{' after 'else', found {:?}", tok));
                None
            } else {
                Some(Box::new(self.parse_block_statement()))
            }
        } else {
            None
        };

        Node::IfStatement {
            condition: Box::new(condition),
            consequence: Box::new(consequence),
            alternative,
        }
    }
    
    fn parse_expression_statement(&mut self) -> Option<Node> {
        let expr = self.parse_expression(0)?;
        
        if self.peek_token == Token::Semicolon {
            self.next_token(); // Skip to semicolon
        }
        
        Some(Node::ExpressionStatement(Box::new(expr)))
    }
    
    fn parse_expression(&mut self, precedence: u8) -> Option<Node> {
        // Parse prefix expressions
        let mut left_expr = match &self.current_token {
            Token::Identifier(name) => Some(Node::Identifier(name.clone())),
            Token::IntLiteral(value) => Some(Node::IntegerLiteral(*value)),
            Token::FloatLiteral(value) => Some(Node::FloatLiteral(*value)),
            Token::StringLiteral(value) => Some(Node::StringLiteral(value.clone())),
            Token::BoolLiteral(value) => Some(Node::BooleanLiteral(*value)),
            // RES-012: prefix operators `!` and `-`. Precedence is higher
            // than any infix operator, so the operand consumes only the
            // tightest-binding next expression.
            Token::Bang | Token::Minus => {
                let op = if self.current_token == Token::Bang { "!" } else { "-" };
                self.next_token();
                // Prefix precedence is higher than any infix operator
                // so `-1 + 2` parses as `(-1) + 2`, not `-(1 + 2)`.
                let right = self.parse_expression(11)?;
                Some(Node::PrefixExpression {
                    operator: op.to_string(),
                    right: Box::new(right),
                })
            }
            Token::LeftParen => {
                self.next_token(); // Skip '('
                let expr = self.parse_expression(0);
                if self.current_token != Token::RightParen {
                    let tok = self.current_token.clone();
                    self.record_error(format!(
                        "Expected ')' closing parenthesized expression, found {:?}",
                        tok
                    ));
                }
                expr
            },
            Token::LeftBracket => Some(self.parse_array_literal()),
            Token::New => Some(self.parse_struct_literal()),
            Token::Match => Some(self.parse_match_expression()),
            Token::Function => Some(self.parse_function_literal()),
            _ => None,
        };
        
        // Parse infix expressions
        while self.peek_token != Token::Semicolon && precedence < self.peek_precedence() {
            let Some(current_left) = left_expr else {
                // No prefix expression to build on; stop trying to
                // fold infix operators into nothing.
                return None;
            };
            left_expr = match &self.peek_token {
                Token::Plus | Token::Minus | Token::Multiply | Token::Divide | Token::Modulo |
                Token::Equal | Token::NotEqual | Token::Less | Token::Greater |
                Token::LessEqual | Token::GreaterEqual | Token::And | Token::Or |
                Token::BitAnd | Token::BitOr | Token::BitXor |
                Token::ShiftLeft | Token::ShiftRight => {
                    self.next_token();
                    self.parse_infix_expression(current_left)
                },
                Token::LeftParen => {
                    self.next_token();
                    self.parse_call_expression(current_left)
                },
                Token::LeftBracket => {
                    self.next_token(); // move onto '['
                    self.parse_index_expression(current_left)
                },
                Token::Dot => {
                    self.next_token(); // move onto '.'
                    self.parse_field_access(current_left)
                },
                Token::Question => {
                    // Postfix `?` — consume it and wrap.
                    self.next_token();
                    Some(Node::TryExpression(Box::new(current_left)))
                },
                _ => Some(current_left),
            };
        }
        
        left_expr
    }
    
    fn parse_infix_expression(&mut self, left: Node) -> Option<Node> {
        let operator = match &self.current_token {
            Token::Plus => "+".to_string(),
            Token::Minus => "-".to_string(),
            Token::Multiply => "*".to_string(),
            Token::Divide => "/".to_string(),
            Token::Modulo => "%".to_string(),
            Token::And => "&&".to_string(),
            Token::Or => "||".to_string(),
            Token::BitAnd => "&".to_string(),
            Token::BitOr => "|".to_string(),
            Token::BitXor => "^".to_string(),
            Token::ShiftLeft => "<<".to_string(),
            Token::ShiftRight => ">>".to_string(),
            Token::Equal => "==".to_string(),
            Token::NotEqual => "!=".to_string(),
            Token::Less => "<".to_string(),
            Token::Greater => ">".to_string(),
            Token::LessEqual => "<=".to_string(),
            Token::GreaterEqual => ">=".to_string(),
            _ => {
                // Unreachable in practice (the caller only dispatches
                // known operator tokens), but better to report than panic.
                let tok = self.current_token.clone();
                self.record_error(format!("Internal: unexpected operator token {:?}", tok));
                return None;
            }
        };
        
        let precedence = self.current_precedence();
        self.next_token();
        
        let right = self.parse_expression(precedence).unwrap();
        
        Some(Node::InfixExpression {
            left: Box::new(left),
            operator,
            right: Box::new(right),
        })
    }
    
    fn parse_call_expression(&mut self, function: Node) -> Option<Node> {
        let arguments = self.parse_call_arguments();
        
        Some(Node::CallExpression {
            function: Box::new(function),
            arguments,
        })
    }
    
    fn parse_call_arguments(&mut self) -> Vec<Node> {
        let mut args = Vec::new();
        
        if self.peek_token == Token::RightParen {
            self.next_token();
            return args;
        }
        
        self.next_token();
        args.push(self.parse_expression(0).unwrap());
        
        while self.peek_token == Token::Comma {
            self.next_token(); // Skip current
            self.next_token(); // Skip comma
            args.push(self.parse_expression(0).unwrap());
        }
        
        if self.peek_token != Token::RightParen {
            let tok = self.peek_token.clone();
            self.record_error(format!(
                "Expected ')' after call arguments, found {:?}",
                tok
            ));
        } else {
            self.next_token(); // Skip to ')'
        }

        args
    }

    /// Parse `struct NAME { TYPE FIELD, ... }`. current_token is `struct`
    /// on entry; on exit current_token is `}`.
    fn parse_struct_decl(&mut self) -> Node {
        self.next_token(); // skip 'struct'
        let name = match &self.current_token {
            Token::Identifier(n) => n.clone(),
            _ => {
                let tok = self.current_token.clone();
                self.record_error(format!("Expected identifier after 'struct', found {:?}", tok));
                String::new()
            }
        };
        self.next_token();
        if self.current_token != Token::LeftBrace {
            let tok = self.current_token.clone();
            self.record_error(format!("Expected '{{' after struct name, found {:?}", tok));
            return Node::StructDecl { name, fields: Vec::new() };
        }
        self.next_token(); // skip '{'

        let mut fields = Vec::new();
        while self.current_token != Token::RightBrace && self.current_token != Token::Eof {
            let ty = match &self.current_token {
                Token::Identifier(t) => t.clone(),
                _ => {
                    let tok = self.current_token.clone();
                    self.record_error(format!("Expected field type, found {:?}", tok));
                    break;
                }
            };
            self.next_token();
            let fname = match &self.current_token {
                Token::Identifier(n) => n.clone(),
                _ => {
                    let tok = self.current_token.clone();
                    self.record_error(format!("Expected field name after type '{}', found {:?}", ty, tok));
                    break;
                }
            };
            fields.push((ty, fname));
            self.next_token();
            if self.current_token == Token::Comma {
                self.next_token();
            } else if self.current_token != Token::RightBrace {
                let tok = self.current_token.clone();
                self.record_error(format!(
                    "Expected ',' or '}}' after struct field, found {:?}",
                    tok
                ));
                break;
            }
        }
        Node::StructDecl { name, fields }
    }

    /// Parse `new NAME { field: expr, ... }`. current_token is `new`
    /// on entry; on exit current_token is `}`.
    fn parse_struct_literal(&mut self) -> Node {
        self.next_token(); // skip 'new'
        let name = match &self.current_token {
            Token::Identifier(n) => n.clone(),
            _ => {
                let tok = self.current_token.clone();
                self.record_error(format!("Expected struct name after 'new', found {:?}", tok));
                String::new()
            }
        };
        self.next_token();
        if self.current_token != Token::LeftBrace {
            let tok = self.current_token.clone();
            self.record_error(format!("Expected '{{' after struct name, found {:?}", tok));
            return Node::StructLiteral { name, fields: Vec::new() };
        }

        let mut fields: Vec<(String, Node)> = Vec::new();

        if self.peek_token == Token::RightBrace {
            self.next_token(); // to '}'
            return Node::StructLiteral { name, fields };
        }

        self.next_token(); // skip '{'
        loop {
            let fname = match &self.current_token {
                Token::Identifier(n) => n.clone(),
                _ => {
                    let tok = self.current_token.clone();
                    self.record_error(format!(
                        "Expected field name in struct literal, found {:?}",
                        tok
                    ));
                    break;
                }
            };
            self.next_token();
            if self.current_token != Token::Colon {
                let tok = self.current_token.clone();
                self.record_error(format!(
                    "Expected ':' after field name '{}' in struct literal, found {:?}",
                    fname, tok
                ));
                break;
            }
            self.next_token(); // skip ':'
            let value = self.parse_expression(0).unwrap_or(Node::IntegerLiteral(0));
            fields.push((fname, value));
            // parse_expression leaves current on the last token of the
            // expression; advance to move past it.
            self.next_token();
            if self.current_token == Token::Comma {
                self.next_token();
            } else if self.current_token == Token::RightBrace {
                break;
            } else {
                let tok = self.current_token.clone();
                self.record_error(format!(
                    "Expected ',' or '}}' in struct literal, found {:?}",
                    tok
                ));
                break;
            }
        }
        Node::StructLiteral { name, fields }
    }

    /// Parse an anonymous `fn(params) -> TYPE? requires/ensures? { body }`.
    fn parse_function_literal(&mut self) -> Node {
        self.next_token(); // skip 'fn'
        if self.current_token != Token::LeftParen {
            let tok = self.current_token.clone();
            self.record_error(format!(
                "Expected '(' after anonymous 'fn', found {:?}",
                tok
            ));
            return Node::FunctionLiteral {
                parameters: Vec::new(),
                body: Box::new(Node::Block(Vec::new())),
                requires: Vec::new(),
                ensures: Vec::new(),
                return_type: None,
            };
        }
        self.next_token(); // skip '('
        let parameters = self.parse_function_parameters();
        let return_type = self.parse_optional_return_type();
        let (requires, ensures) = self.parse_function_contracts();
        if self.current_token != Token::LeftBrace {
            let tok = self.current_token.clone();
            self.record_error(format!(
                "Expected '{{' in anonymous fn, found {:?}",
                tok
            ));
            return Node::FunctionLiteral {
                parameters,
                body: Box::new(Node::Block(Vec::new())),
                requires,
                ensures,
                return_type,
            };
        }
        let body = self.parse_block_statement();
        Node::FunctionLiteral {
            parameters,
            body: Box::new(body),
            requires,
            ensures,
            return_type,
        }
    }

    /// Parse `match SCRUTINEE { PATTERN => EXPR, ... }`. Current token
    /// is `match` on entry; on exit it's `}`.
    fn parse_match_expression(&mut self) -> Node {
        self.next_token(); // skip 'match'
        let scrutinee = self.parse_expression(0).unwrap_or(Node::BooleanLiteral(false));
        self.next_token(); // past last token of scrutinee

        if self.current_token != Token::LeftBrace {
            let tok = self.current_token.clone();
            self.record_error(format!("Expected '{{' after match scrutinee, found {:?}", tok));
            return Node::Match {
                scrutinee: Box::new(scrutinee),
                arms: Vec::new(),
            };
        }

        let mut arms: Vec<(Pattern, Node)> = Vec::new();
        if self.peek_token == Token::RightBrace {
            self.next_token(); // to '}'
            return Node::Match {
                scrutinee: Box::new(scrutinee),
                arms,
            };
        }

        self.next_token(); // skip '{'
        loop {
            let pattern = self.parse_pattern();
            self.next_token(); // advance past the pattern to '=>'
            if self.current_token != Token::FatArrow {
                let tok = self.current_token.clone();
                self.record_error(format!(
                    "Expected '=>' after match pattern, found {:?}",
                    tok
                ));
                break;
            }
            self.next_token(); // skip '=>'
            let body = self.parse_expression(0).unwrap_or(Node::IntegerLiteral(0));
            arms.push((pattern, body));
            self.next_token(); // past last token of body
            if self.current_token == Token::Comma {
                self.next_token();
            }
            if self.current_token == Token::RightBrace {
                break;
            }
            if matches!(self.current_token, Token::Eof) {
                self.record_error("Unexpected EOF inside match expression".to_string());
                break;
            }
        }

        Node::Match {
            scrutinee: Box::new(scrutinee),
            arms,
        }
    }

    /// Parse a single match pattern. On exit current_token is the
    /// pattern's last token.
    fn parse_pattern(&mut self) -> Pattern {
        match &self.current_token {
            Token::Underscore => Pattern::Wildcard,
            Token::IntLiteral(n) => Pattern::Literal(Node::IntegerLiteral(*n)),
            Token::FloatLiteral(f) => Pattern::Literal(Node::FloatLiteral(*f)),
            Token::StringLiteral(s) => Pattern::Literal(Node::StringLiteral(s.clone())),
            Token::BoolLiteral(b) => Pattern::Literal(Node::BooleanLiteral(*b)),
            Token::Identifier(name) => Pattern::Identifier(name.clone()),
            other => {
                let tok = other.clone();
                self.record_error(format!(
                    "Unsupported match pattern starting with {:?}",
                    tok
                ));
                Pattern::Wildcard
            }
        }
        // Caller expects current_token on the last token of the pattern.
        // All of the above are single-token patterns, so no advance.
    }

    /// Parse `.field`. current_token is `.` on entry; on exit current is `field`.
    fn parse_field_access(&mut self, target: Node) -> Option<Node> {
        self.next_token(); // skip '.'
        let field = match &self.current_token {
            Token::Identifier(n) => n.clone(),
            _ => {
                let tok = self.current_token.clone();
                self.record_error(format!("Expected field name after '.', found {:?}", tok));
                return Some(target);
            }
        };
        Some(Node::FieldAccess {
            target: Box::new(target),
            field,
        })
    }

    /// Parse `[e1, e2, ...]`. current_token is `[` on entry; on exit
    /// current_token is `]`.
    fn parse_array_literal(&mut self) -> Node {
        let mut items = Vec::new();
        if self.peek_token == Token::RightBracket {
            self.next_token(); // to ]
            return Node::ArrayLiteral(items);
        }
        self.next_token(); // skip '['
        if let Some(first) = self.parse_expression(0) {
            items.push(first);
        }
        while self.peek_token == Token::Comma {
            self.next_token(); // to current item's last token
            self.next_token(); // skip ','
            if let Some(next) = self.parse_expression(0) {
                items.push(next);
            }
        }
        if self.peek_token != Token::RightBracket {
            let tok = self.peek_token.clone();
            self.record_error(format!(
                "Expected ']' to close array literal, found {:?}",
                tok
            ));
        } else {
            self.next_token(); // to ]
        }
        Node::ArrayLiteral(items)
    }

    /// Parse `target[index]`. current_token is `[` on entry; on exit
    /// current_token is `]`.
    fn parse_index_expression(&mut self, target: Node) -> Option<Node> {
        self.next_token(); // skip '['
        let index = self.parse_expression(0)?;
        if self.peek_token != Token::RightBracket {
            let tok = self.peek_token.clone();
            self.record_error(format!(
                "Expected ']' to close index expression, found {:?}",
                tok
            ));
            return Some(Node::IndexExpression {
                target: Box::new(target),
                index: Box::new(index),
            });
        }
        self.next_token(); // to ]
        Some(Node::IndexExpression {
            target: Box::new(target),
            index: Box::new(index),
        })
    }

    fn current_precedence(&self) -> u8 {
        match &self.current_token {
            Token::Or => 1,
            Token::And => 2,
            Token::BitOr => 3,
            Token::BitXor => 4,
            Token::BitAnd => 5,
            Token::Equal | Token::NotEqual => 6,
            Token::Less | Token::Greater | Token::LessEqual | Token::GreaterEqual => 7,
            Token::ShiftLeft | Token::ShiftRight => 8,
            Token::Plus | Token::Minus => 9,
            Token::Multiply | Token::Divide | Token::Modulo => 10,
            Token::LeftParen => 11,
            Token::LeftBracket => 11,
            Token::Dot => 11,
            Token::Question => 12,
            _ => 0,
        }
    }
    
    fn peek_precedence(&self) -> u8 {
        match &self.peek_token {
            Token::Or => 1,
            Token::And => 2,
            Token::BitOr => 3,
            Token::BitXor => 4,
            Token::BitAnd => 5,
            Token::Equal | Token::NotEqual => 6,
            Token::Less | Token::Greater | Token::LessEqual | Token::GreaterEqual => 7,
            Token::ShiftLeft | Token::ShiftRight => 8,
            Token::Plus | Token::Minus => 9,
            Token::Multiply | Token::Divide | Token::Modulo => 10,
            Token::LeftParen => 11,
            Token::LeftBracket => 11,
            Token::Dot => 11,
            Token::Question => 12,
            _ => 0,
        }
    }
}

// Signature for native Rust functions exposed to the interpreter.
type BuiltinFn = fn(&[Value]) -> RResult<Value>;

// Value types for our interpreter
#[derive(Clone)]
enum Value {
    Int(i64),
    Float(f64),
    String(String),
    Bool(bool),
    Function {
        parameters: Vec<(String, String)>,
        body: Box<Node>,
        env: Environment,
        /// RES-035: pre-conditions propagated into the runtime Value so
        /// apply_function can check them. Empty when absent.
        requires: Vec<Node>,
        ensures: Vec<Node>,
        /// Function name — used for better contract-violation messages.
        name: String,
    },
    /// Native function. `name` is the identifier it was registered as,
    /// for diagnostics only.
    Builtin {
        name: &'static str,
        func: BuiltinFn,
    },
    /// RES-032: dynamic array. Mixed types allowed at runtime until a
    /// real type system (G7) can enforce a single element type.
    Array(Vec<Value>),
    /// RES-038: user-defined record. Fields are stored in declaration
    /// order so Display is stable.
    Struct {
        name: String,
        fields: Vec<(String, Value)>,
    },
    /// RES-040: first-class Result type.
    ///
    /// `ok = true` means the payload is the success value.
    /// `ok = false` means the payload is the error (typically a
    /// `Value::String`, but any value is allowed).
    Result {
        ok: bool,
        payload: Box<Value>,
    },
    Return(Box<Value>),
    Void,
}

impl std::fmt::Debug for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Value::Int(i) => write!(f, "Int({})", i),
            Value::Float(fl) => write!(f, "Float({})", fl),
            Value::String(s) => write!(f, "String({:?})", s),
            Value::Bool(b) => write!(f, "Bool({})", b),
            Value::Function { parameters, .. } => {
                write!(f, "Function({} params)", parameters.len())
            }
            Value::Builtin { name, .. } => write!(f, "Builtin({})", name),
            Value::Array(items) => write!(f, "Array({} items)", items.len()),
            Value::Struct { name, fields } => {
                write!(f, "Struct({}, {} fields)", name, fields.len())
            }
            Value::Result { ok, payload } => {
                write!(f, "{}({:?})", if *ok { "Ok" } else { "Err" }, payload)
            }
            Value::Return(v) => write!(f, "Return({:?})", v),
            Value::Void => write!(f, "Void"),
        }
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Value::Int(i) => write!(f, "{}", i),
            Value::Float(fl) => write!(f, "{}", fl),
            Value::String(s) => write!(f, "\"{}\"", s),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Function { .. } => write!(f, "<function>"),
            Value::Builtin { name, .. } => write!(f, "<builtin {}>", name),
            Value::Array(items) => {
                write!(f, "[")?;
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")
            }
            Value::Struct { name, fields } => {
                write!(f, "{} {{ ", name)?;
                for (i, (fname, fval)) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", fname, fval)?;
                }
                write!(f, " }}")
            }
            Value::Result { ok, payload } => {
                write!(f, "{}({})", if *ok { "Ok" } else { "Err" }, payload)
            }
            Value::Return(v) => write!(f, "{}", v),
            Value::Void => write!(f, "void"),
        }
    }
}

// Result type for handling errors in our language
type RResult<T> = Result<T, String>;

// Environment for storing variables
#[derive(Debug, Clone)]
struct Environment {
    store: HashMap<String, Value>,
    outer: Option<Box<Environment>>,
}

impl Environment {
    fn new() -> Self {
        Environment {
            store: HashMap::new(),
            outer: None,
        }
    }
    
    fn new_enclosed(outer: Environment) -> Self {
        Environment {
            store: HashMap::new(),
            outer: Some(Box::new(outer)),
        }
    }
    
    fn get(&self, name: &str) -> Option<Value> {
        match self.store.get(name) {
            Some(value) => Some(value.clone()),
            None => {
                if let Some(outer) = &self.outer {
                    outer.get(name)
                } else {
                    None
                }
            }
        }
    }
    
    fn set(&mut self, name: String, value: Value) {
        self.store.insert(name, value);
    }

    /// Update `name` in the frame where it was first defined. Returns
    /// `true` if the name was found and updated, `false` if it doesn't
    /// exist anywhere in the chain.
    fn reassign(&mut self, name: &str, value: Value) -> bool {
        if self.store.contains_key(name) {
            self.store.insert(name.to_string(), value);
            true
        } else if let Some(outer) = self.outer.as_mut() {
            outer.reassign(name, value)
        } else {
            false
        }
    }
}

/// Walk a `FieldAssignment` target tree, collecting the chain of field
/// names. Returns (root_identifier, [field1, field2, ...]). If the root
/// isn't an identifier, the first return is None.
fn flatten_field_target(target: &Node, last_field: &str) -> (Option<String>, Vec<String>) {
    let mut path = vec![last_field.to_string()];
    let mut node = target;
    loop {
        match node {
            Node::Identifier(name) => return (Some(name.clone()), {
                path.reverse();
                path
            }),
            Node::FieldAccess { target: t, field } => {
                path.push(field.clone());
                node = t;
            }
            _ => return (None, Vec::new()),
        }
    }
}

/// Given a root struct value, set the field chain to `new_val` and
/// return the updated root. Errors if any intermediate is not a struct
/// or a field is absent.
fn set_nested_field(root: Value, path: &[String], new_val: Value) -> RResult<Value> {
    if path.is_empty() {
        return Ok(new_val);
    }
    match root {
        Value::Struct { name, mut fields } => {
            let head = &path[0];
            let tail = &path[1..];
            let idx = fields.iter().position(|(n, _)| n == head).ok_or_else(|| {
                format!("Struct {} has no field '{}'", name, head)
            })?;
            let old = std::mem::replace(&mut fields[idx].1, Value::Void);
            let updated = set_nested_field(old, tail, new_val)?;
            fields[idx].1 = updated;
            Ok(Value::Struct { name, fields })
        }
        other => Err(format!(
            "Cannot assign field on non-struct value {:?}",
            other
        )),
    }
}

/// Human-readable rendering of a contract clause for the error message.
/// Deliberately lossy: we just want the user to recognize which clause
/// fired, not reconstruct the full AST.
fn format_contract_expr(node: &Node) -> String {
    match node {
        Node::Identifier(s) => s.clone(),
        Node::IntegerLiteral(n) => n.to_string(),
        Node::FloatLiteral(f) => f.to_string(),
        Node::StringLiteral(s) => format!("{:?}", s),
        Node::BooleanLiteral(b) => b.to_string(),
        Node::PrefixExpression { operator, right } => {
            format!("{}{}", operator, format_contract_expr(right))
        }
        Node::InfixExpression { left, operator, right } => {
            format!(
                "{} {} {}",
                format_contract_expr(left),
                operator,
                format_contract_expr(right)
            )
        }
        Node::CallExpression { function, arguments } => {
            let args: Vec<String> = arguments.iter().map(format_contract_expr).collect();
            format!("{}({})", format_contract_expr(function), args.join(", "))
        }
        Node::IndexExpression { target, index } => {
            format!(
                "{}[{}]",
                format_contract_expr(target),
                format_contract_expr(index)
            )
        }
        _ => "<expr>".to_string(),
    }
}

/// Textual form of a value for string concatenation (`+` with at least one
/// string operand). Returns `None` for values that should NOT be implicitly
/// coerced (functions, builtins, void, returns). Strings come back as their
/// raw contents — *without* the surrounding quotes that `Display` adds.
fn stringify_for_concat(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Int(i) => Some(i.to_string()),
        Value::Float(f) => Some(f.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

// ---------- Builtins ----------
//
// Native functions registered into every Interpreter's top-level
// environment. Keep this list small and predictable — it is the
// language's minimal stdlib until a proper module system arrives.

fn register_builtins(env: &mut Environment) {
    for (name, func) in BUILTINS {
        env.set(
            (*name).to_string(),
            Value::Builtin {
                name,
                func: *func,
            },
        );
    }
}

/// Canonical list of every native function visible in a fresh
/// Resilient program.
const BUILTINS: &[(&str, BuiltinFn)] = &[
    ("println", builtin_println),
    ("print", builtin_print),
    ("abs", builtin_abs),
    ("min", builtin_min),
    ("max", builtin_max),
    ("sqrt", builtin_sqrt),
    ("pow", builtin_pow),
    ("floor", builtin_floor),
    ("ceil", builtin_ceil),
    ("len", builtin_len),
    ("push", builtin_push),
    ("pop", builtin_pop),
    ("slice", builtin_slice),
    ("split", builtin_split),
    ("trim", builtin_trim),
    ("contains", builtin_contains),
    ("to_upper", builtin_to_upper),
    ("to_lower", builtin_to_lower),
    ("Ok", builtin_ok),
    ("Err", builtin_err),
    ("is_ok", builtin_is_ok),
    ("is_err", builtin_is_err),
    ("unwrap", builtin_unwrap),
    ("unwrap_err", builtin_unwrap_err),
];

/// Print the single argument followed by a newline and return `Void`.
///
/// Strings print without surrounding quotes (so `println("hi")` writes
/// `hi`, not `"hi"`). Other values print via their `Display` impl.
fn builtin_println(args: &[Value]) -> RResult<Value> {
    match args {
        [] => {
            println!();
            Ok(Value::Void)
        }
        [single] => {
            match single {
                Value::String(s) => println!("{}", s),
                other => println!("{}", other),
            }
            Ok(Value::Void)
        }
        many => Err(format!(
            "println expects 0 or 1 argument, got {}",
            many.len()
        )),
    }
}

/// `print(x)` — like println but without the trailing newline. Useful
/// for building a line from multiple values or for prompt-style output.
fn builtin_print(args: &[Value]) -> RResult<Value> {
    use std::io::Write as _;
    match args {
        [] => {
            // No-op with flush so partial-line state is consistent.
            let _ = std::io::stdout().flush();
            Ok(Value::Void)
        }
        [single] => {
            match single {
                Value::String(s) => print!("{}", s),
                other => print!("{}", other),
            }
            let _ = std::io::stdout().flush();
            Ok(Value::Void)
        }
        many => Err(format!("print expects 0 or 1 argument, got {}", many.len())),
    }
}

/// `sqrt(x)` — square root, float-returning. Int arg coerced to f64.
fn builtin_sqrt(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(i)] => Ok(Value::Float((*i as f64).sqrt())),
        [Value::Float(f)] => Ok(Value::Float(f.sqrt())),
        [other] => Err(format!("sqrt: expected numeric, got {:?}", other)),
        _ => Err(format!("sqrt: expected 1 argument, got {}", args.len())),
    }
}

/// `pow(base, exp)` — base^exp. Float-returning.
fn builtin_pow(args: &[Value]) -> RResult<Value> {
    let to_f = |v: &Value| match v {
        Value::Int(i) => Some(*i as f64),
        Value::Float(f) => Some(*f),
        _ => None,
    };
    match args {
        [a, b] => {
            let (Some(base), Some(exp)) = (to_f(a), to_f(b)) else {
                return Err(format!("pow: expected numeric args, got {:?} and {:?}", a, b));
            };
            Ok(Value::Float(base.powf(exp)))
        }
        _ => Err(format!("pow: expected 2 arguments, got {}", args.len())),
    }
}

/// `floor(x)` — round toward negative infinity. Always returns float.
fn builtin_floor(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(i)] => Ok(Value::Float(*i as f64)),
        [Value::Float(f)] => Ok(Value::Float(f.floor())),
        [other] => Err(format!("floor: expected numeric, got {:?}", other)),
        _ => Err(format!("floor: expected 1 argument, got {}", args.len())),
    }
}

/// `ceil(x)` — round toward positive infinity. Always returns float.
fn builtin_ceil(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(i)] => Ok(Value::Float(*i as f64)),
        [Value::Float(f)] => Ok(Value::Float(f.ceil())),
        [other] => Err(format!("ceil: expected numeric, got {:?}", other)),
        _ => Err(format!("ceil: expected 1 argument, got {}", args.len())),
    }
}

/// `Ok(v)` — wrap a success value as a Result.
fn builtin_ok(args: &[Value]) -> RResult<Value> {
    match args {
        [v] => Ok(Value::Result {
            ok: true,
            payload: Box::new(v.clone()),
        }),
        _ => Err(format!("Ok: expected 1 argument, got {}", args.len())),
    }
}

/// `Err(e)` — wrap a failure value as a Result.
fn builtin_err(args: &[Value]) -> RResult<Value> {
    match args {
        [v] => Ok(Value::Result {
            ok: false,
            payload: Box::new(v.clone()),
        }),
        _ => Err(format!("Err: expected 1 argument, got {}", args.len())),
    }
}

/// `is_ok(r)` — true iff `r` is an Ok-tagged Result.
fn builtin_is_ok(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Result { ok, .. }] => Ok(Value::Bool(*ok)),
        [other] => Err(format!("is_ok: expected Result, got {:?}", other)),
        _ => Err(format!("is_ok: expected 1 argument, got {}", args.len())),
    }
}

/// `is_err(r)` — true iff `r` is an Err-tagged Result.
fn builtin_is_err(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Result { ok, .. }] => Ok(Value::Bool(!ok)),
        [other] => Err(format!("is_err: expected Result, got {:?}", other)),
        _ => Err(format!("is_err: expected 1 argument, got {}", args.len())),
    }
}

/// `unwrap(r)` — return the Ok payload or error at runtime.
fn builtin_unwrap(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Result { ok: true, payload }] => Ok((**payload).clone()),
        [Value::Result { ok: false, payload }] => {
            Err(format!("unwrap called on Err({})", payload))
        }
        [other] => Err(format!("unwrap: expected Result, got {:?}", other)),
        _ => Err(format!("unwrap: expected 1 argument, got {}", args.len())),
    }
}

/// `unwrap_err(r)` — return the Err payload or error at runtime.
fn builtin_unwrap_err(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Result { ok: false, payload }] => Ok((**payload).clone()),
        [Value::Result { ok: true, payload }] => {
            Err(format!("unwrap_err called on Ok({})", payload))
        }
        [other] => Err(format!("unwrap_err: expected Result, got {:?}", other)),
        _ => Err(format!(
            "unwrap_err: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `split(s, sep)` — split `s` at every occurrence of `sep`, returning
/// an array of pieces. Empty `sep` splits into Unicode scalars.
fn builtin_split(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(s), Value::String(sep)] => {
            let parts: Vec<Value> = if sep.is_empty() {
                s.chars().map(|c| Value::String(c.to_string())).collect()
            } else {
                s.split(sep.as_str()).map(|p| Value::String(p.to_string())).collect()
            };
            Ok(Value::Array(parts))
        }
        [a, b] => Err(format!(
            "split: expected (string, string), got ({:?}, {:?})",
            a, b
        )),
        _ => Err(format!("split: expected 2 arguments, got {}", args.len())),
    }
}

/// `trim(s)` — strip leading and trailing ASCII whitespace.
fn builtin_trim(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(s)] => Ok(Value::String(s.trim().to_string())),
        [other] => Err(format!("trim: expected string, got {:?}", other)),
        _ => Err(format!("trim: expected 1 argument, got {}", args.len())),
    }
}

/// `contains(haystack, needle)` — substring test.
fn builtin_contains(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(h), Value::String(n)] => Ok(Value::Bool(h.contains(n.as_str()))),
        [a, b] => Err(format!(
            "contains: expected (string, string), got ({:?}, {:?})",
            a, b
        )),
        _ => Err(format!("contains: expected 2 arguments, got {}", args.len())),
    }
}

/// `to_upper(s)` — Unicode uppercase.
fn builtin_to_upper(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(s)] => Ok(Value::String(s.to_uppercase())),
        [other] => Err(format!("to_upper: expected string, got {:?}", other)),
        _ => Err(format!("to_upper: expected 1 argument, got {}", args.len())),
    }
}

/// `to_lower(s)` — Unicode lowercase.
fn builtin_to_lower(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(s)] => Ok(Value::String(s.to_lowercase())),
        [other] => Err(format!("to_lower: expected string, got {:?}", other)),
        _ => Err(format!("to_lower: expected 1 argument, got {}", args.len())),
    }
}

/// `push(arr, x)` — returns a new array with `x` appended. The input
/// array is not mutated (pass-by-value semantics).
fn builtin_push(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items), x] => {
            let mut out = items.clone();
            out.push(x.clone());
            Ok(Value::Array(out))
        }
        [other, _] => Err(format!("push: expected array as first arg, got {:?}", other)),
        _ => Err(format!("push: expected 2 arguments, got {}", args.len())),
    }
}

/// `pop(arr)` — returns a new array without the last element. Errors on empty.
fn builtin_pop(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items)] => {
            if items.is_empty() {
                Err("pop: cannot pop from an empty array".to_string())
            } else {
                let mut out = items.clone();
                out.pop();
                Ok(Value::Array(out))
            }
        }
        [other] => Err(format!("pop: expected array, got {:?}", other)),
        _ => Err(format!("pop: expected 1 argument, got {}", args.len())),
    }
}

/// `slice(arr, start, end)` — half-open range `[start, end)`, returning a new array.
fn builtin_slice(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items), Value::Int(start), Value::Int(end)] => {
            let len = items.len() as i64;
            if *start < 0 || *end < 0 || *start > len || *end > len || *start > *end {
                return Err(format!(
                    "slice: range [{}, {}) is invalid for array of length {}",
                    start, end, len
                ));
            }
            let s = *start as usize;
            let e = *end as usize;
            Ok(Value::Array(items[s..e].to_vec()))
        }
        [a, b, c] => Err(format!(
            "slice: expected (array, int, int), got ({:?}, {:?}, {:?})",
            a, b, c
        )),
        _ => Err(format!("slice: expected 3 arguments, got {}", args.len())),
    }
}

/// `len(x)` — element count. For strings: Unicode scalar count (not bytes).
/// For arrays: number of items.
fn builtin_len(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(s)] => Ok(Value::Int(s.chars().count() as i64)),
        [Value::Array(items)] => Ok(Value::Int(items.len() as i64)),
        [other] => Err(format!("len: expected string or array, got {:?}", other)),
        _ => Err(format!("len: expected 1 argument, got {}", args.len())),
    }
}

/// `abs(x)` — absolute value for `int` and `float`.
fn builtin_abs(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(i)] => Ok(Value::Int(i.abs())),
        [Value::Float(f)] => Ok(Value::Float(f.abs())),
        [other] => Err(format!("abs: expected int or float, got {:?}", other)),
        _ => Err(format!("abs: expected 1 argument, got {}", args.len())),
    }
}

/// `min(a, b)` — smaller of two numeric values. Coerces int↔float.
fn builtin_min(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(a), Value::Int(b)] => Ok(Value::Int((*a).min(*b))),
        [Value::Float(a), Value::Float(b)] => Ok(Value::Float(a.min(*b))),
        [Value::Int(a), Value::Float(b)] => Ok(Value::Float((*a as f64).min(*b))),
        [Value::Float(a), Value::Int(b)] => Ok(Value::Float(a.min(*b as f64))),
        [a, b] => Err(format!("min: expected numeric args, got {:?} and {:?}", a, b)),
        _ => Err(format!("min: expected 2 arguments, got {}", args.len())),
    }
}

/// `max(a, b)` — larger of two numeric values. Coerces int↔float.
fn builtin_max(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(a), Value::Int(b)] => Ok(Value::Int((*a).max(*b))),
        [Value::Float(a), Value::Float(b)] => Ok(Value::Float(a.max(*b))),
        [Value::Int(a), Value::Float(b)] => Ok(Value::Float((*a as f64).max(*b))),
        [Value::Float(a), Value::Int(b)] => Ok(Value::Float(a.max(*b as f64))),
        [a, b] => Err(format!("max: expected numeric args, got {:?} and {:?}", a, b)),
        _ => Err(format!("max: expected 2 arguments, got {}", args.len())),
    }
}

// Interpreter for executing Resilient programs
struct Interpreter {
    env: Environment,
    /// RES-013: static-let bindings. Shared across every sub-interpreter
    /// created for function calls so the values survive across invocations.
    /// Keyed by the static's identifier (caveat: two functions using the
    /// same static name currently share — good enough for MVP).
    statics: Rc<RefCell<HashMap<String, Value>>>,
}

impl Interpreter {
    fn new() -> Self {
        let mut env = Environment::new();
        register_builtins(&mut env);
        Interpreter {
            env,
            statics: Rc::new(RefCell::new(HashMap::new())),
        }
    }
    
    fn eval(&mut self, node: &Node) -> RResult<Value> {
        match node {
            Node::Program(statements) => self.eval_program(statements),
            Node::Function { name, parameters, body, requires, ensures, .. } => {
                let func = Value::Function {
                    parameters: parameters.clone(),
                    body: body.clone(),
                    env: self.env.clone(),
                    requires: requires.clone(),
                    ensures: ensures.clone(),
                    name: name.clone(),
                };
                self.env.set(name.clone(), func);
                Ok(Value::Void)
            },
            Node::LiveBlock { body, invariants } => self.eval_live_block(body, invariants),
            Node::Assert { condition, message } => self.eval_assert(condition, message),
            Node::Block(statements) => self.eval_block_statement(statements),
            Node::LetStatement { name, value, .. } => {
                let val = self.eval(value)?;
                // RES-041: if the RHS short-circuited (e.g. via `?`),
                // propagate the Return instead of binding it.
                if matches!(val, Value::Return(_)) {
                    return Ok(val);
                }
                self.env.set(name.clone(), val);
                Ok(Value::Void)
            },
            Node::StaticLet { name, value } => {
                // Initialize only once. Subsequent executions of the
                // same declaration are no-ops (the value persists in
                // self.statics across function calls).
                if !self.statics.borrow().contains_key(name) {
                    let val = self.eval(value)?;
                    self.statics.borrow_mut().insert(name.clone(), val);
                }
                Ok(Value::Void)
            },
            Node::Assignment { name, value } => {
                let val = self.eval(value)?;
                if matches!(val, Value::Return(_)) {
                    return Ok(val);
                }
                if self.env.reassign(name, val.clone()) {
                    Ok(Value::Void)
                } else if self.statics.borrow().contains_key(name) {
                    self.statics.borrow_mut().insert(name.clone(), val);
                    Ok(Value::Void)
                } else {
                    Err(format!("Cannot assign to undeclared variable '{}'", name))
                }
            },
            Node::ReturnStatement { value } => {
                let val = match value {
                    Some(expr) => self.eval(expr)?,
                    None => Value::Void,
                };
                Ok(Value::Return(Box::new(val)))
            },
            Node::ForInStatement { name, iterable, body } => {
                let iter_val = self.eval(iterable)?;
                let items = match iter_val {
                    Value::Array(v) => v,
                    other => return Err(format!(
                        "`for` iterable must be an array, got {:?}",
                        other
                    )),
                };
                for item in items {
                    self.env.set(name.clone(), item);
                    let result = self.eval(body)?;
                    if let Value::Return(_) = result {
                        return Ok(result);
                    }
                }
                Ok(Value::Void)
            },
            Node::WhileStatement { condition, body } => {
                // Cap iterations as a safety net so a buggy loop can't
                // freeze the interpreter. 1M is big enough for
                // realistic work and small enough to catch runaways.
                const MAX_ITERS: usize = 1_000_000;
                let mut iters = 0usize;
                loop {
                    iters += 1;
                    if iters > MAX_ITERS {
                        return Err(format!(
                            "while loop exceeded {MAX_ITERS} iterations (runaway?)"
                        ));
                    }
                    let cond_val = self.eval(condition)?;
                    if !self.is_truthy(&cond_val) {
                        break;
                    }
                    let result = self.eval(body)?;
                    if let Value::Return(_) = result {
                        return Ok(result);
                    }
                }
                Ok(Value::Void)
            },
            Node::IfStatement { condition, consequence, alternative } => {
                let condition_value = self.eval(condition)?;
                if self.is_truthy(&condition_value) {
                    self.eval(consequence)
                } else if let Some(alt) = alternative {
                    self.eval(alt)
                } else {
                    Ok(Value::Void)
                }
            },
            Node::ExpressionStatement(expr) => self.eval(expr),
            Node::Identifier(name) => {
                if let Some(value) = self.env.get(name) {
                    Ok(value)
                } else if let Some(value) = self.statics.borrow().get(name).cloned() {
                    Ok(value)
                } else {
                    Err(format!("Identifier not found: {}", name))
                }
            },
            Node::IntegerLiteral(value) => Ok(Value::Int(*value)),
            Node::FloatLiteral(value) => Ok(Value::Float(*value)),
            Node::StringLiteral(value) => Ok(Value::String(value.clone())),
            Node::BooleanLiteral(value) => Ok(Value::Bool(*value)),
            Node::PrefixExpression { operator, right } => {
                let right_val = self.eval(right)?;
                self.eval_prefix_expression(operator, right_val)
            },
            Node::InfixExpression { left, operator, right } => {
                let left_val = self.eval(left)?;
                let right_val = self.eval(right)?;
                self.eval_infix_expression(operator, left_val, right_val)
            },
            Node::CallExpression { function, arguments } => {
                let func = self.eval(function)?;
                let args = self.eval_expressions(arguments)?;
                self.apply_function(func, args)
            },
            Node::ArrayLiteral(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.eval(item)?);
                }
                Ok(Value::Array(out))
            },
            Node::FunctionLiteral { parameters, body, requires, ensures, .. } => {
                Ok(Value::Function {
                    parameters: parameters.clone(),
                    body: body.clone(),
                    env: self.env.clone(),
                    requires: requires.clone(),
                    ensures: ensures.clone(),
                    name: "<anon>".to_string(),
                })
            },
            Node::TryExpression(inner) => {
                let v = self.eval(inner)?;
                match v {
                    Value::Result { ok: true, payload } => Ok(*payload),
                    Value::Result { ok: false, payload } => {
                        // Propagate: return Err(payload) from the
                        // enclosing function via the Value::Return
                        // short-circuit path already used by `return`.
                        Ok(Value::Return(Box::new(Value::Result {
                            ok: false,
                            payload,
                        })))
                    }
                    other => Err(format!(
                        "? operator expects a Result, got {:?}",
                        other
                    )),
                }
            },
            Node::Match { scrutinee, arms } => {
                let sval = self.eval(scrutinee)?;
                for (pattern, body) in arms {
                    if let Some(binding) = self.match_pattern(pattern, &sval)? {
                        if let Some((name, value)) = binding {
                            // Create a transient enclosing env so the
                            // identifier pattern's binding doesn't
                            // leak out of the arm.
                            let saved = self.env.clone();
                            self.env = Environment::new_enclosed(saved.clone());
                            self.env.set(name, value);
                            let result = self.eval(body);
                            self.env = saved;
                            return result;
                        } else {
                            return self.eval(body);
                        }
                    }
                }
                // No arm matched → void.
                Ok(Value::Void)
            },
            Node::StructDecl { .. } => {
                // Declarations are pure compile-time metadata today.
                // The typechecker (G7) will register them in a struct
                // table; for now they're a runtime no-op, and Value
                // construction trusts the literal.
                Ok(Value::Void)
            },
            Node::StructLiteral { name, fields } => {
                let mut out = Vec::with_capacity(fields.len());
                for (fname, fexpr) in fields {
                    out.push((fname.clone(), self.eval(fexpr)?));
                }
                Ok(Value::Struct {
                    name: name.clone(),
                    fields: out,
                })
            },
            Node::FieldAccess { target, field } => {
                let tval = self.eval(target)?;
                match tval {
                    Value::Struct { name, fields } => {
                        fields
                            .into_iter()
                            .find(|(n, _)| n == field)
                            .map(|(_, v)| v)
                            .ok_or_else(|| {
                                format!("Struct {} has no field '{}'", name, field)
                            })
                    }
                    other => Err(format!(
                        "Cannot access field '{}' on non-struct {:?}",
                        field, other
                    )),
                }
            },
            Node::FieldAssignment { target, field, value } => {
                // Only support `IDENT.field = v` and `IDENT.f1.f2 = v`
                // for MVP. The target tree is a chain of Identifier and
                // FieldAccess nodes; we walk it to find the root binding,
                // then mutate a cloned copy and reassign.
                let new_val = self.eval(value)?;
                let (root_name, path) = flatten_field_target(target, field);
                let Some(root_name) = root_name else {
                    return Err(
                        "Field assignment target must start with an identifier"
                            .to_string(),
                    );
                };
                let current = self
                    .env
                    .get(&root_name)
                    .ok_or_else(|| format!("Identifier not found: {}", root_name))?;
                let updated = set_nested_field(current, &path, new_val)?;
                let _ = self.env.reassign(&root_name, updated);
                Ok(Value::Void)
            },
            Node::IndexExpression { target, index } => {
                let target_val = self.eval(target)?;
                let index_val = self.eval(index)?;
                match (target_val, index_val) {
                    (Value::Array(items), Value::Int(i)) => {
                        if i < 0 || (i as usize) >= items.len() {
                            Err(format!(
                                "Index {} out of bounds for array of length {}",
                                i,
                                items.len()
                            ))
                        } else {
                            Ok(items[i as usize].clone())
                        }
                    }
                    (Value::Array(_), other) => Err(format!(
                        "Array index must be int, got {:?}",
                        other
                    )),
                    (other, _) => Err(format!("Cannot index {:?}", other)),
                }
            },
            Node::IndexAssignment { target, index, value } => {
                // target must be an identifier (restricted form for now).
                let name = match target.as_ref() {
                    Node::Identifier(n) => n.clone(),
                    _ => return Err("Index assignment target must be an identifier".to_string()),
                };
                let index_val = self.eval(index)?;
                let new_val = self.eval(value)?;
                let Value::Int(i) = index_val else {
                    return Err(format!("Array index must be int, got {:?}", index_val));
                };
                // Read, modify, write. This relies on Environment storing
                // arrays by value; true aliasing would need Rc/RefCell
                // and is tracked as a future ticket.
                let current = self
                    .env
                    .get(&name)
                    .ok_or_else(|| format!("Identifier not found: {}", name))?;
                let Value::Array(mut items) = current else {
                    return Err(format!("Cannot index-assign into non-array '{}'", name));
                };
                if i < 0 || (i as usize) >= items.len() {
                    return Err(format!(
                        "Index {} out of bounds for array of length {}",
                        i,
                        items.len()
                    ));
                }
                items[i as usize] = new_val;
                let _ = self.env.reassign(&name, Value::Array(items));
                Ok(Value::Void)
            },
        }
    }
    
    fn eval_program(&mut self, statements: &[Node]) -> RResult<Value> {
        // RES-018: hoist function bindings so they can forward-reference
        // each other. First pass: bind every top-level fn. Second pass:
        // re-bind so each captured env includes ALL sibling functions.
        // Then run non-fn statements in declaration order.
        for statement in statements {
            if matches!(statement, Node::Function { .. }) {
                self.eval(statement)?;
            }
        }
        for statement in statements {
            if matches!(statement, Node::Function { .. }) {
                self.eval(statement)?;
            }
        }

        let mut result = Value::Void;
        for statement in statements {
            if matches!(statement, Node::Function { .. }) {
                continue;
            }
            result = self.eval(statement)?;
            if let Value::Return(value) = result {
                return Ok(*value);
            }
        }

        Ok(result)
    }
    
    fn eval_block_statement(&mut self, statements: &[Node]) -> RResult<Value> {
        let mut result = Value::Void;
        
        for statement in statements {
            result = self.eval(statement)?;
            
            if let Value::Return(_) = result {
                return Ok(result);
            }
        }
        
        Ok(result)
    }
    
    fn eval_live_block(&mut self, body: &Node, invariants: &[Node]) -> RResult<Value> {
        const MAX_RETRIES: usize = 3;
        let mut retry_count = 0;

        // Create a snapshot of the environment
        let env_snapshot = self.env.clone();

        // Log the start of live block execution
        eprintln!("\x1B[36m[LIVE BLOCK] Starting execution of live block\x1B[0m");

        // Try to evaluate the body with multiple retries
        loop {
            // RES-036: treat an invariant failure as the same class of
            // recoverable error the retry loop already handles. The
            // body eval either succeeds or returns Err; then we check
            // each invariant and convert a false result into an Err.
            let outcome = self.eval(body).and_then(|value| {
                for clause in invariants {
                    let v = self.eval(clause)?;
                    if !self.is_truthy(&v) {
                        return Err(format!(
                            "Invariant violation in live block: {} failed",
                            format_contract_expr(clause)
                        ));
                    }
                }
                Ok(value)
            });

            match outcome {
                Ok(value) => {
                    eprintln!("\x1B[32m[LIVE BLOCK] Successfully executed live block\x1B[0m");
                    return Ok(value);
                }
                Err(error) => {
                    retry_count += 1;

                    eprintln!(
                        "\x1B[33m[LIVE BLOCK] Error detected (attempt {}/{}): {}\x1B[0m",
                        retry_count, MAX_RETRIES, error
                    );

                    if retry_count >= MAX_RETRIES {
                        eprintln!(
                            "\x1B[31m[LIVE BLOCK] Maximum retry attempts reached, propagating error\x1B[0m"
                        );
                        return Err(format!(
                            "Live block failed after {} attempts: {}",
                            MAX_RETRIES, error
                        ));
                    }

                    eprintln!(
                        "\x1B[36m[LIVE BLOCK] Restoring environment to last known good state\x1B[0m"
                    );
                    eprintln!(
                        "\x1B[36m[LIVE BLOCK] Retrying execution (attempt {}/{})\x1B[0m",
                        retry_count + 1,
                        MAX_RETRIES
                    );

                    // Restore the environment from the snapshot
                    self.env = env_snapshot.clone();
                }
            }
        }
    }
    
    fn eval_assert(&mut self, condition: &Node, message: &Option<Box<Node>>) -> RResult<Value> {
        let condition_value = self.eval(condition)?;

        if !self.is_truthy(&condition_value) {
            let error_message = if let Some(msg) = message {
                match self.eval(msg)? {
                    Value::String(s) => s,
                    other => format!("Assertion failed with message: {}", other),
                }
            } else {
                "Assertion failed".to_string()
            };

            // RES-028: if the condition is a comparison, show both
            // operand values so "fuel >= 0" doesn't just say "false" —
            // it says "fuel = -5, 0 >= 0 — got: -5 >= 0 == false".
            let detail = self.format_assert_detail(condition, &condition_value);

            return Err(format!(
                "ASSERTION ERROR: {}\n  - {}",
                error_message, detail
            ));
        }

        Ok(Value::Void)
    }

    /// Produce the "why did this assertion fail" line. For infix
    /// comparisons we re-evaluate the operands to show their values;
    /// for anything else we just show the final value.
    fn format_assert_detail(&mut self, condition: &Node, final_value: &Value) -> String {
        if let Node::InfixExpression { left, operator, right } = condition
            && matches!(operator.as_str(), "==" | "!=" | "<" | ">" | "<=" | ">=")
            && let (Ok(lv), Ok(rv)) = (self.eval(left), self.eval(right))
        {
            return format!(
                "condition {} {} {} was {}",
                lv, operator, rv, final_value
            );
        }
        format!("Condition evaluated to: {}", final_value)
    }
    
    fn eval_prefix_expression(&mut self, operator: &str, right: Value) -> RResult<Value> {
        match operator {
            "!" => self.eval_bang_operator_expression(right),
            "-" => self.eval_minus_prefix_operator_expression(right),
            _ => Err(format!("Unknown operator: {}{}", operator, right)),
        }
    }
    
    fn eval_bang_operator_expression(&mut self, right: Value) -> RResult<Value> {
        match right {
            Value::Bool(b) => Ok(Value::Bool(!b)),
            Value::Int(0) => Ok(Value::Bool(true)),
            Value::Int(_) => Ok(Value::Bool(false)),
            Value::Float(0.0) => Ok(Value::Bool(true)),
            Value::Float(_) => Ok(Value::Bool(false)),
            Value::String(s) if s.is_empty() => Ok(Value::Bool(true)),
            Value::String(_) => Ok(Value::Bool(false)),
            _ => Ok(Value::Bool(false)),
        }
    }
    
    fn eval_minus_prefix_operator_expression(&mut self, right: Value) -> RResult<Value> {
        match right {
            Value::Int(i) => Ok(Value::Int(-i)),
            Value::Float(f) => Ok(Value::Float(-f)),
            _ => Err(format!("Unknown operator: -{}", right)),
        }
    }
    
    fn eval_infix_expression(&mut self, operator: &str, left: Value, right: Value) -> RResult<Value> {
        // String + <primitive> coercion (RES-008): when `+` has a string on
        // either side and the other side is a primitive (int / float / bool),
        // coerce the primitive to its textual form and concatenate. This only
        // applies to `+` — other operators keep their strict behavior.
        if operator == "+"
            && (matches!(left, Value::String(_)) || matches!(right, Value::String(_)))
            && let (Some(ls), Some(rs)) = (
                stringify_for_concat(&left),
                stringify_for_concat(&right),
            )
        {
            return Ok(Value::String(format!("{ls}{rs}")));
        }

        // Array concat: `[1,2] + [3]` → `[1,2,3]`. Only for `+`.
        if operator == "+"
            && let (Value::Array(mut l), Value::Array(r)) = (left.clone(), right.clone())
        {
            l.extend(r);
            return Ok(Value::Array(l));
        }

        match (left.clone(), right.clone()) {
            (Value::Int(l), Value::Int(r)) => self.eval_integer_infix_expression(operator, l, r),
            (Value::Float(l), Value::Float(r)) => self.eval_float_infix_expression(operator, l, r),
            (Value::Int(l), Value::Float(r)) => self.eval_float_infix_expression(operator, l as f64, r),
            (Value::Float(l), Value::Int(r)) => self.eval_float_infix_expression(operator, l, r as f64),
            (Value::String(l), Value::String(r)) => self.eval_string_infix_expression(operator, l, r),
            (Value::Bool(l), Value::Bool(r)) => self.eval_boolean_infix_expression(operator, l, r),
            _ => Err(format!("Type mismatch: {} {} {}", left, operator, right)),
        }
    }
    
    fn eval_integer_infix_expression(&mut self, operator: &str, left: i64, right: i64) -> RResult<Value> {
        match operator {
            "+" => Ok(Value::Int(left + right)),
            "-" => Ok(Value::Int(left - right)),
            "*" => Ok(Value::Int(left * right)),
            "/" => {
                if right == 0 {
                    Err("Division by zero".to_string())
                } else {
                    Ok(Value::Int(left / right))
                }
            },
            "%" => {
                if right == 0 {
                    Err("Modulo by zero".to_string())
                } else {
                    Ok(Value::Int(left % right))
                }
            },
            "&" => Ok(Value::Int(left & right)),
            "|" => Ok(Value::Int(left | right)),
            "^" => Ok(Value::Int(left ^ right)),
            "<<" => {
                if !(0..64).contains(&right) {
                    Err(format!("shift amount out of range: {}", right))
                } else {
                    Ok(Value::Int(left << right))
                }
            },
            ">>" => {
                if !(0..64).contains(&right) {
                    Err(format!("shift amount out of range: {}", right))
                } else {
                    Ok(Value::Int(left >> right))
                }
            },
            "==" => Ok(Value::Bool(left == right)),
            "!=" => Ok(Value::Bool(left != right)),
            "<" => Ok(Value::Bool(left < right)),
            ">" => Ok(Value::Bool(left > right)),
            "<=" => Ok(Value::Bool(left <= right)),
            ">=" => Ok(Value::Bool(left >= right)),
            _ => Err(format!("Unknown operator: {} {} {}", left, operator, right)),
        }
    }
    
    fn eval_float_infix_expression(&mut self, operator: &str, left: f64, right: f64) -> RResult<Value> {
        match operator {
            "+" => Ok(Value::Float(left + right)),
            "-" => Ok(Value::Float(left - right)),
            "*" => Ok(Value::Float(left * right)),
            "/" => {
                if right == 0.0 {
                    Err("Division by zero".to_string())
                } else {
                    Ok(Value::Float(left / right))
                }
            },
            "%" => {
                if right == 0.0 {
                    Err("Modulo by zero".to_string())
                } else {
                    Ok(Value::Float(left % right))
                }
            },
            "==" => Ok(Value::Bool(left == right)),
            "!=" => Ok(Value::Bool(left != right)),
            "<" => Ok(Value::Bool(left < right)),
            ">" => Ok(Value::Bool(left > right)),
            "<=" => Ok(Value::Bool(left <= right)),
            ">=" => Ok(Value::Bool(left >= right)),
            _ => Err(format!("Unknown operator: {} {} {}", left, operator, right)),
        }
    }
    
    fn eval_string_infix_expression(&mut self, operator: &str, left: String, right: String) -> RResult<Value> {
        // Lexicographic comparison for <, >, <=, >= matches the standard
        // behavior users expect from strings in most languages.
        match operator {
            "+" => Ok(Value::String(format!("{}{}", left, right))),
            "==" => Ok(Value::Bool(left == right)),
            "!=" => Ok(Value::Bool(left != right)),
            "<" => Ok(Value::Bool(left < right)),
            ">" => Ok(Value::Bool(left > right)),
            "<=" => Ok(Value::Bool(left <= right)),
            ">=" => Ok(Value::Bool(left >= right)),
            _ => Err(format!("Unknown operator: {} {} {}", left, operator, right)),
        }
    }
    
    fn eval_boolean_infix_expression(&mut self, operator: &str, left: bool, right: bool) -> RResult<Value> {
        match operator {
            "==" => Ok(Value::Bool(left == right)),
            "!=" => Ok(Value::Bool(left != right)),
            "&&" => Ok(Value::Bool(left && right)),
            "||" => Ok(Value::Bool(left || right)),
            _ => Err(format!("Unknown operator: {} {} {}", left, operator, right)),
        }
    }
    
    fn eval_expressions(&mut self, expressions: &[Node]) -> RResult<Vec<Value>> {
        let mut result = Vec::new();
        
        for expr in expressions {
            let value = self.eval(expr)?;
            result.push(value);
        }
        
        Ok(result)
    }
    
    fn apply_function(&mut self, func: Value, args: Vec<Value>) -> RResult<Value> {
        match func {
            Value::Function { parameters, body, env, requires, ensures, name } => {
                let mut extended_env = Environment::new_enclosed(env);

                for (i, (_, param_name)) in parameters.iter().enumerate() {
                    if i < args.len() {
                        extended_env.set(param_name.clone(), args[i].clone());
                    }
                }

                let mut interpreter = Interpreter {
                    env: extended_env,
                    statics: self.statics.clone(),
                };

                // RES-035: check each `requires` clause BEFORE running
                // the body. Parameters are already in scope; anything
                // else (e.g. `static` bindings, closed-over vars) is
                // reachable just like inside the body.
                for clause in &requires {
                    let v = interpreter.eval(clause)?;
                    if !interpreter.is_truthy(&v) {
                        return Err(format!(
                            "Contract violation in fn {}: requires {} failed",
                            name,
                            format_contract_expr(clause)
                        ));
                    }
                }

                let body_result = interpreter.eval(&body)?;
                let return_value = if let Value::Return(v) = body_result {
                    *v
                } else {
                    body_result
                };

                // RES-035: check each `ensures` clause AFTER, with the
                // special identifier `result` bound to the return value.
                if !ensures.is_empty() {
                    interpreter
                        .env
                        .set("result".to_string(), return_value.clone());
                    for clause in &ensures {
                        let v = interpreter.eval(clause)?;
                        if !interpreter.is_truthy(&v) {
                            return Err(format!(
                                "Contract violation in fn {}: ensures {} failed (result = {})",
                                name,
                                format_contract_expr(clause),
                                return_value
                            ));
                        }
                    }
                }

                Ok(return_value)
            }
            Value::Builtin { func, .. } => func(&args),
            _ => Err(format!("Not a function: {}", func)),
        }
    }
    
    /// RES-039: test a pattern against a value. On match, returns
    /// `Some(binding)` where binding is `Some((name, value))` for an
    /// identifier pattern or `None` otherwise. On no match, returns `None`.
    #[allow(clippy::type_complexity)]
    fn match_pattern(
        &mut self,
        pattern: &Pattern,
        value: &Value,
    ) -> RResult<Option<Option<(String, Value)>>> {
        match pattern {
            Pattern::Wildcard => Ok(Some(None)),
            Pattern::Identifier(name) => Ok(Some(Some((name.clone(), value.clone())))),
            Pattern::Literal(node) => {
                let pat_val = self.eval(node)?;
                let is_equal = match (&pat_val, value) {
                    (Value::Int(a), Value::Int(b)) => a == b,
                    (Value::Float(a), Value::Float(b)) => a == b,
                    (Value::Int(a), Value::Float(b)) => (*a as f64) == *b,
                    (Value::Float(a), Value::Int(b)) => *a == (*b as f64),
                    (Value::String(a), Value::String(b)) => a == b,
                    (Value::Bool(a), Value::Bool(b)) => a == b,
                    _ => false,
                };
                Ok(if is_equal { Some(None) } else { None })
            }
        }
    }

    fn is_truthy(&self, value: &Value) -> bool {
        match value {
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::Float(f) => *f != 0.0,
            Value::String(s) => !s.is_empty(),
            _ => true,
        }
    }
}

// REPL for interactive evaluation.
// Kept as a reference implementation; the actual REPL used is `repl::EnhancedREPL`.
#[allow(dead_code)]
fn start_repl() -> RustylineResult<()> {
    let mut interpreter = Interpreter::new();
    let mut rl = DefaultEditor::new()?;
    let mut type_check_enabled = false;
    
    // Load history if available
    let history_path = match env::var("HOME") {
        Ok(home) => Path::new(&home).join(".resilient_history"),
        Err(_) => Path::new(".resilient_history").to_path_buf(),
    };
    
    if history_path.exists()
        && let Err(err) = rl.load_history(&history_path)
    {
        eprintln!("Error loading history: {}", err);
    }

    println!("Resilient Programming Language REPL (v0.1.0)");
    println!("Type 'exit' to quit, 'help' for command list");
    
    loop {
        let prompt = if type_check_enabled {
            ">> [typecheck] "
        } else {
            ">> "
        };
        
        let readline = rl.readline(prompt);
        
        match readline {
            Ok(line) => {
                let input = line.trim();
                
                // Skip empty lines
                if input.is_empty() {
                    continue;
                }
                
                // Add to history
                rl.add_history_entry(input)?;
                
                // Handle special commands
                match input {
                    "exit" | "quit" => break,
                    "help" => {
                        println!("Available commands:");
                        println!("  help       - Show this help message");
                        println!("  exit       - Exit the REPL");
                        println!("  clear      - Clear the screen");
                        println!("  examples   - Show example code snippets");
                        println!("  typecheck  - Toggle type checking (currently {})", 
                                 if type_check_enabled { "enabled" } else { "disabled" });
                        continue;
                    },
                    "clear" => {
                        print!("\x1B[2J\x1B[1;1H"); // ANSI escape code to clear screen
                        io::stdout().flush().unwrap();
                        continue;
                    },
                    "typecheck" => {
                        type_check_enabled = !type_check_enabled;
                        println!("Type checking {}", 
                                 if type_check_enabled { "enabled" } else { "disabled" });
                        continue;
                    },
                    "examples" => {
                        println!("Example code snippets:");
                        println!("\n1. Basic variable and function:");
                        println!("let x = 42;");
                        println!("fn add(int a, int b) {{ return a + b; }}");
                        println!("add(x, 10);");
                        
                        println!("\n2. Live block example:");
                        println!("live {{");
                        println!("  let result = 100 / 0; // This would normally crash");
                        println!("  println(\"Result: \" + result);");
                        println!("}}");
                        
                        println!("\n3. Assertion example:");
                        println!("let age = 25;");
                        println!("assert(age >= 18, \"Must be an adult\");");
                        println!("println(\"Access granted\");");
                        continue;
                    },
                    _ => {}
                }
                
                // Parse the input
                let lexer = Lexer::new(input.to_string());
                let mut parser = Parser::new(lexer);
                let program = parser.parse_program();
                
                // Skip evaluation if any parser errors were recorded
                if !parser.errors.is_empty() {
                    continue;
                }
                
                // Run type checker if enabled
                if type_check_enabled {
                    match typechecker::TypeChecker::new().check_program(&program) {
                        Ok(_) => println!("\x1B[32mType check passed\x1B[0m"), // Green text
                        Err(e) => {
                            eprintln!("\x1B[31mType error: {}\x1B[0m", e); // Red text
                            continue; // Skip execution if type checking fails
                        }
                    }
                }
                
                // Evaluate the input
                match interpreter.eval(&program) {
                    Ok(value) => {
                        if !matches!(value, Value::Void) {
                            println!("{}", value);
                        }
                    },
                    Err(error) => {
                        eprintln!("\x1B[31mError: {}\x1B[0m", error); // Red error text
                    }
                }
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
    if let Err(err) = rl.save_history(&history_path) {
        eprintln!("Error saving history: {}", err);
    }
    
    Ok(())
}

// Execute a Resilient source file
fn execute_file(filename: &str, type_check: bool) -> RResult<()> {
    let contents = fs::read_to_string(filename)
        .map_err(|e| format!("Error reading file: {}", e))?;
    
    let lexer = Lexer::new(contents);
    let mut parser = Parser::new(lexer);
    let program = parser.parse_program();
    
    // Check for parser errors (already printed at the point they occurred)
    if !parser.errors.is_empty() {
        return Err(format!("Failed to parse program: {} parser error(s)", parser.errors.len()));
    }
    
    // Type checking if enabled
    if type_check {
        println!("Running type checker...");
        match typechecker::TypeChecker::new().check_program(&program) {
            Ok(_) => println!("\x1B[32mType check passed\x1B[0m"), // Green text
            Err(e) => {
                eprintln!("\x1B[31mType error: {}\x1B[0m", e); // Red text
                return Err(format!("Type check failed: {}", e));
            }
        }
    }
    
    let mut interpreter = Interpreter::new();
    interpreter.eval(&program)?;
    
    Ok(())
}

// Example programs
fn create_examples_directory() -> std::io::Result<()> {
    let examples_dir = Path::new("examples");
    
    if !examples_dir.exists() {
        fs::create_dir(examples_dir)?;
    }
    
    // Create an example file demonstrating live blocks and assertions
    let example_content = r#"
// Simple example of Resilient's key features
// This simulates a sensor reading system with error handling

// Simulates reading from a sensor, can fail with a negative value
fn read_sensor() {
    // Return a random value for testing
    // In a real system, this would read from hardware
    if read_random() < 0.2 {
        return -1; // Simulated failure
    }
    
    return read_random() * 100;
}

// Helper function to generate random values
fn read_random() {
    // For the MVP, just return 0.5
    // In a real implementation, this would use a RNG
    return 0.5;
}

// Function to check if sensor reading is valid
fn is_valid_reading(reading) {
    return reading >= 0;
}

// Main control loop
fn main_loop() {
    let threshold = 50;
    
    // System invariant - this must never be violated
    assert(threshold > 0, "Threshold must be positive");
    
    // The live block will handle recoverable errors
    live {
        let sensor_value = read_sensor();
        
        // Validate the reading
        assert(is_valid_reading(sensor_value), "Invalid sensor reading");
        
        // Process the valid reading
        if sensor_value > threshold {
            println("Warning: High sensor value: " + sensor_value);
        } else {
            println("Sensor value normal: " + sensor_value);
        }
    }
}

// Start the main loop
main_loop();
"#;
    
    fs::write(examples_dir.join("sensor_example.rs"), example_content)?;
    
    // Create another example demonstrating the self-healing property
    let healing_example = r#"
// Example demonstrating Resilient's self-healing capability

// Simulates an unreliable operation that might fail
fn unreliable_operation() {
    // Simulates a failure 50% of the time
    if read_random() < 0.5 {
        return -1; // Error condition
    }
    
    return 42; // Success value
}

// Helper function to generate random values
fn read_random() {
    // For the MVP, alternates between 0.25 and 0.75
    // In a real implementation, this would use a RNG
    static let toggle = false;
    toggle = !toggle;
    
    if toggle {
        return 0.25;
    } else {
        return 0.75;
    }
}

fn main() {
    let max_attempts = 5;
    let current_attempt = 0;
    
    // This will retry until it succeeds or reaches max attempts
    live {
        current_attempt = current_attempt + 1;
        println("Attempt " + current_attempt + " of " + max_attempts);
        
        let result = unreliable_operation();
        
        // If result is negative, this will cause the live block to retry
        assert(result >= 0, "Operation failed, retrying...");
        
        // If we get here, the operation succeeded
        println("Operation succeeded with result: " + result);
        
        // Break out of retry loop
        if current_attempt >= max_attempts {
            println("Reached maximum retry attempts");
            return;
        }
    }
}

main();
"#;
    
    fs::write(examples_dir.join("self_healing.rs"), healing_example)?;
    
    Ok(())
}

fn main() {
    // Get command line arguments
    let args: Vec<String> = env::args().collect();
    
    // Create examples directory
    if let Err(e) = create_examples_directory() {
        eprintln!("Error creating examples: {}", e);
    }
    
    let mut type_check = false;
    let mut filename = "";
    
    // Simple argument parsing
    if args.len() > 1 {
        for arg in args.iter().skip(1) {
            if arg == "--typecheck" || arg == "-t" {
                type_check = true;
            } else {
                filename = arg;
            }
        }
        
        if !filename.is_empty() {
            // Execute a file. RES-027: a failed run exits non-zero so
            // `run_examples.sh` / CI / ops tooling can distinguish
            // success from failure without parsing stdout.
            match execute_file(filename, type_check) {
                Ok(_) => {
                    println!("Program executed successfully");
                    return;
                }
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
        }
    }

    // Start the enhanced REPL if no file was provided
    let mut enhanced_repl = repl::EnhancedREPL::new();
    if let Err(e) = enhanced_repl.run() {
        eprintln!("REPL error: {}", e);
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lex the entire input into a Vec<Token>, stopping at (and including) Eof.
    fn tokenize(input: &str) -> Vec<Token> {
        let mut lexer = Lexer::new(input.to_string());
        let mut out = Vec::new();
        loop {
            let tok = lexer.next_token();
            let is_eof = matches!(tok, Token::Eof);
            out.push(tok);
            if is_eof {
                break;
            }
        }
        out
    }

    // ---------- Lexer ----------

    #[test]
    fn lexer_handles_identifier_adjacent_to_paren() {
        // Regression for RES-001: the old lexer swallowed the character
        // after every identifier, so `fn add_one(` lost the `(`.
        let tokens = tokenize("fn add_one(int x) {}");
        assert_eq!(
            tokens,
            vec![
                Token::Function,
                Token::Identifier("add_one".into()),
                Token::LeftParen,
                Token::Identifier("int".into()),
                Token::Identifier("x".into()),
                Token::RightParen,
                Token::LeftBrace,
                Token::RightBrace,
                Token::Eof,
            ]
        );
    }

    #[test]
    fn lexer_distinguishes_int_and_float() {
        let tokens = tokenize("let x = 42; let y = 3.14;");
        // Grab the literals in order.
        let literals: Vec<_> = tokens
            .into_iter()
            .filter(|t| matches!(t, Token::IntLiteral(_) | Token::FloatLiteral(_)))
            .collect();
        assert_eq!(
            literals,
            vec![Token::IntLiteral(42), Token::FloatLiteral(3.14)]
        );
    }

    #[test]
    fn lexer_recognizes_keywords_and_operators() {
        let tokens = tokenize("if true { return; } else { assert(x == 1); }");
        assert!(tokens.contains(&Token::If));
        assert!(tokens.contains(&Token::Else));
        assert!(tokens.contains(&Token::Return));
        assert!(tokens.contains(&Token::Assert));
        assert!(tokens.contains(&Token::BoolLiteral(true)));
        assert!(tokens.contains(&Token::Equal));
    }

    #[test]
    fn lexer_parses_string_literals_with_escapes() {
        let tokens = tokenize(r#"let s = "hi\n";"#);
        let has_string = tokens
            .iter()
            .any(|t| matches!(t, Token::StringLiteral(s) if s == "hi\n"));
        assert!(has_string, "expected StringLiteral(\"hi\\n\") in {:?}", tokens);
    }

    // ---------- Parser ----------

    fn parse(input: &str) -> (Node, Vec<String>) {
        let lexer = Lexer::new(input.to_string());
        let mut parser = Parser::new(lexer);
        let program = parser.parse_program();
        (program, parser.errors)
    }

    #[test]
    fn parser_let_statement_produces_expected_shape() {
        let (program, errors) = parse("let x = 42;");
        assert!(errors.is_empty(), "unexpected errors: {:?}", errors);
        match program {
            Node::Program(stmts) => {
                assert_eq!(stmts.len(), 1);
                match &stmts[0] {
                    Node::LetStatement { name, value, .. } => {
                        assert_eq!(name, "x");
                        assert!(matches!(**value, Node::IntegerLiteral(42)));
                    }
                    other => panic!("expected LetStatement, got {:?}", other),
                }
            }
            other => panic!("expected Program, got {:?}", other),
        }
    }

    #[test]
    fn parser_function_with_no_parameters() {
        // RES-004: `fn main()` must parse. Historically the parser
        // appeared to reject it, but that was the RES-001 lexer bug
        // eating the `(`. The parameter-list parser itself already
        // handled empty `()`; this test locks that in.
        let (program, errors) = parse("fn main() { let x = 1; }");
        assert!(errors.is_empty(), "unexpected errors: {:?}", errors);
        match program {
            Node::Program(stmts) => match &stmts[0] {
                Node::Function { name, parameters, .. } => {
                    assert_eq!(name, "main");
                    assert!(parameters.is_empty(), "expected no params, got {:?}", parameters);
                }
                other => panic!("expected Function, got {:?}", other),
            },
            other => panic!("expected Program, got {:?}", other),
        }
    }

    #[test]
    fn parser_function_with_parameters_roundtrips() {
        let (program, errors) = parse("fn add(int a, int b) { return a + b; }");
        assert!(errors.is_empty(), "unexpected errors: {:?}", errors);
        match program {
            Node::Program(stmts) => match &stmts[0] {
                Node::Function { name, parameters, .. } => {
                    assert_eq!(name, "add");
                    assert_eq!(
                        parameters,
                        &vec![
                            ("int".to_string(), "a".to_string()),
                            ("int".to_string(), "b".to_string())
                        ]
                    );
                }
                other => panic!("expected Function, got {:?}", other),
            },
            other => panic!("expected Program, got {:?}", other),
        }
    }

    // ---------- Typechecker ----------

    #[test]
    fn typechecker_accepts_valid_program() {
        let (program, errors) = parse("let x = 42; let y = x + 1;");
        assert!(errors.is_empty(), "parse errors: {:?}", errors);
        let mut tc = typechecker::TypeChecker::new();
        assert!(tc.check_program(&program).is_ok());
    }

    // ---------- Interpreter ----------

    #[test]
    fn interpreter_has_println_registered() {
        // RES-003 contract: every fresh Interpreter has `println` callable.
        let interp = Interpreter::new();
        match interp.env.get("println") {
            Some(Value::Builtin { name, .. }) => assert_eq!(name, "println"),
            other => panic!("expected Builtin(println), got {:?}", other),
        }
    }

    #[test]
    fn builtin_println_rejects_too_many_args() {
        let err = builtin_println(&[Value::Int(1), Value::Int(2)]).unwrap_err();
        assert!(err.contains("expects 0 or 1"), "err was: {}", err);
    }

    #[test]
    fn string_plus_int_coerces() {
        // RES-008: `"x=" + 42` → `"x=42"`
        let (program, errors) = parse(r#"let s = "x=" + 42;"#);
        assert!(errors.is_empty(), "errors: {:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&program).unwrap();
        match interp.env.get("s").unwrap() {
            Value::String(s) => assert_eq!(s, "x=42"),
            other => panic!("expected String, got {:?}", other),
        }
    }

    #[test]
    fn int_plus_string_coerces() {
        let (program, _errors) = parse(r#"let s = 1 + "x";"#);
        let mut interp = Interpreter::new();
        interp.eval(&program).unwrap();
        match interp.env.get("s").unwrap() {
            Value::String(s) => assert_eq!(s, "1x"),
            other => panic!("expected String, got {:?}", other),
        }
    }

    #[test]
    fn string_plus_bool_coerces() {
        let (program, _errors) = parse(r#"let s = "on=" + true;"#);
        let mut interp = Interpreter::new();
        interp.eval(&program).unwrap();
        match interp.env.get("s").unwrap() {
            Value::String(s) => assert_eq!(s, "on=true"),
            other => panic!("expected String, got {:?}", other),
        }
    }

    #[test]
    fn int_plus_int_still_arithmetic() {
        // Regression: make sure coercion didn't hijack pure-int `+`.
        let (program, _errors) = parse("let n = 1 + 2;");
        let mut interp = Interpreter::new();
        interp.eval(&program).unwrap();
        match interp.env.get("n").unwrap() {
            Value::Int(n) => assert_eq!(n, 3),
            other => panic!("expected Int(3), got {:?}", other),
        }
    }

    #[test]
    fn lexer_dot_token_and_float_literal_coexist() {
        // RES-010 used to assert Token::Unknown('.'), but RES-038
        // promotes `.` to a real token for field access. Numeric
        // literals with decimals (1.5) still lex correctly because
        // read_number consumes the `.` before the outer dispatcher
        // gets a chance.
        let tokens = tokenize(". 1.5");
        assert_eq!(tokens[0], Token::Dot);
        assert!(
            tokens.iter().any(|t| matches!(t, Token::FloatLiteral(f) if *f == 1.5)),
            "expected FloatLiteral(1.5) to follow, got {:?}",
            tokens
        );
    }

    // ---------- Array builtins (RES-033) ----------

    #[test]
    fn push_returns_new_array() {
        let (p, _e) = parse("let a = [1, 2]; let b = push(a, 3);");
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        match interp.env.get("b").unwrap() {
            Value::Array(v) => {
                assert_eq!(v.len(), 3);
                assert!(matches!(v[2], Value::Int(3)));
            }
            other => panic!("{:?}", other),
        }
        // original untouched
        match interp.env.get("a").unwrap() {
            Value::Array(v) => assert_eq!(v.len(), 2),
            _ => panic!("expected Array"),
        }
    }

    #[test]
    fn pop_returns_shorter_array() {
        let (p, _e) = parse("let a = [1, 2, 3]; let b = pop(a);");
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        match interp.env.get("b").unwrap() {
            Value::Array(v) => assert_eq!(v.len(), 2),
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn pop_empty_errors() {
        let (p, _e) = parse("let a = []; let b = pop(a);");
        let mut interp = Interpreter::new();
        let err = interp.eval(&p).unwrap_err();
        assert!(err.contains("empty"), "{}", err);
    }

    #[test]
    fn slice_returns_subrange() {
        let (p, _e) = parse("let a = [10, 20, 30, 40]; let b = slice(a, 1, 3);");
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        match interp.env.get("b").unwrap() {
            Value::Array(v) => {
                assert_eq!(v.len(), 2);
                assert!(matches!(v[0], Value::Int(20)));
                assert!(matches!(v[1], Value::Int(30)));
            }
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn slice_out_of_range_errors() {
        let (p, _e) = parse("let a = [1]; let b = slice(a, 0, 5);");
        let mut interp = Interpreter::new();
        let err = interp.eval(&p).unwrap_err();
        assert!(err.contains("invalid"), "{}", err);
    }

    // ---------- Static contract discharge (RES-060) ----------

    #[test]
    fn contract_tautology_passes_typecheck() {
        // `5 != 0` is provably true — the typechecker folds it.
        typecheck_src(r#"
            fn f() requires 5 != 0 { return 1; }
        "#).unwrap();
    }

    #[test]
    fn contract_contradiction_rejected_at_compile_time() {
        let err = typecheck_src(r#"
            fn f() requires 0 != 0 { return 1; }
        "#).unwrap_err();
        assert!(
            err.contains("contract can never hold"),
            "unexpected: {}",
            err
        );
    }

    #[test]
    fn contract_literal_false_rejected() {
        let err = typecheck_src(r#"
            fn f() requires false { return 1; }
        "#).unwrap_err();
        assert!(
            err.contains("contract can never hold"),
            "unexpected: {}",
            err
        );
    }

    #[test]
    fn contract_with_free_variable_not_folded() {
        // `x > 0` can't be proven at compile time; typecheck should
        // accept it and leave the check for runtime.
        typecheck_src(r#"
            fn f(int x) requires x > 0 { return x; }
        "#).unwrap();
    }

    #[test]
    fn contract_complex_arithmetic_folds() {
        // 2 + 3 == 5 → tautology.
        typecheck_src(r#"
            fn f() requires 2 + 3 == 5 { return 1; }
        "#).unwrap();
        // 2 + 3 == 4 → contradiction.
        let err = typecheck_src(r#"
            fn g() requires 2 + 3 == 4 { return 1; }
        "#).unwrap_err();
        assert!(err.contains("never hold"), "unexpected: {}", err);
    }

    // ---------- Typechecker rejection (RES-053) ----------

    fn typecheck_src(src: &str) -> Result<(), String> {
        let (program, errors) = parse(src);
        assert!(errors.is_empty(), "parse errors: {:?}", errors);
        typechecker::TypeChecker::new().check_program(&program).map(|_| ())
    }

    #[test]
    fn typecheck_rejects_let_annot_mismatch() {
        let err = typecheck_src(r#"let x: int = "hi";"#).unwrap_err();
        assert!(
            err.contains("let x: int") && err.contains("string"),
            "unexpected: {}",
            err
        );
    }

    #[test]
    fn typecheck_accepts_matching_let_annot() {
        typecheck_src("let x: int = 42;").unwrap();
        typecheck_src(r#"let s: string = "hi";"#).unwrap();
    }

    #[test]
    fn typecheck_rejects_int_plus_bool() {
        let err = typecheck_src("let x = 1 + true;").unwrap_err();
        assert!(err.contains("Cannot apply"), "unexpected: {}", err);
    }

    #[test]
    fn typecheck_rejects_fn_return_type_mismatch() {
        let err = typecheck_src(r#"fn f() -> int { return "hi"; }"#).unwrap_err();
        assert!(
            err.contains("return type mismatch"),
            "unexpected: {}",
            err
        );
    }

    #[test]
    fn typecheck_accepts_well_typed_fn() {
        typecheck_src("fn add(int a, int b) -> int { return a + b; }").unwrap();
    }

    #[test]
    fn typecheck_rejects_calling_a_non_function() {
        // 42() -> error because 42 has type Int, not a function.
        let err = typecheck_src("let x = 42; let y = x(0);").unwrap_err();
        assert!(
            err.contains("Cannot call non-function"),
            "unexpected: {}",
            err
        );
    }

    #[test]
    fn typecheck_rejects_bitwise_on_float() {
        let err = typecheck_src("let x = 1.5 & 2;").unwrap_err();
        assert!(err.contains("Bitwise"), "unexpected: {}", err);
    }

    #[test]
    fn typecheck_accepts_string_plus_int_coercion() {
        // RES-008 compatibility.
        typecheck_src(r#"let s = "n=" + 42;"#).unwrap();
    }

    #[test]
    fn typecheck_rejects_try_on_non_result() {
        let err = typecheck_src("let x = 42?;").unwrap_err();
        assert!(err.contains("? operator"), "unexpected: {}", err);
    }

    // ---------- Typed declarations (RES-052) ----------

    #[test]
    fn typed_let_parses_and_records_annotation() {
        let (p, errors) = parse("let x: int = 42;");
        assert!(errors.is_empty(), "{:?}", errors);
        match p {
            Node::Program(stmts) => match &stmts[0] {
                Node::LetStatement { name, value, type_annot } => {
                    assert_eq!(name, "x");
                    assert_eq!(type_annot.as_deref(), Some("int"));
                    assert!(matches!(**value, Node::IntegerLiteral(42)));
                }
                other => panic!("expected LetStatement, got {:?}", other),
            },
            _ => panic!("expected Program"),
        }
    }

    #[test]
    fn typed_let_still_executes() {
        let (p, _e) = parse("let x: int = 42; let y = x + 1;");
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("y").unwrap(), Value::Int(43)));
    }

    #[test]
    fn fn_with_return_type_parses() {
        let src = r#"
            fn add(int a, int b) -> int {
                return a + b;
            }
            let r = add(2, 3);
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        // Find the Function node to check its return_type.
        match p {
            Node::Program(stmts) => match &stmts[0] {
                Node::Function { name, return_type, .. } => {
                    assert_eq!(name, "add");
                    assert_eq!(return_type.as_deref(), Some("int"));
                }
                other => panic!("expected Function, got {:?}", other),
            },
            _ => panic!(),
        }
    }

    #[test]
    fn return_type_on_anonymous_fn() {
        let (p, errors) = parse("let f = fn(int x) -> int { return x + 1; };");
        assert!(errors.is_empty(), "{:?}", errors);
        // Execute and confirm behavior is unchanged.
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        // Also confirm it behaves callably.
        let (p2, _e) = parse("let f = fn(int x) -> int { return x + 1; }; let r = f(10);");
        let mut interp2 = Interpreter::new();
        interp2.eval(&p2).unwrap();
        assert!(matches!(interp2.env.get("r").unwrap(), Value::Int(11)));
    }

    // ---------- First-class functions (RES-042) ----------

    #[test]
    fn anonymous_fn_called_inline() {
        let (p, errors) = parse("let add = fn(int a, int b) { return a + b; }; let r = add(2, 3);");
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("r").unwrap(), Value::Int(5)));
    }

    #[test]
    fn closure_captures_enclosing_variable() {
        let src = r#"
            fn make_adder(int n) {
                return fn(int x) { return x + n; };
            }
            let add5 = make_adder(5);
            let r = add5(10);
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("r").unwrap(), Value::Int(15)));
    }

    #[test]
    fn anonymous_fn_can_have_contracts() {
        // The anonymous-fn form inherits the full fn parser, including
        // requires/ensures.
        let src = r#"
            let safe_div = fn(int a, int b)
                requires b != 0
            {
                return a / b;
            };
            let r = safe_div(20, 5);
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("r").unwrap(), Value::Int(4)));
    }

    // ---------- Result type (RES-040) ----------

    #[test]
    fn result_ok_and_err_construct() {
        let (p, _e) = parse(r#"
            let good = Ok(42);
            let bad = Err("boom");
            let g_ok = is_ok(good);
            let b_ok = is_ok(bad);
            let g = unwrap(good);
            let b = unwrap_err(bad);
        "#);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("g_ok").unwrap(), Value::Bool(true)));
        assert!(matches!(interp.env.get("b_ok").unwrap(), Value::Bool(false)));
        assert!(matches!(interp.env.get("g").unwrap(), Value::Int(42)));
        assert!(matches!(interp.env.get("b").unwrap(), Value::String(s) if s == "boom"));
    }

    #[test]
    fn unwrap_on_err_errors() {
        let (p, _e) = parse(r#"let x = unwrap(Err("no"));"#);
        let mut interp = Interpreter::new();
        let err = interp.eval(&p).unwrap_err();
        assert!(err.contains("unwrap called on Err"), "{}", err);
    }

    // ---------- ? propagation (RES-041) ----------

    #[test]
    fn try_operator_propagates_err() {
        let src = r#"
            fn parse_int() { return Err("not a number"); }
            fn double() {
                let n = parse_int()?;
                return Ok(n + n);
            }
            let r = double();
            let was_err = is_err(r);
            let msg = unwrap_err(r);
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("was_err").unwrap(), Value::Bool(true)));
        assert!(matches!(interp.env.get("msg").unwrap(), Value::String(s) if s == "not a number"));
    }

    #[test]
    fn try_operator_extracts_ok() {
        let src = r#"
            fn get() { return Ok(7); }
            fn user() {
                let n = get()?;
                return Ok(n * 3);
            }
            let r = unwrap(user());
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("r").unwrap(), Value::Int(21)));
    }

    #[test]
    fn try_operator_on_non_result_errors() {
        let (p, _e) = parse("let x = 42?;");
        let mut interp = Interpreter::new();
        let err = interp.eval(&p).unwrap_err();
        assert!(err.contains("? operator expects a Result"), "{}", err);
    }

    // ---------- String builtins (RES-043) ----------

    #[test]
    fn string_builtins_basic() {
        let (p, _e) = parse(r#"
            let parts = split("a,b,c", ",");
            let t = trim("   hi   ");
            let hasH = contains("hello", "ell");
            let up = to_upper("Foo");
            let lo = to_lower("BAR");
        "#);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        match interp.env.get("parts").unwrap() {
            Value::Array(v) => {
                assert_eq!(v.len(), 3);
                assert!(matches!(&v[1], Value::String(s) if s == "b"));
            }
            _ => panic!("expected Array"),
        }
        assert!(matches!(interp.env.get("t").unwrap(), Value::String(s) if s == "hi"));
        assert!(matches!(interp.env.get("hasH").unwrap(), Value::Bool(true)));
        assert!(matches!(interp.env.get("up").unwrap(), Value::String(s) if s == "FOO"));
        assert!(matches!(interp.env.get("lo").unwrap(), Value::String(s) if s == "bar"));
    }

    #[test]
    fn split_empty_sep_per_char() {
        let (p, _e) = parse(r#"let cs = split("abc", "");"#);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        match interp.env.get("cs").unwrap() {
            Value::Array(v) => assert_eq!(v.len(), 3),
            _ => panic!("expected Array"),
        }
    }

    // ---------- Match exhaustiveness (RES-054) ----------

    #[test]
    fn typecheck_rejects_non_exhaustive_bool_match() {
        let err = typecheck_src(r#"
            let b = true;
            let r = match b { true => 1, };
        "#).unwrap_err();
        assert!(err.contains("Non-exhaustive match on bool"), "got: {}", err);
        assert!(err.contains("missing `false`"), "got: {}", err);
    }

    #[test]
    fn typecheck_accepts_exhaustive_bool_match() {
        typecheck_src(r#"
            let b = true;
            let r = match b { true => 1, false => 0, };
        "#).unwrap();
    }

    #[test]
    fn typecheck_accepts_bool_match_with_wildcard() {
        typecheck_src(r#"
            let b = true;
            let r = match b { true => 1, _ => 0, };
        "#).unwrap();
    }

    #[test]
    fn typecheck_rejects_int_match_without_default() {
        let err = typecheck_src(r#"
            let n = 5;
            let r = match n { 0 => "zero", 1 => "one", };
        "#).unwrap_err();
        assert!(err.contains("Non-exhaustive match on int"), "got: {}", err);
    }

    #[test]
    fn typecheck_accepts_int_match_with_identifier_default() {
        typecheck_src(r#"
            let n = 5;
            let r = match n { 0 => "zero", x => "other", };
        "#).unwrap();
    }

    // ---------- match (RES-039) ----------

    #[test]
    fn match_literal_arm() {
        let src = r#"
            let r = match 2 {
                0 => "zero",
                1 => "one",
                2 => "two",
                n => "other",
            };
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        match interp.env.get("r").unwrap() {
            Value::String(s) => assert_eq!(s, "two"),
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn match_identifier_binding() {
        let src = r#"
            let r = match 42 {
                0 => -1,
                n => n * 2,
            };
        "#;
        let (p, _e) = parse(src);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("r").unwrap(), Value::Int(84)));
    }

    #[test]
    fn match_wildcard_falls_through() {
        let src = r#"
            let r = match "nope" {
                "yes" => 1,
                _ => 0,
            };
        "#;
        let (p, _e) = parse(src);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("r").unwrap(), Value::Int(0)));
    }

    #[test]
    fn match_no_arm_matches_returns_void() {
        let src = r#"
            let r = match 5 {
                0 => 1,
                1 => 2,
            };
        "#;
        let (p, _e) = parse(src);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("r").unwrap(), Value::Void));
    }

    #[test]
    fn match_binding_does_not_leak() {
        // Identifier pattern binding must not escape the match.
        let src = r#"
            let m = match 1 { n => n + 1, };
            let outer = 99;
        "#;
        let (p, _e) = parse(src);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        // `n` should NOT be visible outside the match arm.
        assert!(interp.env.get("n").is_none());
        assert!(matches!(interp.env.get("m").unwrap(), Value::Int(2)));
    }

    // ---------- Structs (RES-038) ----------

    #[test]
    fn struct_decl_literal_and_access() {
        let src = r#"
            struct Point { int x, int y, }
            let p = new Point { x: 3, y: 4 };
            let dx = p.x;
            let dy = p.y;
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "parse errors: {:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("dx").unwrap(), Value::Int(3)));
        assert!(matches!(interp.env.get("dy").unwrap(), Value::Int(4)));
    }

    #[test]
    fn struct_field_assignment() {
        let src = r#"
            struct Point { int x, int y, }
            let p = new Point { x: 0, y: 0 };
            p.x = 7;
            let got = p.x;
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("got").unwrap(), Value::Int(7)));
    }

    #[test]
    fn struct_nested_field_assignment() {
        let src = r#"
            struct Inner { int v, }
            struct Outer { int tag, int v, }
            let o = new Outer { tag: 1, v: 0 };
            o.v = 99;
            let got = o.v;
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("got").unwrap(), Value::Int(99)));
    }

    #[test]
    fn struct_unknown_field_errors() {
        let src = r#"
            struct Point { int x, int y, }
            let p = new Point { x: 1, y: 2 };
            let z = p.z;
        "#;
        let (p, _e) = parse(src);
        let mut interp = Interpreter::new();
        let err = interp.eval(&p).unwrap_err();
        assert!(err.contains("no field 'z'"), "err: {}", err);
    }

    #[test]
    fn struct_empty() {
        let src = r#"
            struct Empty {}
            let e = new Empty {};
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        match interp.env.get("e").unwrap() {
            Value::Struct { name, fields } => {
                assert_eq!(name, "Empty");
                assert!(fields.is_empty());
            }
            other => panic!("{:?}", other),
        }
    }

    // ---------- Live-block invariants (RES-036) ----------

    #[test]
    fn live_block_with_passing_invariant() {
        let src = r#"
            let fuel = 100;
            live invariant fuel >= 0 {
                fuel = fuel - 10;
            }
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("fuel").unwrap(), Value::Int(90)));
    }

    #[test]
    fn live_block_invariant_violation_retries_then_fails() {
        // This body ALWAYS leaves fuel negative. After three retries
        // the block gives up with an invariant-violation error.
        let src = r#"
            let fuel = 5;
            live invariant fuel >= 0 {
                fuel = fuel - 100;
            }
        "#;
        let (p, _e) = parse(src);
        let mut interp = Interpreter::new();
        let err = interp.eval(&p).unwrap_err();
        assert!(
            err.contains("Invariant violation") && err.contains("fuel >= 0"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn live_block_multiple_invariants() {
        let src = r#"
            let x = 5;
            let y = 10;
            live
                invariant x >= 0
                invariant y > x
            {
                x = x + 1;
                y = y + 1;
            }
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("x").unwrap(), Value::Int(6)));
        assert!(matches!(interp.env.get("y").unwrap(), Value::Int(11)));
    }

    // ---------- for..in (RES-037) ----------

    #[test]
    fn for_in_sums_array() {
        let src = r#"
            let xs = [1, 2, 3, 4, 5];
            let s = 0;
            for x in xs { s = s + x; }
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("s").unwrap(), Value::Int(15)));
    }

    #[test]
    fn for_in_empty_array_is_noop() {
        let (p, _e) = parse("let s = 99; for x in [] { s = 0; }");
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("s").unwrap(), Value::Int(99)));
    }

    #[test]
    fn for_in_nested_arrays() {
        let src = r#"
            let m = [[1, 2], [3, 4]];
            let s = 0;
            for row in m { for v in row { s = s + v; } }
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("s").unwrap(), Value::Int(10)));
    }

    #[test]
    fn for_in_non_array_errors() {
        let (p, _e) = parse("for x in 42 { let y = 1; }");
        let mut interp = Interpreter::new();
        let err = interp.eval(&p).unwrap_err();
        assert!(err.contains("iterable must be an array"), "{}", err);
    }

    // ---------- Function contracts (RES-035) ----------

    #[test]
    fn contract_requires_valid_passes() {
        let src = r#"
            fn divide(int a, int b)
                requires b != 0
            {
                return a / b;
            }
            let x = divide(10, 2);
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("x").unwrap(), Value::Int(5)));
    }

    #[test]
    fn contract_requires_violation_errors() {
        let src = r#"
            fn divide(int a, int b)
                requires b != 0
            {
                return a / b;
            }
            let x = divide(10, 0);
        "#;
        let (p, _e) = parse(src);
        let mut interp = Interpreter::new();
        let err = interp.eval(&p).unwrap_err();
        assert!(
            err.contains("Contract violation") && err.contains("requires"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn contract_ensures_valid_passes() {
        let src = r#"
            fn double(int n)
                ensures result == n * 2
            {
                return n + n;
            }
            let x = double(7);
        "#;
        let (p, _e) = parse(src);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("x").unwrap(), Value::Int(14)));
    }

    #[test]
    fn contract_ensures_violation_errors() {
        let src = r#"
            fn broken_double(int n)
                ensures result == n * 2
            {
                return n + 1;
            }
            let x = broken_double(7);
        "#;
        let (p, _e) = parse(src);
        let mut interp = Interpreter::new();
        let err = interp.eval(&p).unwrap_err();
        assert!(
            err.contains("Contract violation") && err.contains("ensures"),
            "unexpected error: {}",
            err
        );
        assert!(
            err.contains("result = 8"),
            "expected result value in error, got: {}",
            err
        );
    }

    #[test]
    fn contract_multiple_clauses() {
        let src = r#"
            fn clamped(int n, int lo, int hi)
                requires lo <= hi
                requires n >= lo
                requires n <= hi
                ensures result == n
            {
                return n;
            }
            let x = clamped(5, 0, 10);
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("x").unwrap(), Value::Int(5)));
    }

    #[test]
    fn array_literal_and_index() {
        let (p, errors) = parse("let a = [10, 20, 30]; let b = a[1];");
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("b").unwrap(), Value::Int(20)));
    }

    #[test]
    fn array_index_assignment() {
        let src = "let a = [1, 2, 3]; a[0] = 99;";
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        match interp.env.get("a").unwrap() {
            Value::Array(items) => {
                assert_eq!(items.len(), 3);
                assert!(matches!(items[0], Value::Int(99)));
                assert!(matches!(items[1], Value::Int(2)));
            }
            other => panic!("expected Array, got {:?}", other),
        }
    }

    #[test]
    fn array_out_of_bounds_errors() {
        let (p, _e) = parse("let a = [1]; let b = a[5];");
        let mut interp = Interpreter::new();
        let err = interp.eval(&p).unwrap_err();
        assert!(err.contains("out of bounds"), "{}", err);
    }

    #[test]
    fn array_concat_and_len() {
        let (p, _e) = parse("let a = [1,2] + [3,4,5]; let n = len(a);");
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("n").unwrap(), Value::Int(5)));
        match interp.env.get("a").unwrap() {
            Value::Array(items) => assert_eq!(items.len(), 5),
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn empty_array_literal() {
        let (p, errors) = parse("let a = []; let n = len(a);");
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("n").unwrap(), Value::Int(0)));
    }

    #[test]
    fn nested_array() {
        let (p, _e) = parse("let m = [[1,2],[3,4]]; let x = m[1][0];");
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("x").unwrap(), Value::Int(3)));
    }

    #[test]
    fn bitwise_operators() {
        let (p, _e) = parse(r#"
            let a = 0xF0 & 0x33;
            let b = 0x01 | 0x02;
            let c = 0xFF ^ 0x0F;
            let d = 1 << 4;
            let e = 256 >> 3;
        "#);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("a").unwrap(), Value::Int(0x30)));
        assert!(matches!(interp.env.get("b").unwrap(), Value::Int(0x03)));
        assert!(matches!(interp.env.get("c").unwrap(), Value::Int(0xF0)));
        assert!(matches!(interp.env.get("d").unwrap(), Value::Int(16)));
        assert!(matches!(interp.env.get("e").unwrap(), Value::Int(32)));
    }

    #[test]
    fn bitwise_shift_out_of_range_errors() {
        let (p, _e) = parse("let x = 1 << 64;");
        let mut interp = Interpreter::new();
        let err = interp.eval(&p).unwrap_err();
        assert!(err.contains("out of range"), "{}", err);
    }

    #[test]
    fn assert_shows_both_operands() {
        // RES-028: when an infix comparison assert fails, both sides
        // appear in the error so the user can see the actual values.
        let src = r#"
            let fuel = -5;
            assert(fuel >= 0, "Fuel must be non-negative");
        "#;
        let (p, _e) = parse(src);
        let mut interp = Interpreter::new();
        let err = interp.eval(&p).unwrap_err();
        assert!(err.contains("Fuel must be non-negative"), "msg lost: {}", err);
        assert!(
            err.contains("-5 >= 0") || err.contains("condition -5 >= 0"),
            "expected both operands in error, got: {}",
            err
        );
    }

    #[test]
    fn hex_and_binary_literals() {
        let (p, _e) = parse("let a = 0xFF; let b = 0b1010; let c = 0xDEAD_BEEF;");
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("a").unwrap(), Value::Int(255)));
        assert!(matches!(interp.env.get("b").unwrap(), Value::Int(10)));
        assert!(matches!(interp.env.get("c").unwrap(), Value::Int(0xDEADBEEF)));
    }

    #[test]
    fn block_comments_are_stripped() {
        let src = "let /* inline */ x = /* another */ 42; /* trailing */";
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("x").unwrap(), Value::Int(42)));
    }

    #[test]
    fn block_comment_spanning_lines() {
        let src = "let x = 1;\n/* line two\nand three */\nlet y = 2;";
        let (_p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
    }

    #[test]
    fn while_loop_counts_to_ten() {
        let src = r#"
            let i = 0;
            let sum = 0;
            while i < 10 {
                sum = sum + i;
                i = i + 1;
            }
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        match interp.env.get("sum").unwrap() {
            Value::Int(n) => assert_eq!(n, 45), // 0+1+..+9
            other => panic!("expected Int(45), got {:?}", other),
        }
    }

    #[test]
    fn while_loop_runaway_is_capped() {
        // A tight `while true` should error out rather than hang.
        let (p, _e) = parse("let x = 0; while true { x = x + 1; }");
        let mut interp = Interpreter::new();
        let err = interp.eval(&p).unwrap_err();
        assert!(err.contains("runaway"), "{}", err);
    }

    #[test]
    fn string_comparisons() {
        let (p, _e) = parse(r#"
            let a = "apple" < "banana";
            let b = "abc" == "abc";
            let c = "xy" >= "xz";
            let d = len("héllo");
        "#);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        let g = |n: &str| interp.env.get(n).unwrap();
        assert!(matches!(g("a"), Value::Bool(true)));
        assert!(matches!(g("b"), Value::Bool(true)));
        assert!(matches!(g("c"), Value::Bool(false)));
        // "héllo" is 5 Unicode scalars.
        assert!(matches!(g("d"), Value::Int(5)));
    }

    #[test]
    fn logical_and_or_evaluate() {
        let (p, _e) = parse(r#"
            let a = true && false;
            let b = true || false;
            let c = false || (1 < 2);
            let d = (5 > 0) && (5 < 10);
        "#);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        let g = |n: &str| match interp.env.get(n).unwrap() {
            Value::Bool(b) => b,
            other => panic!("expected Bool for {}, got {:?}", n, other),
        };
        assert!(!g("a"));
        assert!(g("b"));
        assert!(g("c"));
        assert!(g("d"));
    }

    #[test]
    fn if_with_and_or_condition() {
        // Integration with parser: complex conditions in `if`.
        let (_p, errors) = parse("fn f() { if true && false { let x = 1; } }");
        assert!(errors.is_empty(), "{:?}", errors);
    }

    #[test]
    fn math_builtins_sqrt_pow_floor_ceil() {
        let src = r#"
            let a = sqrt(16);
            let b = pow(2, 10);
            let c = floor(3.7);
            let d = ceil(3.2);
        "#;
        let (p, _e) = parse(src);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        let get = |n: &str| interp.env.get(n).unwrap();
        assert!(matches!(get("a"), Value::Float(v) if (v - 4.0).abs() < 1e-9));
        assert!(matches!(get("b"), Value::Float(v) if (v - 1024.0).abs() < 1e-9));
        assert!(matches!(get("c"), Value::Float(v) if (v - 3.0).abs() < 1e-9));
        assert!(matches!(get("d"), Value::Float(v) if (v - 4.0).abs() < 1e-9));
    }

    #[test]
    fn math_builtins_abs_min_max() {
        let src = r#"
            let a = abs(-5);
            let b = abs(-3.5);
            let c = min(3, 7);
            let d = max(3.0, 7);
        "#;
        let (p, _e) = parse(src);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        let get = |n: &str| interp.env.get(n).unwrap();
        assert!(matches!(get("a"), Value::Int(5)));
        assert!(matches!(get("b"), Value::Float(v) if (v - 3.5).abs() < 1e-9));
        assert!(matches!(get("c"), Value::Int(3)));
        assert!(matches!(get("d"), Value::Float(v) if (v - 7.0).abs() < 1e-9));
    }

    #[test]
    fn math_builtins_arity_checks() {
        let e_abs = builtin_abs(&[Value::Int(1), Value::Int(2)]).unwrap_err();
        assert!(e_abs.contains("expected 1"), "{}", e_abs);
        let e_min = builtin_min(&[Value::Int(1)]).unwrap_err();
        assert!(e_min.contains("expected 2"), "{}", e_min);
    }

    #[test]
    fn forward_reference_between_functions() {
        // RES-018: caller is defined before callee, which only works if
        // eval_program hoists function definitions.
        let src = r#"
            fn caller() { return callee(); }
            fn callee() { return 42; }
            let x = caller();
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        match interp.env.get("x").unwrap() {
            Value::Int(n) => assert_eq!(n, 42),
            other => panic!("expected Int(42), got {:?}", other),
        }
    }

    #[test]
    fn static_let_persists_across_calls() {
        // RES-013: counter survives across calls. Three calls → 1, 2, 3.
        let src = r#"
            fn tick() {
                static let n = 0;
                n = n + 1;
                return n;
            }
            let a = tick();
            let b = tick();
            let c = tick();
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        let extract = |name: &str| match interp.env.get(name).unwrap() {
            Value::Int(n) => n,
            other => panic!("expected Int for {}, got {:?}", name, other),
        };
        assert_eq!(extract("a"), 1);
        assert_eq!(extract("b"), 2);
        assert_eq!(extract("c"), 3);
    }

    #[test]
    fn assignment_updates_variable() {
        let (p, errors) = parse("let x = 1; x = 42;");
        assert!(errors.is_empty(), "parse errors: {:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        match interp.env.get("x").unwrap() {
            Value::Int(n) => assert_eq!(n, 42),
            other => panic!("expected Int(42), got {:?}", other),
        }
    }

    #[test]
    fn assignment_to_undeclared_errors() {
        let (p, _e) = parse("x = 42;");
        let mut interp = Interpreter::new();
        let err = interp.eval(&p).unwrap_err();
        assert!(
            err.contains("Cannot assign to undeclared variable"),
            "err was: {}",
            err
        );
    }

    #[test]
    fn error_message_includes_line_and_column() {
        // RES-005: errors carry `line:col:` prefix from the Lexer.
        let src = "fn main() {\n    let = 1;\n}";
        let (_p, errors) = parse(src);
        assert!(!errors.is_empty(), "expected an error for missing ident");
        // The missing identifier is on line 2.
        let first = &errors[0];
        assert!(
            first.starts_with("2:"),
            "expected error prefixed with '2:', got: {}",
            first
        );
    }

    #[test]
    fn lexer_tracks_line_across_newlines() {
        let mut lex = Lexer::new("let x = 1;\nlet y = 2;".to_string());
        let _ = lex.next_token(); // let (line 1)
        let _ = lex.next_token(); // x
        let _ = lex.next_token(); // =
        let _ = lex.next_token(); // 1
        let _ = lex.next_token(); // ;
        let _ = lex.next_token(); // let (line 2)
        assert_eq!(lex.last_token_line, 2, "second `let` should be on line 2");
    }

    #[test]
    fn int_modulo() {
        let (p, _e) = parse("let x = 7 % 3;");
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        match interp.env.get("x").unwrap() {
            Value::Int(n) => assert_eq!(n, 1),
            other => panic!("expected Int(1), got {:?}", other),
        }
    }

    #[test]
    fn int_modulo_by_zero_errors() {
        let (p, _e) = parse("let x = 5 % 0;");
        let mut interp = Interpreter::new();
        let err = interp.eval(&p).unwrap_err();
        assert!(err.contains("Modulo by zero"), "err: {}", err);
    }

    #[test]
    fn prefix_bang_evaluates() {
        let (p, _e) = parse("let x = !true;");
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        match interp.env.get("x").unwrap() {
            Value::Bool(b) => assert!(!b),
            other => panic!("expected Bool(false), got {:?}", other),
        }
    }

    #[test]
    fn prefix_minus_evaluates() {
        let (p, _e) = parse("let x = -5;");
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        match interp.env.get("x").unwrap() {
            Value::Int(n) => assert_eq!(n, -5),
            other => panic!("expected Int(-5), got {:?}", other),
        }
    }

    #[test]
    fn prefix_bang_on_identifier() {
        let (p, errors) = parse("let t = true; let f = !t;");
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        match interp.env.get("f").unwrap() {
            Value::Bool(b) => assert!(!b),
            other => panic!("expected Bool(false), got {:?}", other),
        }
    }

    #[test]
    fn parser_if_with_infix_condition() {
        // RES-014: `if call_expr() < 0.5 { ... }` used to report
        // "Expected '{' after if condition, found FloatLiteral(0.5)"
        // because parse_expression left current_token on the last
        // literal of the condition.
        let (_p, errors) = parse("fn f() { if 1 < 2 { let x = 1; } }");
        assert!(errors.is_empty(), "expected no errors, got: {:?}", errors);
    }

    #[test]
    fn parser_if_with_function_call_comparison() {
        let (_p, errors) = parse("fn f() { if add(1, 2) == 3 { let x = 1; } }");
        assert!(errors.is_empty(), "expected no errors, got: {:?}", errors);
    }

    #[test]
    fn parser_recovers_from_missing_if_brace() {
        // RES-009: before this ticket, `if x == 1 foo();` (no `{` after
        // the condition) panicked the whole interpreter. Now it should
        // record a parse error and keep going.
        let (_program, errors) = parse("fn f() { if x == 1 x; }");
        assert!(
            !errors.is_empty(),
            "expected a parse error for missing `{{`"
        );
        assert!(
            errors.iter().any(|e| e.contains("Expected '{'")),
            "expected a message naming the missing brace, got {:?}",
            errors
        );
    }

    #[test]
    fn parser_accepts_bare_return() {
        // RES-011: `return;` used to panic on unwrap().
        let (program, errors) = parse("fn foo() { return; }");
        assert!(errors.is_empty(), "errors: {:?}", errors);
        match program {
            Node::Program(stmts) => match &stmts[0] {
                Node::Function { body, .. } => match body.as_ref() {
                    Node::Block(inner) => match &inner[0] {
                        Node::ReturnStatement { value } => assert!(value.is_none()),
                        other => panic!("expected ReturnStatement, got {:?}", other),
                    },
                    other => panic!("expected Block, got {:?}", other),
                },
                other => panic!("expected Function, got {:?}", other),
            },
            other => panic!("expected Program, got {:?}", other),
        }
    }

    #[test]
    fn parser_accepts_return_with_value() {
        let (_program, errors) = parse("fn foo() { return 42; }");
        assert!(errors.is_empty(), "errors: {:?}", errors);
    }

    #[test]
    fn interpreter_evaluates_let_and_arithmetic() {
        let (program, errors) = parse("let x = 40; let y = x + 2;");
        assert!(errors.is_empty(), "parse errors: {:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&program).expect("eval should succeed");
        match interp.env.get("y").expect("y defined") {
            Value::Int(v) => assert_eq!(v, 42),
            other => panic!("expected Int(42), got {:?}", other),
        }
    }
}