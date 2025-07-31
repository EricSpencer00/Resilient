// Improved Parser for Resilient language
use crate::{Lexer, Token, Node};
use std::fmt;

// Error type for better diagnostics
#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
    pub token: Token,
    pub line: usize,  // We'll need to track line numbers
    pub column: usize, // And column positions
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Parse error at line {}, column {}: {} (token: {:?})", 
               self.line, self.column, self.message, self.token)
    }
}

pub struct Parser {
    lexer: Lexer,
    current_token: Token,
    peek_token: Token,
    pub errors: Vec<ParseError>,
    line: usize,    // Track current line
    column: usize,  // Track current column
}

impl Parser {
    pub fn new(lexer: Lexer) -> Self {
        let mut parser = Parser {
            lexer,
            current_token: Token::Eof,
            peek_token: Token::Eof,
            errors: Vec::new(),
            line: 1,
            column: 1,
        };
        
        parser.next_token();
        parser.next_token();
        parser
    }
    
    pub fn next_token(&mut self) {
        self.current_token = self.peek_token.clone();
        self.peek_token = self.lexer.next_token();
        
        // Update position tracking (ideally lexer would provide this)
        // This is a simplified version - in a real implementation, 
        // the lexer would track positions accurately
        if let Token::StringLiteral(s) = &self.current_token {
            for c in s.chars() {
                if c == '\n' {
                    self.line += 1;
                    self.column = 1;
                } else {
                    self.column += 1;
                }
            }
        } else {
            // Simple approximation
            self.column += 1;
            if self.current_token == Token::Semicolon {
                self.line += 1;
                self.column = 1;
            }
        }
    }
    
    pub fn parse_program(&mut self) -> Result<Node, Vec<ParseError>> {
        let mut program = Vec::new();
        
        while self.current_token != Token::Eof {
            match self.parse_statement() {
                Ok(statement) => program.push(statement),
                Err(error) => self.errors.push(error)
            }
            
            // Try to recover from errors by skipping to next statement
            if self.errors.len() > 0 {
                self.synchronize();
            }
            
            // Move to next statement if we're not at EOF
            if self.current_token != Token::Eof {
                self.next_token();
            }
        }
        
        if !self.errors.is_empty() {
            Err(self.errors.clone())
        } else {
            Ok(Node::Program(program))
        }
    }
    
    // Error recovery - skip tokens until we find a likely statement boundary
    fn synchronize(&mut self) {
        self.next_token();
        
        while self.current_token != Token::Eof {
            // Semicolons mark the end of statements
            if self.current_token == Token::Semicolon {
                self.next_token();
                return;
            }
            
            // Keywords likely indicate the start of a new statement
            match self.current_token {
                Token::Function | Token::Let | Token::Return | 
                Token::Live | Token::Assert | Token::If => return,
                _ => {}
            }
            
            self.next_token();
        }
    }
    
    fn parse_statement(&mut self) -> Result<Node, ParseError> {
        match self.current_token {
            Token::Function => self.parse_function(),
            Token::Let => self.parse_let_statement(),
            Token::Return => Ok(self.parse_return_statement()),
            Token::Live => Ok(self.parse_live_block()),
            Token::Assert => Ok(self.parse_assert()),
            Token::If => Ok(self.parse_if_statement()),
            _ => Ok(self.parse_expression_statement()),
        }
    }
    
    fn parse_function(&mut self) -> Result<Node, ParseError> {
        self.next_token(); // Skip 'fn'
        
        let name = match &self.current_token {
            Token::Identifier(name) => name.clone(),
            _ => {
                return Err(ParseError {
                    message: format!("Expected identifier after 'fn', found {:?}", self.current_token),
                    token: self.current_token.clone(),
                    line: self.line,
                    column: self.column,
                });
            },
        };
        
        self.next_token(); // Skip name
        
        // Check if we have a left parenthesis as expected
        if self.current_token != Token::LeftParen {
            // For better error messages, provide more context
            let message = if name == "main" {
                format!("Expected '(' after function name '{}'. Functions in Resilient must have parameters, even if unused. Try: fn main(int dummy) {{ ... }}", name)
            } else {
                format!("Expected '(' after function name '{}'", name)
            };
            
            return Err(ParseError {
                message,
                token: self.current_token.clone(),
                line: self.line,
                column: self.column,
            });
        }
        
        self.next_token(); // Skip '('
        
        let parameters = self.parse_function_parameters()?;
        
        if self.current_token != Token::LeftBrace {
            return Err(ParseError {
                message: format!("Expected '{{' after function parameters for '{}'", name),
                token: self.current_token.clone(),
                line: self.line,
                column: self.column,
            });
        }
        
        let body = self.parse_block_statement()?;
        
        Ok(Node::Function {
            name,
            parameters,
            body: Box::new(body),
        })
    }
    
