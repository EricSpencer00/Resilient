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
                } else {
                    Token::Assign
                }
            },
            '+' => Token::Plus,
            '-' => Token::Minus,
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

// AST nodes for our parser
#[derive(Debug, Clone)]
enum Node {
    Program(Vec<Node>),
    Function {
        name: String,
        parameters: Vec<(String, String)>, // (type, name)
        body: Box<Node>,
    },
    LiveBlock {
        body: Box<Node>,
    },
    Assert {
        condition: Box<Node>,
        message: Option<Box<Node>>,
    },
    Block(Vec<Node>),
    LetStatement {
        name: String,
        value: Box<Node>,
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
            Token::Let => Some(self.parse_let_statement()),
            Token::Static => Some(self.parse_static_let_statement()),
            Token::Return => Some(self.parse_return_statement()),
            Token::Live => Some(self.parse_live_block()),
            Token::Assert => Some(self.parse_assert()),
            Token::If => Some(self.parse_if_statement()),
            Token::While => Some(self.parse_while_statement()),
            Token::Unknown(ch) => {
                self.record_error(format!("Unexpected character '{}'", ch));
                None
            }
            // Assignment: `IDENT = EXPR;` — disambiguated from an
            // expression statement by looking ahead for `=`.
            Token::Identifier(_) if self.peek_token == Token::Assign => {
                Some(self.parse_assignment())
            }
            _ => self.parse_expression_statement(),
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
                };
            }
            
            let body = self.parse_block_statement();
            return Node::Function {
                name,
                parameters: Vec::new(),
                body: Box::new(body),
            };
        }
        
        self.next_token(); // Skip '('
        
        let parameters = self.parse_function_parameters();
        
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
                };
            }
        }
        
        let body = self.parse_block_statement();
        
        Node::Function {
            name,
            parameters,
            body: Box::new(body),
        }
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
            Node::LetStatement { name, value } => Node::StaticLet { name, value },
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
                };
            }
        };

        self.next_token(); // Skip name

        if self.current_token != Token::Assign {
            let tok = self.current_token.clone();
            self.record_error(format!(
                "Expected '=' after identifier '{}' in let statement, found {:?}",
                name, tok
            ));
            return Node::LetStatement {
                name,
                value: Box::new(Node::IntegerLiteral(0)),
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
        
        if self.current_token != Token::LeftBrace {
            let tok = self.current_token.clone();
            self.record_error(format!("Expected '{{' after 'live', found {:?}", tok));
            return Node::LiveBlock {
                body: Box::new(Node::Block(Vec::new())),
            };
        }

        let body = self.parse_block_statement();
        
        Node::LiveBlock {
            body: Box::new(body),
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
    },
    /// Native function. `name` is the identifier it was registered as,
    /// for diagnostics only.
    Builtin {
        name: &'static str,
        func: BuiltinFn,
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

/// `len(s)` — length of a string, in Unicode scalars (not bytes). Returns int.
fn builtin_len(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(s)] => Ok(Value::Int(s.chars().count() as i64)),
        [other] => Err(format!("len: expected string, got {:?}", other)),
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
            Node::Function { name, parameters, body } => {
                let func = Value::Function {
                    parameters: parameters.clone(),
                    body: body.clone(),
                    env: self.env.clone(),
                };
                self.env.set(name.clone(), func);
                Ok(Value::Void)
            },
            Node::LiveBlock { body } => self.eval_live_block(body),
            Node::Assert { condition, message } => self.eval_assert(condition, message),
            Node::Block(statements) => self.eval_block_statement(statements),
            Node::LetStatement { name, value } => {
                let val = self.eval(value)?;
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
    
    fn eval_live_block(&mut self, body: &Node) -> RResult<Value> {
        const MAX_RETRIES: usize = 3;
        let mut retry_count = 0;
        
        // Create a snapshot of the environment
        let env_snapshot = self.env.clone();
        
        // Log the start of live block execution
        eprintln!("\x1B[36m[LIVE BLOCK] Starting execution of live block\x1B[0m");
        
        // Try to evaluate the body with multiple retries
        loop {
            match self.eval(body) {
                Ok(value) => {
                    eprintln!("\x1B[32m[LIVE BLOCK] Successfully executed live block\x1B[0m");
                    return Ok(value)
                },
                Err(error) => {
                    retry_count += 1;
                    
                    // Log the error with more context and colorized output
                    eprintln!("\x1B[33m[LIVE BLOCK] Error detected (attempt {}/{}): {}\x1B[0m", 
                              retry_count, MAX_RETRIES, error);
                    
                    if retry_count >= MAX_RETRIES {
                        eprintln!("\x1B[31m[LIVE BLOCK] Maximum retry attempts reached, propagating error\x1B[0m");
                        return Err(format!("Live block failed after {} attempts: {}", MAX_RETRIES, error));
                    }
                    
                    eprintln!("\x1B[36m[LIVE BLOCK] Restoring environment to last known good state\x1B[0m");
                    eprintln!("\x1B[36m[LIVE BLOCK] Retrying execution (attempt {}/{})\x1B[0m", 
                              retry_count + 1, MAX_RETRIES);
                    
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
            Value::Function { parameters, body, env } => {
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
                let result = interpreter.eval(&body)?;

                if let Value::Return(value) = result {
                    Ok(*value)
                } else {
                    Ok(result)
                }
            }
            Value::Builtin { func, .. } => func(&args),
            _ => Err(format!("Not a function: {}", func)),
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
                    Node::LetStatement { name, value } => {
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
    fn lexer_emits_unknown_instead_of_panicking_on_dot() {
        // RES-010: a stray `.` used to hit `panic!("Unexpected character: .")`.
        // Now it comes out as Token::Unknown('.') and the parser keeps going.
        let tokens = tokenize(". 1.5");
        assert_eq!(
            tokens[0],
            Token::Unknown('.'),
            "expected Token::Unknown('.'), got {:?}",
            tokens[0]
        );
        // And the following valid float still lexes.
        assert!(
            tokens.iter().any(|t| matches!(t, Token::FloatLiteral(f) if *f == 1.5)),
            "expected FloatLiteral(1.5) to follow, got {:?}",
            tokens
        );
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