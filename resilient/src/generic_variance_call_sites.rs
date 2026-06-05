//! RES-2616: variance-aware subtype checks at generic call sites.

#[cfg(test)]
mod tests {
    use crate::parse;
    use crate::typechecker::TypeChecker;

    fn check_ok(src: &str) {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program(&prog)
            .unwrap_or_else(|e| panic!("unexpected type error: {e}"));
    }

    fn check_err(src: &str) -> String {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program(&prog)
            .expect_err("expected type error")
    }

    #[test]
    fn covariant_generic_accepts_wider_trait_argument() {
        check_ok(
            "trait Animal { fn tag(self) -> string; }\n\
             trait Cat extends Animal { fn meow(self) -> string; }\n\
             fn use_twice<T>(fn(T) -> void first, fn(T) -> void second) -> void { }\n\
             fn take_cat(Cat c) -> void { }\n\
             fn take_animal(Animal a) -> void { }\n\
             use_twice(take_cat, take_animal);\n",
        );
    }

    #[test]
    fn contravariant_generic_accepts_narrower_return_trait() {
        check_ok(
            "trait Animal { fn tag(self) -> string; }\n\
             trait Cat extends Animal { fn meow(self) -> string; }\n\
             struct Dog { int id }\n\
             struct Kitty { int id }\n\
             impl Animal for Dog { fn tag(self) -> string { return \"dog\"; } }\n\
             impl Animal for Kitty { fn tag(self) -> string { return \"cat\"; } }\n\
             impl Cat for Kitty { fn meow(self) -> string { return \"meow\"; } }\n\
             fn merge<T>(fn() -> T first, fn() -> T second) -> void { }\n\
             fn make_animal() -> Animal { return new Dog { id: 1 }; }\n\
             fn make_cat() -> Cat { return new Kitty { id: 2 }; }\n\
             merge(make_animal, make_cat);\n",
        );
    }

    #[test]
    fn invariant_generic_rejects_variance_mismatch() {
        let err = check_err(
            "trait Animal { fn tag(self) -> string; }\n\
             trait Cat extends Animal { fn meow(self) -> string; }\n\
             struct Kitty { int id }\n\
             impl Animal for Kitty { fn tag(self) -> string { return \"cat\"; } }\n\
             impl Cat for Kitty { fn meow(self) -> string { return \"meow\"; } }\n\
             fn pair<T>(fn(T) -> T first, fn(T) -> T second) -> void { }\n\
             fn id_cat(Cat c) -> Cat { return c; }\n\
             fn widen(Animal a) -> Cat { return new Kitty { id: 1 }; }\n\
             pair(id_cat, widen);\n",
        );
        assert!(
            err.contains("variance")
                || err.contains("invariant")
                || err.contains("type parameter `T`"),
            "expected variance diagnostic, got: {err}"
        );
    }
}