    fn parse_function_parameters(&mut self) -> Result<Vec<(String, String)>, ParseError> {
        let mut parameters = Vec::new();
        
        if self.current_token == Token::RightParen {
            self.next_token(); // Skip ')'
            return Ok(parameters);
        }
        
        while self.current_token != Token::RightParen && self.current_token != Token::Eof {
            let param_type = match &self.current_token {
                Token::Identifier(typ) => typ.clone(),
                _ => {
                    return Err(ParseError {
                        message: format!("Expected parameter type, found {:?}", self.current_token),
                        token: self.current_token.clone(),
                        line: self.line,
                        column: self.column,
                    });
                }
            };
            
            self.next_token(); // Skip type
            
            if self.current_token == Token::RightParen {
                // No parameter name, just type
                parameters.push((param_type, String::from("_unnamed")));
                break;
            }
            
            let param_name = match &self.current_token {
                Token::Identifier(name) => name.clone(),
                _ => {
                    return Err(ParseError {
                        message: format!("Expected parameter name after type '{}', found {:?}", param_type, self.current_token),
                        token: self.current_token.clone(),
                        line: self.line,
                        column: self.column,
                    });
                }
            };
            
            parameters.push((param_type, param_name));
            
            self.next_token(); // Skip parameter name
            
            if self.current_token == Token::Comma {
                self.next_token(); // Skip comma
            } else if self.current_token != Token::RightParen {
                return Err(ParseError {
                    message: format!("Expected ',' or ')' after parameter, found {:?}", self.current_token),
                    token: self.current_token.clone(),
                    line: self.line,
                    column: self.column,
                });
            }
        }
        
        if self.current_token == Token::RightParen {
            self.next_token(); // Skip ')'
        } else {
            return Err(ParseError {
                message: "Unclosed parameter list, expected ')'".to_string(),
                token: self.current_token.clone(),
                line: self.line,
                column: self.column,
            });
        }
        
        Ok(parameters)
    }
    
    fn parse_block_statement(&mut self) -> Result<Node, ParseError> {
        let mut statements = Vec::new();
        
        self.next_token(); // Skip '{'
        
        while self.current_token != Token::RightBrace && self.current_token != Token::Eof {
            match self.parse_statement() {
                Ok(stmt) => statements.push(stmt),
                Err(error) => {
                    // Add error but continue parsing
                    self.errors.push(error);
                    // Try to synchronize to next statement
                    self.synchronize();
                }
            }
            if self.current_token != Token::RightBrace && self.current_token != Token::Eof {
                self.next_token();
            }
        }
        
        if self.current_token == Token::Eof {
            return Err(ParseError {
                message: "Unclosed block, expected '}'".to_string(),
                token: self.current_token.clone(),
                line: self.line,
                column: self.column,
            });
        }
        
        Ok(Node::Block(statements))
    }
    
    fn parse_let_statement(&mut self) -> Result<Node, ParseError> {
        self.next_token(); // Skip 'let'
        
        let name = match &self.current_token {
            Token::Identifier(name) => name.clone(),
            _ => {
                return Err(ParseError {
                    message: format!("Expected identifier after 'let', found {:?}", self.current_token),
                    token: self.current_token.clone(),
                    line: self.line,
                    column: self.column,
                });
            }
        };
        
        self.next_token(); // Skip name
        
        if self.current_token != Token::Assign {
            return Err(ParseError {
                message: format!("Expected '=' after identifier in let statement, found {:?}", self.current_token),
                token: self.current_token.clone(),
                line: self.line,
                column: self.column,
            });
        }
        
        self.next_token(); // Skip '='
        
        // For now, we'll create a simple placeholder for expressions
        // In a real implementation, we would have a proper expression parser
        let value = match &self.current_token {
            Token::IntLiteral(val) => Node::IntegerLiteral(*val),
            Token::FloatLiteral(val) => Node::FloatLiteral(*val),
            Token::StringLiteral(val) => Node::StringLiteral(val.clone()),
            Token::BoolLiteral(val) => Node::BooleanLiteral(*val),
            Token::Identifier(val) => Node::Identifier(val.clone()),
            _ => {
                return Err(ParseError {
                    message: format!("Expected expression after '=' in let statement, found {:?}", self.current_token),
                    token: self.current_token.clone(),
                    line: self.line,
                    column: self.column,
                });
            }
        };
        
        self.next_token(); // Skip value
        
        if self.current_token == Token::Semicolon {
            self.next_token(); // Skip semicolon
        }
        
        Ok(Node::LetStatement {
            name,
            value: Box::new(value),
        })
    }
    
