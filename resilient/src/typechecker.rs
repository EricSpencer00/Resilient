// Type checker module for Resilient language
use std::collections::HashMap;
use crate::Node;

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Int,
    Float,
    String,
    Bool,
    Function {
        params: Vec<Type>,
        return_type: Box<Type>,
    },
    Void,
    Any, // Used for untyped variables during inference
}

impl std::fmt::Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Type::Int => write!(f, "int"),
            Type::Float => write!(f, "float"),
            Type::String => write!(f, "string"),
            Type::Bool => write!(f, "bool"),
            Type::Function { params, return_type } => {
                write!(f, "fn(")?;
                for (i, param) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", param)?;
                }
                write!(f, ") -> {}", return_type)
            },
            Type::Void => write!(f, "void"),
            Type::Any => write!(f, "any"),
        }
    }
}

// Environment for storing type information
#[derive(Debug, Clone)]
pub struct TypeEnvironment {
    store: HashMap<String, Type>,
    outer: Option<Box<TypeEnvironment>>,
}

impl TypeEnvironment {
    pub fn new() -> Self {
        TypeEnvironment {
            store: HashMap::new(),
            outer: None,
        }
    }
    
    pub fn new_enclosed(outer: TypeEnvironment) -> Self {
        TypeEnvironment {
            store: HashMap::new(),
            outer: Some(Box::new(outer)),
        }
    }
    
    pub fn get(&self, name: &str) -> Option<Type> {
        match self.store.get(name) {
            Some(typ) => Some(typ.clone()),
            None => {
                if let Some(outer) = &self.outer {
                    outer.get(name)
                } else {
                    None
                }
            }
        }
    }
    
    pub fn set(&mut self, name: String, typ: Type) {
        self.store.insert(name, typ);
    }
}

// Type checker for verifying type correctness
pub struct TypeChecker {
    env: TypeEnvironment,
}

impl TypeChecker {
    pub fn new() -> Self {
        let mut env = TypeEnvironment::new();
        
        // Add built-in functions
        env.set("println".to_string(), Type::Function { 
            params: vec![Type::Any], 
            return_type: Box::new(Type::Void) 
        });
        
        TypeChecker { env }
    }
    
    pub fn check_program(&mut self, program: &Node) -> Result<Type, String> {
        match program {
            Node::Program(statements) => {
                let mut result_type = Type::Void;
                
                for stmt in statements {
                    result_type = self.check_node(stmt)?;
                }
                
                Ok(result_type)
            },
            _ => Err("Expected program node".to_string()),
        }
    }
    
