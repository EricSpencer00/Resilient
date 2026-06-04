//! RES-2612: Compile-time string interning for reduced binary size and O(1) equality.
//!
//! String interning deduplicates identical string literals into a single memory location.
//! This reduces binary bloat and enables pointer-based equality checks.

use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Global string interning pool. Maps normalized strings to unique IDs.
static INTERNING_POOL: OnceLock<parking_lot::Mutex<InterningPool>> = OnceLock::new();

static NEXT_STRING_ID: AtomicUsize = AtomicUsize::new(0);

/// A deduplicated string with a stable numeric ID.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct InternedString {
    /// Unique identifier for this interned string
    pub id: usize,
    /// The actual string content
    pub content: String,
}

impl InternedString {
    /// Get the address-based hash for O(1) equality.
    pub fn ptr_id(&self) -> usize {
        self.id
    }
}

/// The interning pool that manages all interned strings.
pub struct InterningPool {
    /// Maps canonical string content to InternedString entries
    strings: HashMap<String, InternedString>,
    /// Reverse mapping: ID -> InternedString (for ID-based lookup)
    by_id: BTreeMap<usize, InternedString>,
}

impl InterningPool {
    pub fn new() -> Self {
        Self {
            strings: HashMap::new(),
            by_id: BTreeMap::new(),
        }
    }

    /// Intern a string: return existing ID if already interned, else create new.
    pub fn intern(&mut self, content: String) -> InternedString {
        if let Some(existing) = self.strings.get(&content) {
            return existing.clone();
        }

        let id = NEXT_STRING_ID.fetch_add(1, Ordering::SeqCst);
        let interned = InternedString {
            id,
            content: content.clone(),
        };
        self.strings.insert(content, interned.clone());
        self.by_id.insert(id, interned.clone());
        interned
    }

    /// Look up an interned string by ID.
    pub fn get_by_id(&self, id: usize) -> Option<InternedString> {
        self.by_id.get(&id).cloned()
    }

    /// Get all interned strings (for code generation).
    pub fn all_strings(&self) -> Vec<InternedString> {
        self.by_id.values().cloned().collect()
    }

    /// Clear the pool (used in tests/REPL resets).
    pub fn clear(&mut self) {
        self.strings.clear();
        self.by_id.clear();
        NEXT_STRING_ID.store(0, Ordering::SeqCst);
    }
}

impl Default for InterningPool {
    fn default() -> Self {
        Self::new()
    }
}

fn get_pool() -> &'static parking_lot::Mutex<InterningPool> {
    INTERNING_POOL.get_or_init(|| parking_lot::Mutex::new(InterningPool::new()))
}

/// Global entry point: intern a string and return its ID.
pub fn intern_string(content: String) -> usize {
    let mut pool = get_pool().lock();
    pool.intern(content).id
}

/// Look up an interned string by its ID.
pub fn get_interned_string(id: usize) -> Option<String> {
    let pool = get_pool().lock();
    pool.get_by_id(id).map(|s| s.content)
}

/// Collect all interned strings (for codegen).
pub fn all_interned_strings() -> Vec<(usize, String)> {
    let pool = get_pool().lock();
    pool.all_strings()
        .into_iter()
        .map(|s| (s.id, s.content))
        .collect()
}

/// Reset the interning pool (for REPL, tests).
pub fn reset_interning_pool() {
    let mut pool = get_pool().lock();
    pool.clear();
}

/// Check if two strings are equal. Uses O(1) pointer comparison if both are interned.
/// In this implementation, we rely on the fact that interned strings with the same
/// ID will retrieve the same String value from the pool.
pub fn strings_equal(id1: usize, id2: usize) -> bool {
    // If both intern_ids are the same, the strings are definitely equal (O(1)).
    id1 == id2
}

/// Check if a given ID corresponds to an interned string in the pool.
pub fn is_interned(id: usize) -> bool {
    get_interned_string(id).is_some()
}

/// RES-2612 Task 4: Type check interned strings.
/// Validates that all StringInternLiteral nodes have valid intern_ids that
/// map to entries in the interning pool, and that the content matches.
pub(crate) fn check_string_interning(program: &crate::Node) -> Result<(), String> {
    check_node(program)
}