    fn parse_return_statement(&mut self) -> Result<Node, ParseError> {
        self.next_token(); // Skip 'return'
        
        // Empty return statement
        if self.current_token == Token::Semicolon {
            return Ok(Node::ReturnStatement {
                value: Box::new(Node::Void),
            });
        }
        
        let value = match self.parse_expression(0) {
            Some(expr) => expr,
            None => {
                return Err(ParseError {
                    message: format!("Expected expression after 'return', found {:?}", self.current_token),
                    token: self.current_token.clone(),
                    line: self.line,
                    column: self.column,
                });
            }
        };
        
        if self.current_token == Token::Semicolon {
            self.next_token(); // Skip semicolon
        }
        
        Ok(Node::ReturnStatement {
            value: Box::new(value),
        })
    }
    
    fn parse_live_block(&mut self) -> Result<Node, ParseError> {
        self.next_token(); // Skip 'live'
        
        if self.current_token != Token::LeftBrace {
            return Err(ParseError {
                message: format!("Expected '{{' after 'live', found {:?}", self.current_token),
                token: self.current_token.clone(),
                line: self.line,
                column: self.column,
            });
        }
        
        let body = self.parse_block_statement()?;
        
        Ok(Node::LiveBlock {
            body: Box::new(body),
        })
    }
    
    fn parse_assert(&mut self) -> Result<Node, ParseError> {
        self.next_token(); // Skip 'assert'
        
        if self.current_token != Token::LeftParen {
            return Err(ParseError {
                message: format!("Expected '(' after 'assert', found {:?}", self.current_token),
                token: self.current_token.clone(),
                line: self.line,
                column: self.column,
            });
        }
        
        self.next_token(); // Skip '('
        
        let condition = match self.parse_expression(0) {
            Some(expr) => expr,
            None => {
                return Err(ParseError {
                    message: format!("Expected condition expression in assert statement"),
                    token: self.current_token.clone(),
                    line: self.line,
                    column: self.column,
                });
            }
        };
        
        let message = if self.current_token == Token::Comma {
            self.next_token(); // Skip ','
            
            match self.parse_expression(0) {
                Some(expr) => Some(Box::new(expr)),
                None => {
                    return Err(ParseError {
                        message: format!("Expected message expression after comma in assert statement"),
                        token: self.current_token.clone(),
                        line: self.line,
                        column: self.column,
                    });
                }
            }
        } else {
            None
        };
        
        if self.current_token != Token::RightParen {
            return Err(ParseError {
                message: format!("Expected ')' after assert condition, found {:?}", self.current_token),
                token: self.current_token.clone(),
                line: self.line,
                column: self.column,
            });
        }
        
        self.next_token(); // Skip ')'
        
        // Optional semicolon after assert
        if self.current_token == Token::Semicolon {
            self.next_token(); // Skip semicolon
        }
        
        Ok(Node::Assert {
            condition: Box::new(condition),
            message,
        })
    }
    
    fn parse_if_statement(&mut self) -> Result<Node, ParseError> {
        self.next_token(); // Skip 'if'
        
        // Handle both if (condition) and if condition forms
        let condition = if self.current_token == Token::LeftParen {
            self.next_token(); // Skip '('
            
            let expr = match self.parse_expression(0) {
                Some(expr) => expr,
                None => {
                    return Err(ParseError {
                        message: format!("Expected condition expression after 'if ('"),
                        token: self.current_token.clone(),
                        line: self.line,
                        column: self.column,
                    });
                }
            };
            
            if self.current_token != Token::RightParen {
                return Err(ParseError {
                    message: format!("Expected ')' after if condition, found {:?}", self.current_token),
                    token: self.current_token.clone(),
                    line: self.line,
                    column: self.column,
                });
            }
            
            self.next_token(); // Skip ')'
            expr
        } else {
            // Form without parentheses
            match self.parse_expression(0) {
                Some(expr) => expr,
                None => {
                    return Err(ParseError {
                        message: format!("Expected condition expression after 'if'"),
                        token: self.current_token.clone(),
                        line: self.line,
                        column: self.column,
                    });
                }
            }
        };
        
        if self.current_token != Token::LeftBrace {
            return Err(ParseError {
                message: format!("Expected '{{' after if condition, found {:?}", self.current_token),
                token: self.current_token.clone(),
                line: self.line,
                column: self.column,
            });
        }
        
        let consequence = self.parse_block_statement()?;
        
        let alternative = if self.peek_token == Token::Else {
            self.next_token(); // Move to 'else'
            self.next_token(); // Skip 'else'
            
            if self.current_token != Token::LeftBrace {
                return Err(ParseError {
                    message: format!("Expected '{{' after 'else', found {:?}", self.current_token),
                    token: self.current_token.clone(),
                    line: self.line,
                    column: self.column,
                });
            }
            
            Some(Box::new(self.parse_block_statement()?))
        } else {
            None
        };
        
        Ok(Node::IfStatement {
            condition: Box::new(condition),
            consequence: Box::new(consequence),
            alternative,
        })
    }
    
