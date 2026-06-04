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
