// Type checker module for Resilient language
use std::collections::HashMap;
use crate::{Node, Pattern};

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
    /// RES-053: Array — element type not tracked at MVP (typed arrays
    /// land with RES-055 / generics).
    Array,
    /// RES-053: Result<T, E> — payload types not tracked at MVP.
    Result,
    /// RES-053: user-defined record by name. Field types looked up
    /// against the struct table when G7 goes deeper.
    Struct(String),
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
            Type::Array => write!(f, "array"),
            Type::Result => write!(f, "Result"),
            Type::Struct(n) => write!(f, "{}", n),
            Type::Void => write!(f, "void"),
            Type::Any => write!(f, "any"),
        }
    }
}

/// RES-053: Two types are compatible if they're equal or if either is
/// Any. Used everywhere we need "same type, or we don't know yet."
fn compatible(a: &Type, b: &Type) -> bool {
    a == b || matches!(a, Type::Any) || matches!(b, Type::Any)
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

        // Built-in function signatures. Any-typed parameters keep the
        // type checker permissive for heterogeneous inputs until real
        // generics arrive (RES-055).
        let fn_any_to_void = || Type::Function {
            params: vec![Type::Any],
            return_type: Box::new(Type::Void),
        };
        let fn_any_any_to_any = || Type::Function {
            params: vec![Type::Any, Type::Any],
            return_type: Box::new(Type::Any),
        };
        let fn_any_to_any = || Type::Function {
            params: vec![Type::Any],
            return_type: Box::new(Type::Any),
        };
        let fn_any_to_result = || Type::Function {
            params: vec![Type::Any],
            return_type: Box::new(Type::Result),
        };
        let fn_result_to_bool = || Type::Function {
            params: vec![Type::Result],
            return_type: Box::new(Type::Bool),
        };
        let fn_result_to_any = || Type::Function {
            params: vec![Type::Result],
            return_type: Box::new(Type::Any),
        };

        // I/O
        env.set("println".to_string(), fn_any_to_void());
        env.set("print".to_string(), fn_any_to_void());

        // Math (single-arg — int/float passed as Any)
        env.set("abs".to_string(), fn_any_to_any());
        env.set("sqrt".to_string(), fn_any_to_any());
        env.set("floor".to_string(), fn_any_to_any());
        env.set("ceil".to_string(), fn_any_to_any());
        env.set("min".to_string(), fn_any_any_to_any());
        env.set("max".to_string(), fn_any_any_to_any());
        env.set("pow".to_string(), fn_any_any_to_any());

        // len: any -> int
        env.set(
            "len".to_string(),
            Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Int),
            },
        );

        // Array builtins: any -> array / (array,int,int) -> array
        env.set("push".to_string(), Type::Function {
            params: vec![Type::Array, Type::Any],
            return_type: Box::new(Type::Array),
        });
        env.set("pop".to_string(), Type::Function {
            params: vec![Type::Array],
            return_type: Box::new(Type::Array),
        });
        env.set("slice".to_string(), Type::Function {
            params: vec![Type::Array, Type::Int, Type::Int],
            return_type: Box::new(Type::Array),
        });

        // String builtins
        env.set("split".to_string(), Type::Function {
            params: vec![Type::String, Type::String],
            return_type: Box::new(Type::Array),
        });
        env.set("trim".to_string(), Type::Function {
            params: vec![Type::String],
            return_type: Box::new(Type::String),
        });
        env.set("contains".to_string(), Type::Function {
            params: vec![Type::String, Type::String],
            return_type: Box::new(Type::Bool),
        });
        env.set("to_upper".to_string(), Type::Function {
            params: vec![Type::String],
            return_type: Box::new(Type::String),
        });
        env.set("to_lower".to_string(), Type::Function {
            params: vec![Type::String],
            return_type: Box::new(Type::String),
        });

        // Result builtins
        env.set("Ok".to_string(), fn_any_to_result());
        env.set("Err".to_string(), fn_any_to_result());
        env.set("is_ok".to_string(), fn_result_to_bool());
        env.set("is_err".to_string(), fn_result_to_bool());
        env.set("unwrap".to_string(), fn_result_to_any());
        env.set("unwrap_err".to_string(), fn_result_to_any());

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
            
            Node::Function { name, parameters, body, return_type: declared_rt, .. } => {
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
                let body_type = self.check_node(body)?;

                // Restore original environment
                std::mem::swap(&mut self.env, &mut function_env);

                // RES-053: enforce declared return type against body.
                let effective_rt = if let Some(rt_name) = declared_rt {
                    let declared = self.parse_type_name(rt_name)?;
                    if !compatible(&declared, &body_type) {
                        return Err(format!(
                            "fn {}: return type mismatch — declared {}, body produces {}",
                            name, declared, body_type
                        ));
                    }
                    declared
                } else {
                    body_type
                };

                // Register function in current environment
                let func_type = Type::Function {
                    params: param_types,
                    return_type: Box::new(effective_rt.clone()),
                };

                self.env.set(name.clone(), func_type);

                Ok(effective_rt)
            },
            
            Node::LiveBlock { body, .. } => {
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
            
            Node::LetStatement { name, value, type_annot } => {
                let value_type = self.check_node(value)?;
                // RES-053: enforce `let x: T = value` — reject if value's
                // type isn't compatible with the declared annotation.
                let bind_type = if let Some(ty_name) = type_annot {
                    let declared = self.parse_type_name(ty_name)?;
                    if !compatible(&declared, &value_type) {
                        return Err(format!(
                            "let {}: {} — value has type {}",
                            name, declared, value_type
                        ));
                    }
                    declared
                } else {
                    value_type
                };
                self.env.set(name.clone(), bind_type);
                Ok(Type::Void)
            },

            Node::ArrayLiteral(items) => {
                for item in items {
                    let _ = self.check_node(item)?;
                }
                Ok(Type::Array)
            },

            Node::TryExpression(inner) => {
                let inner_type = self.check_node(inner)?;
                // `?` expects a Result and unwraps to Any at MVP (we
                // don't track Ok's payload type yet).
                if !compatible(&inner_type, &Type::Result) {
                    return Err(format!(
                        "? operator expects a Result, got {}",
                        inner_type
                    ));
                }
                Ok(Type::Any)
            },

            Node::FunctionLiteral { parameters, body, .. } => {
                // Evaluate the body's type in a child env with params
                // bound, just like named Function.
                let mut param_types = Vec::new();
                let mut fn_env = TypeEnvironment::new_enclosed(self.env.clone());
                for (tname, pname) in parameters {
                    let ty = self.parse_type_name(tname)?;
                    param_types.push(ty.clone());
                    fn_env.set(pname.clone(), ty);
                }
                std::mem::swap(&mut self.env, &mut fn_env);
                let body_type = self.check_node(body)?;
                std::mem::swap(&mut self.env, &mut fn_env);
                Ok(Type::Function {
                    params: param_types,
                    return_type: Box::new(body_type),
                })
            },

            Node::Match { scrutinee, arms } => {
                let scrutinee_type = self.check_node(scrutinee)?;
                for (_, body) in arms {
                    let _ = self.check_node(body)?;
                }

                // RES-054: exhaustiveness check.
                // Any wildcard or identifier pattern makes the match
                // trivially exhaustive.
                let has_default = arms.iter().any(|(p, _)| {
                    matches!(p, Pattern::Wildcard | Pattern::Identifier(_))
                });

                if !has_default {
                    match scrutinee_type {
                        // Bool is the only finite-domain scalar; require
                        // coverage of both true and false.
                        Type::Bool => {
                            let has_true = arms.iter().any(|(p, _)| {
                                matches!(p, Pattern::Literal(Node::BooleanLiteral(true)))
                            });
                            let has_false = arms.iter().any(|(p, _)| {
                                matches!(p, Pattern::Literal(Node::BooleanLiteral(false)))
                            });
                            if !(has_true && has_false) {
                                return Err(format!(
                                    "Non-exhaustive match on bool: {}{}{}",
                                    if has_true { "" } else { "missing `true` arm" },
                                    if !has_true && !has_false { "; " } else { "" },
                                    if has_false { "" } else { "missing `false` arm" },
                                ));
                            }
                        }
                        // For any other scrutinee type — int, float,
                        // string, struct, Result, etc. — a wildcard /
                        // identifier arm is required. The domain is
                        // effectively open.
                        Type::Any => {
                            // Scrutinee type unknown → accept the match
                            // rather than force a wildcard. Real
                            // exhaustiveness for user types lands with
                            // G7's struct-decl table.
                        }
                        other => {
                            return Err(format!(
                                "Non-exhaustive match on {}: add a wildcard `_` or identifier arm to handle unmatched values",
                                other
                            ));
                        }
                    }
                }

                Ok(Type::Any)
            },

            Node::StructDecl { .. } => Ok(Type::Void),

            Node::StructLiteral { name, fields } => {
                for (_, e) in fields {
                    let _ = self.check_node(e)?;
                }
                Ok(Type::Struct(name.clone()))
            },

            Node::FieldAccess { target, .. } => {
                let _ = self.check_node(target)?;
                // Field types not tracked at MVP.
                Ok(Type::Any)
            },

            Node::FieldAssignment { target, value, .. } => {
                let _ = self.check_node(target)?;
                let _ = self.check_node(value)?;
                Ok(Type::Void)
            },

            Node::IndexExpression { target, index } => {
                let _ = self.check_node(target)?;
                let _ = self.check_node(index)?;
                // Element type not tracked at MVP.
                Ok(Type::Any)
            },

            Node::IndexAssignment { target, index, value } => {
                let _ = self.check_node(target)?;
                let _ = self.check_node(index)?;
                let _ = self.check_node(value)?;
                Ok(Type::Void)
            },

            Node::ForInStatement { iterable, body, .. } => {
                let _ = self.check_node(iterable)?;
                let _ = self.check_node(body)?;
                Ok(Type::Void)
            },

            Node::WhileStatement { condition, body } => {
                let _ = self.check_node(condition)?;
                let _ = self.check_node(body)?;
                Ok(Type::Void)
            },

            Node::StaticLet { name, value } => {
                let value_type = self.check_node(value)?;
                self.env.set(name.clone(), value_type);
                Ok(Type::Void)
            },

            Node::Assignment { value, .. } => {
                // Assignment is allowed at runtime; static type check
                // just ensures the RHS type-checks. Per-name existence
                // is enforced by the interpreter. Real type-equality
                // checks land with a proper typechecker (G7).
                let _ = self.check_node(value)?;
                Ok(Type::Void)
            },
            
            Node::ReturnStatement { value } => {
                // Bare `return;` has type Void; otherwise pass through
                // the type of the returned value.
                match value {
                    Some(expr) => self.check_node(expr),
                    None => Ok(Type::Void),
                }
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

                let is_numeric = |t: &Type| matches!(t, Type::Int | Type::Float | Type::Any);
                let is_bool = |t: &Type| matches!(t, Type::Bool | Type::Any);

                match operator.as_str() {
                    "+" => {
                        // String-plus-primitive coercion (RES-008): if
                        // either side is a string, the result is a string.
                        if left_type == Type::String || right_type == Type::String {
                            return Ok(Type::String);
                        }
                        // Array concat.
                        if compatible(&left_type, &Type::Array)
                            && compatible(&right_type, &Type::Array)
                        {
                            return Ok(Type::Array);
                        }
                        if is_numeric(&left_type) && is_numeric(&right_type) {
                            if left_type == Type::Float || right_type == Type::Float {
                                Ok(Type::Float)
                            } else {
                                Ok(Type::Int)
                            }
                        } else {
                            Err(format!(
                                "Cannot apply '+' to {} and {}",
                                left_type, right_type
                            ))
                        }
                    }
                    "-" | "*" | "/" | "%" => {
                        if is_numeric(&left_type) && is_numeric(&right_type) {
                            if left_type == Type::Float || right_type == Type::Float {
                                Ok(Type::Float)
                            } else {
                                Ok(Type::Int)
                            }
                        } else {
                            Err(format!(
                                "Cannot apply '{}' to {} and {}",
                                operator, left_type, right_type
                            ))
                        }
                    }
                    "&" | "|" | "^" | "<<" | ">>" => {
                        // Bitwise operators are int-only.
                        if compatible(&left_type, &Type::Int)
                            && compatible(&right_type, &Type::Int)
                        {
                            Ok(Type::Int)
                        } else {
                            Err(format!(
                                "Bitwise '{}' requires int operands, got {} and {}",
                                operator, left_type, right_type
                            ))
                        }
                    }
                    "&&" | "||" => {
                        if is_bool(&left_type) && is_bool(&right_type) {
                            Ok(Type::Bool)
                        } else {
                            Err(format!(
                                "Logical '{}' requires bool operands, got {} and {}",
                                operator, left_type, right_type
                            ))
                        }
                    }
                    "==" | "!=" | "<" | ">" | "<=" | ">=" => {
                        if compatible(&left_type, &right_type) {
                            Ok(Type::Bool)
                        } else {
                            Err(format!(
                                "Cannot compare {} and {}",
                                left_type, right_type
                            ))
                        }
                    }
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
            "Result" => Ok(Type::Result),
            "array" => Ok(Type::Array),
            "" => Ok(Type::Any), // Empty type name means "any" for now
            // RES-053: any other identifier is assumed to be a
            // user-defined struct. G7 will register struct decls and
            // reject unknown type names, but at MVP we're permissive.
            other => Ok(Type::Struct(other.to_string())),
        }
    }
}