/// Recursively walk the AST and validate all StringInternLiteral nodes.
fn check_node(node: &crate::Node) -> Result<(), String> {
    use crate::Node;

    match node {
        // StringInternLiteral: validate the intern_id is in the pool and content matches
        Node::StringInternLiteral {
            intern_id, content, ..
        } => {
            match get_interned_string(*intern_id) {
                Some(pooled_content) => {
                    if pooled_content != *content {
                        return Err(format!(
                            "String interning mismatch: intern_id {} has content '{}' in pool but '{}' in AST",
                            intern_id, pooled_content, content
                        ));
                    }
                }
                None => {
                    return Err(format!(
                        "Invalid intern_id {} in StringInternLiteral: not found in pool",
                        intern_id
                    ));
                }
            }
            Ok(())
        }

        // Program: check all top-level statements
        Node::Program(stmts) => {
            for stmt in stmts {
                check_node(&stmt.node)?;
            }
            Ok(())
        }

        // Function: check all components of a function definition
        Node::Function {
            body,
            defaults,
            requires,
            ensures,
            ..
        } => {
            check_node(body)?;
            for default_expr in defaults.iter().flatten() {
                check_node(default_expr)?;
            }
            for req in requires {
                check_node(req)?;
            }
            for ens in ensures {
                check_node(ens)?;
            }
            Ok(())
        }

        // Block: check all statements in the block
        Node::Block { stmts, .. } => {
            for stmt in stmts {
                check_node(stmt)?;
            }
            Ok(())
        }

        // IfStatement: check condition, consequence, and optional alternative
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            check_node(condition)?;
            check_node(consequence)?;
            if let Some(alt) = alternative {
                check_node(alt)?;
            }
            Ok(())
        }

        // WhileStatement: check condition and body
        Node::WhileStatement {
            condition, body, ..
        } => {
            check_node(condition)?;
            check_node(body)?;
            Ok(())
        }

        // ForInStatement: check iterable and body
        Node::ForInStatement { iterable, body, .. } => {
            check_node(iterable)?;
            check_node(body)?;
            Ok(())
        }

        // CallExpression: check function and all arguments
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            check_node(function)?;
            for arg in arguments {
                check_node(arg)?;
            }
            Ok(())
        }

        // ReturnStatement: check returned expression if present
        Node::ReturnStatement { value, .. } => {
            if let Some(val) = value {
                check_node(val)?;
            }
            Ok(())
        }

        // LetStatement: check initialization expression
        Node::LetStatement { value, .. } => {
            check_node(value)?;
            Ok(())
        }

        // StaticLet: check initialization expression
        Node::StaticLet { value, .. } => {
            check_node(value)?;
            Ok(())
        }

        // Assignment: check the assigned value
        Node::Assignment { value, .. } => {
            check_node(value)?;
            Ok(())
        }

        // InfixExpression: check both operands
        Node::InfixExpression { left, right, .. } => {
            check_node(left)?;
            check_node(right)?;
            Ok(())
        }

        // PrefixExpression: check the operand
        Node::PrefixExpression { right, .. } => {
            check_node(right)?;
            Ok(())
        }

        // ArrayLiteral: check all items
        Node::ArrayLiteral { items, .. } => {
            for item in items {
                check_node(item)?;
            }
            Ok(())
        }

        // IndexExpression: check target and index
        Node::IndexExpression { target, index, .. } => {
            check_node(target)?;
            check_node(index)?;
            Ok(())
        }

        // StructLiteral: check all field values and base if present
        Node::StructLiteral { fields, base, .. } => {
            for (_, val) in fields {
                check_node(val)?;
            }
            if let Some(b) = base {
                check_node(b)?;
            }
            Ok(())
        }

        // FieldAccess: check the target expression
        Node::FieldAccess { target, .. } => {
            check_node(target)?;
            Ok(())
        }

        // ExpressionStatement: check the expression
        Node::ExpressionStatement { expr, .. } => {
            check_node(expr)?;
            Ok(())
        }

        // TryCatch: check body and handler bodies
        Node::TryCatch { body, handlers, .. } => {
            for stmt in body {
                check_node(stmt)?;
            }
            for (_, handler_stmts) in handlers {
                for stmt in handler_stmts {
                    check_node(stmt)?;
                }
            }
            Ok(())
        }

        // All other node types don't have children or don't need validation
        _ => Ok(()),
    }
}
