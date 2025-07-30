use std::collections::HashMap;
use std::env;
use std::fs;
use std::io;
use std::io::Write;
use std::path::Path;

// Import the typechecker module
mod typechecker;
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
    Assign,
    Equal,
    NotEqual,
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
    
    // Other
    Eof,
}

// Lexer for tokenizing Resilient source code
struct Lexer {
    input: Vec<char>,
    position: usize,
    read_position: usize,
    ch: char,
}

impl Lexer {
    fn new(input: String) -> Self {
        let mut lexer = Lexer {
            input: input.chars().collect(),
            position: 0,
            read_position: 0,
            ch: '\0',
        };
        lexer.read_char();
        lexer
    }
    
    fn read_char(&mut self) {
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
            '/' => {
                if self.peek_char() == '/' {
                    // Skip comment line
                    while self.ch != '\n' && self.ch != '\0' {
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
                } else {
                    Token::Greater
                }
            },
            '<' => {
                if self.peek_char() == '=' {
                    self.read_char();
                    Token::LessEqual
                } else {
                    Token::Less
                }
            },
            '!' => {
                if self.peek_char() == '=' {
                    self.read_char();
                    Token::NotEqual
                } else {
                    panic!("Unexpected character: !");
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
                    let ident = self.read_identifier();
                    match ident.as_str() {
                        "fn" => Token::Function,
                        "let" => Token::Let,
                        "live" => Token::Live,
                        "assert" => Token::Assert,
                        "if" => Token::If,
                        "else" => Token::Else,
                        "return" => Token::Return,
                        "true" => Token::BoolLiteral(true),
                        "false" => Token::BoolLiteral(false),
                        _ => Token::Identifier(ident),
                    }
                } else if self.is_digit(self.ch) {
                    return self.read_number();
                } else {
                    panic!("Unexpected character: {}", self.ch);
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
            Token::FloatLiteral(number_str.parse::<f64>().unwrap())
        } else {
            Token::IntLiteral(number_str.parse::<i64>().unwrap())
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
        ch.is_digit(10)
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
    ReturnStatement {
        value: Box<Node>,
    },
    IfStatement {
        condition: Box<Node>,
        consequence: Box<Node>,
        alternative: Option<Box<Node>>,
    },
    ExpressionStatement(Box<Node>),
    Identifier(String),
    IntegerLiteral(i64),
    FloatLiteral(f64),
    StringLiteral(String),
    BooleanLiteral(bool),
    // Reserved for future language features - unary operators
    #[allow(dead_code)]
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
}

impl Parser {
    fn new(lexer: Lexer) -> Self {
        let mut parser = Parser {
            lexer,
            current_token: Token::Eof,
            peek_token: Token::Eof,
        };
        
        parser.next_token();
        parser.next_token();
        parser
    }
    
    fn next_token(&mut self) {
        self.current_token = self.peek_token.clone();
        self.peek_token = self.lexer.next_token();
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
            Token::Return => Some(self.parse_return_statement()),
            Token::Live => Some(self.parse_live_block()),
            Token::Assert => Some(self.parse_assert()),
            Token::If => Some(self.parse_if_statement()),
            _ => self.parse_expression_statement(),
        }
    }
    
    fn parse_function(&mut self) -> Node {
        self.next_token(); // Skip 'fn'
        
        let name = match &self.current_token {
            Token::Identifier(name) => name.clone(),
            _ => panic!("Expected identifier after 'fn'"),
        };
        
        self.next_token(); // Skip name
        
        // Check if we have a left parenthesis as expected
        if self.current_token != Token::LeftParen {
            // For better error messages, provide more context
            if name == "main" {
                panic!("Expected '(' after function name. Functions in Resilient must have parameters, even if unused. Try: fn main(int dummy) {{ ... }}");
            } else {
                panic!("Expected '(' after function name");
            }
        }
        
        self.next_token(); // Skip '('
        
        let parameters = self.parse_function_parameters();
        
        if self.current_token != Token::LeftBrace {
            panic!("Expected '{{' after function parameters");
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
                _ => panic!("Expected parameter type"),
            };
            
            self.next_token(); // Skip type
            
            let param_name = match &self.current_token {
                Token::Identifier(name) => name.clone(),
                _ => panic!("Expected parameter name"),
            };
            
            parameters.push((param_type, param_name));
            
            self.next_token(); // Skip name
            
            if self.current_token == Token::Comma {
                self.next_token(); // Skip ','
            } else if self.current_token != Token::RightParen {
                panic!("Expected ',' or ')' after parameter");
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
    
    fn parse_let_statement(&mut self) -> Node {
        self.next_token(); // Skip 'let'
        
        let name = match &self.current_token {
            Token::Identifier(name) => name.clone(),
            _ => panic!("Expected identifier after 'let'"),
        };
        
        self.next_token(); // Skip name
        
        if self.current_token != Token::Assign {
            panic!("Expected '=' after identifier in let statement");
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
        
        let value = self.parse_expression(0).unwrap();
        
        if self.peek_token == Token::Semicolon {
            self.next_token(); // Skip to semicolon
        }
        
        Node::ReturnStatement {
            value: Box::new(value),
        }
    }
    
    fn parse_live_block(&mut self) -> Node {
        self.next_token(); // Skip 'live'
        
        if self.current_token != Token::LeftBrace {
            panic!("Expected '{{' after 'live'");
        }
        
        let body = self.parse_block_statement();
        
        Node::LiveBlock {
            body: Box::new(body),
        }
    }
    
    fn parse_assert(&mut self) -> Node {
        self.next_token(); // Skip 'assert'
        
        if self.current_token != Token::LeftParen {
            panic!("Expected '(' after 'assert'");
        }
        
        self.next_token(); // Skip '('
        
        let condition = self.parse_expression(0).unwrap();
        
        let message = if self.current_token == Token::Comma {
            self.next_token(); // Skip ','
            Some(Box::new(self.parse_expression(0).unwrap()))
        } else {
            None
        };
        
        if self.current_token != Token::RightParen {
            panic!("Expected ')' after assert condition");
        }
        
        Node::Assert {
            condition: Box::new(condition),
            message,
        }
    }
    
    fn parse_if_statement(&mut self) -> Node {
        self.next_token(); // Skip 'if'
        
        // Handle both if (condition) and if condition forms
        let condition = if self.current_token == Token::LeftParen {
            self.next_token(); // Skip '('
            let expr = self.parse_expression(0).unwrap();
            
            if self.current_token != Token::RightParen {
                panic!("Expected ')' after if condition");
            }
            self.next_token(); // Skip ')'
            expr
        } else {
            let expr = self.parse_expression(0).unwrap();
            expr
        };
        
        if self.current_token != Token::LeftBrace {
            panic!("Expected '{{' after if condition");
        }
        
        let consequence = self.parse_block_statement();
        
        let alternative = if self.peek_token == Token::Else {
            self.next_token(); // Move to 'else'
            self.next_token(); // Skip 'else'
            
            if self.current_token != Token::LeftBrace {
                panic!("Expected '{{' after 'else'");
            }
            
            Some(Box::new(self.parse_block_statement()))
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
            Token::LeftParen => {
                self.next_token(); // Skip '('
                let expr = self.parse_expression(0);
                if self.current_token != Token::RightParen {
                    panic!("Expected ')'");
                }
                expr
            },
            _ => None,
        };
        
        // Parse infix expressions
        while self.peek_token != Token::Semicolon && precedence < self.peek_precedence() {
            left_expr = match &self.peek_token {
                Token::Plus | Token::Minus | Token::Multiply | Token::Divide |
                Token::Equal | Token::NotEqual | Token::Less | Token::Greater |
                Token::LessEqual | Token::GreaterEqual => {
                    self.next_token();
                    self.parse_infix_expression(left_expr.unwrap())
                },
                Token::LeftParen => {
                    self.next_token();
                    self.parse_call_expression(left_expr.unwrap())
                },
                _ => left_expr,
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
            Token::Equal => "==".to_string(),
            Token::NotEqual => "!=".to_string(),
            Token::Less => "<".to_string(),
            Token::Greater => ">".to_string(),
            Token::LessEqual => "<=".to_string(),
            Token::GreaterEqual => ">=".to_string(),
            _ => panic!("Invalid operator"),
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
            panic!("Expected ')' after arguments");
        }
        
        self.next_token(); // Skip to ')'
        
        args
    }
    
    fn current_precedence(&self) -> u8 {
        match &self.current_token {
            Token::Equal | Token::NotEqual => 2,
            Token::Less | Token::Greater | Token::LessEqual | Token::GreaterEqual => 3,
            Token::Plus | Token::Minus => 4,
            Token::Multiply | Token::Divide => 5,
            Token::LeftParen => 6,
            _ => 0,
        }
    }
    
    fn peek_precedence(&self) -> u8 {
        match &self.peek_token {
            Token::Equal | Token::NotEqual => 2,
            Token::Less | Token::Greater | Token::LessEqual | Token::GreaterEqual => 3,
            Token::Plus | Token::Minus => 4,
            Token::Multiply | Token::Divide => 5,
            Token::LeftParen => 6,
            _ => 0,
        }
    }
}

// Value types for our interpreter
#[derive(Debug, Clone)]
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
    Return(Box<Value>),
    Void,
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Value::Int(i) => write!(f, "{}", i),
            Value::Float(fl) => write!(f, "{}", fl),
            Value::String(s) => write!(f, "\"{}\"", s),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Function { .. } => write!(f, "<function>"),
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
}

// Interpreter for executing Resilient programs
struct Interpreter {
    env: Environment,
}

impl Interpreter {
    fn new() -> Self {
        Interpreter {
            env: Environment::new(),
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
            Node::ReturnStatement { value } => {
                let val = self.eval(value)?;
                Ok(Value::Return(Box::new(val)))
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
                match self.env.get(name) {
                    Some(value) => Ok(value),
                    None => Err(format!("Identifier not found: {}", name)),
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
        let mut result = Value::Void;
        
        for statement in statements {
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
        
        // Try to evaluate the body with multiple retries
        loop {
            match self.eval(body) {
                Ok(value) => return Ok(value),
                Err(error) => {
                    retry_count += 1;
                    
                    // Log the error with more context
                    eprintln!("[LIVE BLOCK] Error detected (attempt {}/{}): {}", 
                              retry_count, MAX_RETRIES, error);
                    
                    if retry_count >= MAX_RETRIES {
                        eprintln!("[LIVE BLOCK] Maximum retry attempts reached, propagating error");
                        return Err(format!("Live block failed after {} attempts: {}", MAX_RETRIES, error));
                    }
                    
                    eprintln!("[LIVE BLOCK] Restoring environment to last known good state and retrying...");
                    
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
            
            // Create a more detailed error message
            let formatted_error = format!("ASSERTION ERROR: {}\n  - Condition evaluated to: {}", 
                                         error_message, condition_value);
            
            return Err(formatted_error);
        }
        
        Ok(Value::Void)
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
            Value::Float(f) if f == 0.0 => Ok(Value::Bool(true)),
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
        match operator {
            "+" => Ok(Value::String(format!("{}{}", left, right))),
            "==" => Ok(Value::Bool(left == right)),
            "!=" => Ok(Value::Bool(left != right)),
            _ => Err(format!("Unknown operator: {} {} {}", left, operator, right)),
        }
    }
    
    fn eval_boolean_infix_expression(&mut self, operator: &str, left: bool, right: bool) -> RResult<Value> {
        match operator {
            "==" => Ok(Value::Bool(left == right)),
            "!=" => Ok(Value::Bool(left != right)),
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
                
                let mut interpreter = Interpreter { env: extended_env };
                let result = interpreter.eval(&body)?;
                
                if let Value::Return(value) = result {
                    Ok(*value)
                } else {
                    Ok(result)
                }
            },
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

// REPL for interactive evaluation
fn start_repl() -> RustylineResult<()> {
    let mut interpreter = Interpreter::new();
    let mut rl = DefaultEditor::new()?;
    let mut type_check_enabled = false;
    
    // Load history if available
    let history_path = match env::var("HOME") {
        Ok(home) => Path::new(&home).join(".resilient_history"),
        Err(_) => Path::new(".resilient_history").to_path_buf(),
    };
    
    if history_path.exists() {
        if let Err(err) = rl.load_history(&history_path) {
            eprintln!("Error loading history: {}", err);
        }
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
        for i in 1..args.len() {
            if args[i] == "--typecheck" || args[i] == "-t" {
                type_check = true;
            } else {
                filename = &args[i];
            }
        }
        
        if !filename.is_empty() {
            // Execute a file
            match execute_file(filename, type_check) {
                Ok(_) => println!("Program executed successfully"),
                Err(e) => eprintln!("Error: {}", e),
            }
            return;
        }
    }
    
    // Start REPL if no file was provided
    if let Err(e) = start_repl() {
        eprintln!("REPL error: {}", e);
    }
}