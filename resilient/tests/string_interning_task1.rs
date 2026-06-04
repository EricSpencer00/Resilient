//! RES-2612 Task 1: String interning module tests.
//!
//! Tests for the core string interning functionality:
//! - Basic interning (same string returns same ID)
//! - Different strings get different IDs
//! - Lookup by ID
//! - Pool reset

#[cfg(test)]
mod tests {
    use resilient::string_interning::{
        all_interned_strings, get_interned_string, intern_string, reset_interning_pool,
    };
    use std::sync::Mutex;

    // Serialize test access to the global pool to prevent race conditions
    static POOL_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_basic_interning_same_string_returns_same_id() {
        let _lock = POOL_LOCK.lock().unwrap();
        reset_interning_pool();

        let id1 = intern_string("hello".to_string());
        let id2 = intern_string("hello".to_string());

        assert_eq!(id1, id2, "Same string should return same ID");
    }

    #[test]
    fn test_different_strings_get_different_ids() {
        let _lock = POOL_LOCK.lock().unwrap();
        reset_interning_pool();

        let id1 = intern_string("hello".to_string());
        let id2 = intern_string("world".to_string());

        assert_ne!(id1, id2, "Different strings should get different IDs");
    }

    #[test]
    fn test_lookup_by_id() {
        let _lock = POOL_LOCK.lock().unwrap();
        reset_interning_pool();

        let original = "test_string".to_string();
        let id = intern_string(original.clone());

        let retrieved = get_interned_string(id);
        assert_eq!(
            retrieved,
            Some(original),
            "Should retrieve the correct string by ID"
        );
    }

    #[test]
    fn test_lookup_nonexistent_id_returns_none() {
        let _lock = POOL_LOCK.lock().unwrap();
        reset_interning_pool();

        let retrieved = get_interned_string(9999);
        assert_eq!(retrieved, None, "Nonexistent ID should return None");
    }

    #[test]
    fn test_pool_reset() {
        let _lock = POOL_LOCK.lock().unwrap();
        reset_interning_pool();

        let id1 = intern_string("first".to_string());
        assert_eq!(id1, 0, "First string should have ID 0");

        reset_interning_pool();

        let id2 = intern_string("first".to_string());
        assert_eq!(id2, 0, "After reset, first string should again have ID 0");
    }

    #[test]
    fn test_all_interned_strings() {
        let _lock = POOL_LOCK.lock().unwrap();
        reset_interning_pool();

        let _id1 = intern_string("apple".to_string());
        let _id2 = intern_string("banana".to_string());
        let _id3 = intern_string("apple".to_string()); // Duplicate, should not add

        let all = all_interned_strings();
        assert_eq!(all.len(), 2, "Should have 2 unique strings");

        // Check that both strings are present
        let strings: Vec<String> = all.iter().map(|(_, s)| s.clone()).collect();
        assert!(strings.contains(&"apple".to_string()));
        assert!(strings.contains(&"banana".to_string()));
    }

    #[test]
    fn test_interning_sequential_ids() {
        let _lock = POOL_LOCK.lock().unwrap();
        reset_interning_pool();

        let id1 = intern_string("str1".to_string());
        let id2 = intern_string("str2".to_string());
        let id3 = intern_string("str3".to_string());

        assert_eq!(id1, 0, "First string should have ID 0");
        assert_eq!(id2, 1, "Second string should have ID 1");
        assert_eq!(id3, 2, "Third string should have ID 2");
    }
}
