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

/// RES-060/061: fold a contract expression down to a concrete boolean.
/// `bindings` maps identifier names to known integer values — used at
/// call sites where the typechecker has constant arguments to
/// substitute for parameters.
///
/// Returns:
///   Some(true)  — provably true (tautology under bindings, discharged)
///   Some(false) — provably false (contradiction, reject)
///   None        — undecidable (leave for runtime check)
///
/// This is the verification core. G9b will swap it for a Z3 query;
/// the return shape (sat/unsat/unknown) stays the same.
fn fold_const_bool(n: &Node, bindings: &HashMap<String, i64>) -> Option<bool> {
    match n {
        Node::BooleanLiteral(b) => Some(*b),
        Node::PrefixExpression { operator, right } if operator == "!" => {
            fold_const_bool(right, bindings).map(|b| !b)
        }
        Node::InfixExpression { left, operator, right } => {
            match operator.as_str() {
                "&&" => match (
                    fold_const_bool(left, bindings),
                    fold_const_bool(right, bindings),
                ) {
                    (Some(a), Some(b)) => Some(a && b),
                    (Some(false), _) | (_, Some(false)) => Some(false),
                    _ => None,
                },
                "||" => match (
                    fold_const_bool(left, bindings),
                    fold_const_bool(right, bindings),
                ) {
                    (Some(a), Some(b)) => Some(a || b),
                    (Some(true), _) | (_, Some(true)) => Some(true),
                    _ => None,
                },
                "==" | "!=" | "<" | ">" | "<=" | ">=" => {
                    let l = fold_const_i64(left, bindings)?;
                    let r = fold_const_i64(right, bindings)?;
                    Some(match operator.as_str() {
                        "==" => l == r,
                        "!=" => l != r,
                        "<" => l < r,
                        ">" => l > r,
                        "<=" => l <= r,
                        ">=" => l >= r,
                        _ => unreachable!(),
                    })
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// RES-064: if the expression is `IDENT == LITERAL` or `LITERAL == IDENT`,
/// extract the assumption as `(name, value)`. Used to push the assumption
/// into const_bindings while checking an `if` consequence.
///
/// This is the first step toward real flow-sensitive verification.
/// Future tickets will extend to inequality bounds, ranges, and the
/// negative branch (else).
fn extract_eq_assumption(cond: &Node) -> Option<(String, i64)> {
    if let Node::InfixExpression { left, operator, right } = cond
        && operator == "=="
    {
        let no_b: HashMap<String, i64> = HashMap::new();
        match (left.as_ref(), right.as_ref()) {
            (Node::Identifier(name), other) => {
                fold_const_i64(other, &no_b).map(|v| (name.clone(), v))
            }
            (other, Node::Identifier(name)) => {
                fold_const_i64(other, &no_b).map(|v| (name.clone(), v))
            }
            _ => None,
        }
    } else {
        None
    }
}

/// Fold an integer-typed expression to a concrete i64 under bindings.
fn fold_const_i64(n: &Node, bindings: &HashMap<String, i64>) -> Option<i64> {
    match n {
        Node::IntegerLiteral(v) => Some(*v),
        Node::Identifier(name) => bindings.get(name).copied(),
        Node::PrefixExpression { operator, right } if operator == "-" => {
            fold_const_i64(right, bindings).map(|v| -v)
        }
        Node::InfixExpression { left, operator, right } => {
            let l = fold_const_i64(left, bindings)?;
            let r = fold_const_i64(right, bindings)?;
            match operator.as_str() {
                "+" => l.checked_add(r),
                "-" => l.checked_sub(r),
                "*" => l.checked_mul(r),
                "/" if r != 0 => l.checked_div(r),
                "%" if r != 0 => l.checked_rem(r),
                _ => None,
            }
        }
        _ => None,
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

/// RES-061: signature-and-contract record stored per top-level fn so
/// the typechecker can fold contracts at constant call sites.
#[derive(Debug, Clone)]
struct ContractInfo {
    parameters: Vec<(String, String)>, // (type_name, param_name)
    requires: Vec<Node>,
    /// Reserved for the symmetric ensures-fold work (post-call result
    /// substitution); not used by the call-site fold today, but kept
    /// in the table so RES-062 can pick up where this leaves off.
    #[allow(dead_code)]
    ensures: Vec<Node>,
}

/// RES-066: counters for the verification audit. Incremented as the
/// typechecker walks the program; read out by `--audit` after the run.
#[derive(Debug, Clone, Default)]
pub struct VerificationStats {
    /// requires clauses with no free variables AND a constant call site,
    /// or pushed assumptions that fold to true.
    pub requires_discharged_at_compile: usize,
    /// requires clauses left for runtime check (couldn't fold).
    pub requires_left_for_runtime: usize,
    /// requires clauses that fold to a tautology (no params used).
    pub requires_tautology: usize,
    /// Total contracted call sites visited.
    pub contracted_call_sites: usize,
}

// Type checker for verifying type correctness
pub struct TypeChecker {
    env: TypeEnvironment,
    /// RES-061: top-level function name → its parameters + contract clauses.
    /// Populated by check_program's first pass; consulted by CallExpression.
    contract_table: HashMap<String, ContractInfo>,
    /// RES-063: identifier → known constant integer value.
    const_bindings: HashMap<String, i64>,
    /// RES-066: verification audit counters.
    pub stats: VerificationStats,
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

        TypeChecker {
            env,
            contract_table: HashMap::new(),
            const_bindings: HashMap::new(),
            stats: VerificationStats::default(),
        }
    }
    
    pub fn check_program(&mut self, program: &Node) -> Result<Type, String> {
        match program {
            Node::Program(statements) => {
                // RES-061: pre-pass to register every top-level Function
                // in the contract table. Mirrors the interpreter's
                // function-hoisting pass so call sites can fold contracts
                // even for forward references.
                for stmt in statements {
                    if let Node::Function {
                        name,
                        parameters,
                        requires,
                        ensures,
                        ..
                    } = stmt
                    {
                        self.contract_table.insert(
                            name.clone(),
                            ContractInfo {
                                parameters: parameters.clone(),
                                requires: requires.clone(),
                                ensures: ensures.clone(),
                            },
                        );
                    }
                }

                let mut result_type = Type::Void;
                for stmt in statements {
                    result_type = self.check_node(stmt)?;
                }
                Ok(result_type)
            }
            _ => Err("Expected program node".to_string()),
        }
    }
    
    pub fn check_node(&mut self, node: &Node) -> Result<Type, String> {
        match node {
            Node::Program(_statements) => self.check_program(node),
            
            Node::Function { name, parameters, body, requires, ensures, return_type: declared_rt } => {
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

                // RES-060: statically fold every requires / ensures
                // clause. A contradiction is a compile-time error; a
                // tautology is discharged; anything else is left for
                // runtime.
                let no_bindings: HashMap<String, i64> = HashMap::new();
                for clause in requires.iter().chain(ensures.iter()) {
                    match fold_const_bool(clause, &no_bindings) {
                        Some(false) => {
                            return Err(format!(
                                "fn {}: contract can never hold (statically false clause)",
                                name
                            ));
                        }
                        Some(true) => {
                            self.stats.requires_tautology += 1;
                        }
                        None => {}
                    }
                }

                // RES-065: push each requires clause's extractable
                // assumption into const_bindings so interior call
                // sites can use them. This is the inter-procedural
                // chaining step.
                let mut pushed_assumptions: Vec<(String, Option<i64>)> = Vec::new();
                for clause in requires {
                    if let Some((aname, av)) = extract_eq_assumption(clause) {
                        let prev = self.const_bindings.get(&aname).copied();
                        self.const_bindings.insert(aname.clone(), av);
                        pushed_assumptions.push((aname, prev));
                    }
                }

                // Check function body
                let body_type = self.check_node(body)?;

                // Restore const_bindings to its pre-body state.
                for (aname, prev) in pushed_assumptions.into_iter().rev() {
                    match prev {
                        Some(v) => { self.const_bindings.insert(aname, v); }
                        None => { self.const_bindings.remove(&aname); }
                    }
                }

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
                // RES-063: if the RHS is a foldable integer constant,
                // remember the value so future call sites can use it.
                // Otherwise REMOVE any prior binding (shadowing kills
                // the old constant).
                let no_b: HashMap<String, i64> = HashMap::new();
                if let Some(v) = fold_const_i64(value, &no_b)
                    .or_else(|| fold_const_i64(value, &self.const_bindings))
                {
                    self.const_bindings.insert(name.clone(), v);
                } else {
                    self.const_bindings.remove(name);
                }
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
                // RES-063: static lets are mutable across calls, so
                // they're never safe to treat as compile-time constants
                // for verification.
                self.const_bindings.remove(name);
                Ok(Type::Void)
            },

            Node::Assignment { name, value } => {
                let _ = self.check_node(value)?;
                // RES-063: any reassignment kills const-tracking. We
                // could try to re-track if RHS is foldable, but
                // mid-function mutation is rare and the conservative
                // choice keeps the verifier sound.
                self.const_bindings.remove(name);
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

                // RES-064: if the condition is `IDENT == LITERAL` (or
                // `LITERAL == IDENT`), assume that equality inside the
                // consequence by pushing the binding. Restore on exit
                // so the assumption doesn't leak.
                let assumption = extract_eq_assumption(condition);
                let saved = if let Some((ref name, value)) = assumption {
                    let prev = self.const_bindings.get(name).copied();
                    self.const_bindings.insert(name.clone(), value);
                    Some((name.clone(), prev))
                } else {
                    None
                };

                let consequence_type = self.check_node(consequence)?;

                // Restore.
                if let Some((name, prev)) = saved {
                    match prev {
                        Some(v) => { self.const_bindings.insert(name, v); }
                        None => { self.const_bindings.remove(&name); }
                    }
                }

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

                // RES-061 + RES-063: if the callee is a known top-level
                // fn with contracts, fold each requires clause with the
                // call's arguments substituted for parameters. Arguments
                // can be literal expressions OR identifiers that resolve
                // to a constant via const_bindings.
                if let Node::Identifier(callee_name) = function.as_ref()
                    && let Some(info) = self.contract_table.get(callee_name).cloned()
                {
                    if !info.requires.is_empty() {
                        self.stats.contracted_call_sites += 1;
                    }
                    let mut bindings: HashMap<String, i64> = HashMap::new();
                    for ((_ty, pname), arg) in info.parameters.iter().zip(arguments.iter()) {
                        if let Some(v) = fold_const_i64(arg, &self.const_bindings) {
                            bindings.insert(pname.clone(), v);
                        }
                    }
                    for clause in &info.requires {
                        match fold_const_bool(clause, &bindings) {
                            Some(false) => {
                                return Err(format!(
                                    "Contract violation: call to fn {} would fail `requires` clause at compile time",
                                    callee_name
                                ));
                            }
                            Some(true) => {
                                self.stats.requires_discharged_at_compile += 1;
                            }
                            None => {
                                self.stats.requires_left_for_runtime += 1;
                            }
                        }
                    }
                }

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