    fn parse_expression_statement(&mut self) -> Result<Node, ParseError> {
        let expr = match self.parse_expression(0) {
            Some(expr) => expr,
            None => {
                return Err(ParseError {
                    message: format!("Expected expression, found {:?}", self.current_token),
                    token: self.current_token.clone(),
                    line: self.line,
                    column: self.column,
                });
            }
        };
        
        if self.current_token == Token::Semicolon {
            self.next_token(); // Skip semicolon
        }
        
        Ok(Node::ExpressionStatement(Box::new(expr)))
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
                    self.errors.push(ParseError {
                        message: format!("Expected ')', found {:?}", self.current_token),
                        token: self.current_token.clone(),
                        line: self.line,
                        column: self.column,
                    });
                    return None;
                }
                
                self.next_token(); // Skip ')'
                expr
            },
            _ => None,
        };
        
        // No prefix expression could be parsed
        if left_expr.is_none() {
            self.errors.push(ParseError {
                message: format!("No prefix parse function for {:?}", self.current_token),
                token: self.current_token.clone(),
                line: self.line,
                column: self.column,
            });
            return None;
        }
        
        // Parse infix expressions
        while self.peek_token != Token::Semicolon && precedence < self.peek_precedence() {
            match &self.peek_token {
                Token::Plus | Token::Minus | Token::Multiply | Token::Divide |
                Token::Equal | Token::NotEqual | Token::Less | Token::Greater |
                Token::LessEqual | Token::GreaterEqual => {
                    self.next_token();
                    left_expr = self.parse_infix_expression(left_expr.unwrap());
                },
                Token::LeftParen => {
                    self.next_token();
                    left_expr = self.parse_call_expression(left_expr.unwrap());
                },
                _ => break,
            };
            
            if left_expr.is_none() {
                break;
            }
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
            _ => {
                self.errors.push(ParseError {
                    message: format!("Invalid operator: {:?}", self.current_token),
                    token: self.current_token.clone(),
                    line: self.line,
                    column: self.column,
                });
                return None;
            }
        };
        
        let precedence = self.current_precedence();
        self.next_token();
        
        let right = match self.parse_expression(precedence) {
            Some(expr) => expr,
            None => {
                self.errors.push(ParseError {
                    message: format!("Expected right-hand expression after {:?}", operator),
                    token: self.current_token.clone(),
                    line: self.line,
                    column: self.column,
                });
                return None;
            }
        };
        
        Some(Node::InfixExpression {
            left: Box::new(left),
            operator,
            right: Box::new(right),
        })
    }
    
    fn parse_call_expression(&mut self, function: Node) -> Option<Node> {
        let arguments = self.parse_call_arguments()?;
        
        Some(Node::CallExpression {
            function: Box::new(function),
            arguments,
        })
    }
    
    fn parse_call_arguments(&mut self) -> Option<Vec<Node>> {
        let mut args = Vec::new();
        
        if self.peek_token == Token::RightParen {
            self.next_token();
            return Some(args);
        }
        
        self.next_token();
        
        match self.parse_expression(0) {
            Some(expr) => args.push(expr),
            None => {
                self.errors.push(ParseError {
                    message: "Expected expression in function arguments".to_string(),
                    token: self.current_token.clone(),
                    line: self.line,
                    column: self.column,
                });
                return None;
            }
        }
        
        while self.peek_token == Token::Comma {
            self.next_token(); // Skip current
            self.next_token(); // Skip comma
            
            match self.parse_expression(0) {
                Some(expr) => args.push(expr),
                None => {
                    self.errors.push(ParseError {
                        message: "Expected expression after comma in function arguments".to_string(),
                        token: self.current_token.clone(),
                        line: self.line,
                        column: self.column,
                    });
                    return None;
                }
            }
        }
        
        if self.peek_token != Token::RightParen {
            self.errors.push(ParseError {
                message: format!("Expected ')' after arguments, found {:?}", self.peek_token),
                token: self.peek_token.clone(),
                line: self.line,
                column: self.column,
            });
            return None;
        }
        
        self.next_token(); // Skip to ')'
        
        Some(args)
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
    
    // Other parsing methods would be implemented similarly with error recovery
}

// Node types
#[derive(Debug, Clone)]
pub enum Node {
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
    Void,
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