    pub fn check_node(&mut self, node: &Node) -> Result<Type, String> {
        match node {
            Node::Program(_statements) => self.check_program(node),
            
            Node::Function { name, parameters, body } => {
                let mut param_types = Vec::new();
                
                // Create a new enclosed environment for function body
                let mut function_env = TypeEnvironment::new_enclosed(self.env.clone());
                
                // Add parameter types to environment
                for (param_type_name, param_name) in parameters {
                    let param_type = self.parse_type_name(param_type_name)?;
                    param_types.push(param_type.clone());
                    function_env.set(param_name.clone(), param_type);
                }
                
                // Temporarily swap environments
                std::mem::swap(&mut self.env, &mut function_env);
                
                // Check function body
                let return_type = self.check_node(body)?;
                
                // Restore original environment
                std::mem::swap(&mut self.env, &mut function_env);
                
                // Register function in current environment
                let func_type = Type::Function {
                    params: param_types,
                    return_type: Box::new(return_type.clone()),
                };
                
                self.env.set(name.clone(), func_type);
                
                Ok(return_type)
            },
            
            Node::LiveBlock { body } => {
                // Live blocks preserve the type of their body
                self.check_node(body)
            },
            
            Node::Assert { condition, message } => {
                // Condition must be a boolean expression
                let condition_type = self.check_node(condition)?;
                if condition_type != Type::Bool && condition_type != Type::Any {
                    return Err(format!("Assert condition must be a boolean, got {}", condition_type));
                }
                
                // Message, if present, should be a string
                if let Some(msg) = message {
                    let msg_type = self.check_node(msg)?;
                    if msg_type != Type::String && msg_type != Type::Any {
                        return Err(format!("Assert message must be a string, got {}", msg_type));
                    }
                }
                
                Ok(Type::Void)
            },
            
            Node::Block(statements) => {
                let mut result_type = Type::Void;
                
                // Create a new enclosed environment for block
                let mut block_env = TypeEnvironment::new_enclosed(self.env.clone());
                std::mem::swap(&mut self.env, &mut block_env);
                
                for stmt in statements {
                    result_type = self.check_node(stmt)?;
                }
                
                // Restore original environment
                std::mem::swap(&mut self.env, &mut block_env);
                
                Ok(result_type)
            },
            
            Node::LetStatement { name, value } => {
                let value_type = self.check_node(value)?;
                self.env.set(name.clone(), value_type);
                Ok(Type::Void)
            },
            
            Node::ReturnStatement { value } => {
                // Simply pass through the type of the returned value
                self.check_node(value)
            },
            
            Node::IfStatement { condition, consequence, alternative } => {
                let condition_type = self.check_node(condition)?;
                if condition_type != Type::Bool && condition_type != Type::Any {
                    return Err(format!("If condition must be a boolean, got {}", condition_type));
                }
                
                let consequence_type = self.check_node(consequence)?;
                
                if let Some(alt) = alternative {
                    let alternative_type = self.check_node(alt)?;
                    
                    // Both branches should have compatible types
                    if consequence_type != alternative_type && 
                       consequence_type != Type::Any && 
                       alternative_type != Type::Any {
                        return Err(format!("If branches have incompatible types: {} and {}", 
                                          consequence_type, alternative_type));
                    }
                }
                
                Ok(consequence_type)
            },
            
            Node::ExpressionStatement(expr) => {
                self.check_node(expr)
            },
            
            Node::Identifier(name) => {
                match self.env.get(name) {
                    Some(typ) => Ok(typ),
                    None => Err(format!("Undefined variable: {}", name)),
                }
            },
            
            Node::IntegerLiteral(_) => Ok(Type::Int),
            Node::FloatLiteral(_) => Ok(Type::Float),
            Node::StringLiteral(_) => Ok(Type::String),
            Node::BooleanLiteral(_) => Ok(Type::Bool),
            
            Node::PrefixExpression { operator, right } => {
                let right_type = self.check_node(right)?;
                
                match operator.as_str() {
                    "!" => {
                        if right_type != Type::Bool && right_type != Type::Any {
                            return Err(format!("Cannot apply '!' to {}", right_type));
                        }
                        Ok(Type::Bool)
                    },
                    "-" => {
                        if right_type != Type::Int && right_type != Type::Float && right_type != Type::Any {
                            return Err(format!("Cannot apply '-' to {}", right_type));
                        }
                        Ok(right_type)
                    },
                    _ => Err(format!("Unknown prefix operator: {}", operator)),
                }
            },
            
            Node::InfixExpression { left, operator, right } => {
                let left_type = self.check_node(left)?;
                let right_type = self.check_node(right)?;
                
                match operator.as_str() {
                    "+" | "-" | "*" | "/" => {
                        // Numeric operations
                        if (left_type == Type::Int || left_type == Type::Float || left_type == Type::Any) &&
                           (right_type == Type::Int || right_type == Type::Float || right_type == Type::Any) {
                            // If either operand is a float, the result is a float
                            if left_type == Type::Float || right_type == Type::Float {
                                Ok(Type::Float)
                            } else {
                                Ok(Type::Int)
                            }
                        } else if operator == "+" && (left_type == Type::String || right_type == Type::String) {
                            // String concatenation
                            Ok(Type::String)
                        } else {
                            Err(format!("Cannot apply '{}' to {} and {}", operator, left_type, right_type))
                        }
                    },
                    "==" | "!=" | "<" | ">" | "<=" | ">=" => {
                        // Comparison operations always return a boolean
                        if (left_type == right_type) || left_type == Type::Any || right_type == Type::Any {
                            Ok(Type::Bool)
                        } else {
                            Err(format!("Cannot compare {} and {}", left_type, right_type))
                        }
                    },
                    _ => Err(format!("Unknown infix operator: {}", operator)),
                }
            },
            
            Node::CallExpression { function, arguments } => {
                let func_type = self.check_node(function)?;
                
                match func_type {
                    Type::Function { params, return_type } => {
                        // Check argument count
                        if arguments.len() != params.len() {
                            return Err(format!("Expected {} arguments, got {}", 
                                              params.len(), arguments.len()));
                        }
                        
                        // Check each argument type
                        for (i, (arg, param_type)) in arguments.iter().zip(params.iter()).enumerate() {
                            let arg_type = self.check_node(arg)?;
                            if arg_type != *param_type && *param_type != Type::Any && arg_type != Type::Any {
                                return Err(format!("Type mismatch in argument {}: expected {}, got {}", 
                                                  i + 1, param_type, arg_type));
                            }
                        }
                        
                        Ok(*return_type)
                    },
                    Type::Any => {
                        // When calling a function of unknown type, we assume it works
                        // This is to support built-in functions that might not be fully typed
                        Ok(Type::Any)
                    },
                    _ => Err(format!("Cannot call non-function type: {}", func_type)),
                }
            },
        }
    }
    
    fn parse_type_name(&self, name: &str) -> Result<Type, String> {
        match name {
            "int" => Ok(Type::Int),
            "float" => Ok(Type::Float),
            "string" => Ok(Type::String),
            "bool" => Ok(Type::Bool),
            "void" => Ok(Type::Void),
            "" => Ok(Type::Any), // Empty type name means "any" for now
            _ => Err(format!("Unknown type: {}", name)),
        }
    }
}
