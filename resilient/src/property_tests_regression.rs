#[cfg(test)]
mod tests {
    use crate::property_tests::check;

    fn property_test_source(item_name: &str) -> String {
        format!("fn {item_name}(int x) requires x > 0 ensures result > 0 {{ return x + 1; }}")
    }

    fn check_with_property_test_attr(
        item_name: &str,
        args: &str,
        line: usize,
    ) -> Result<(), String> {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            item_name,
            crate::feature_attrs::AttrRecord {
                name: "property_test".into(),
                args: args.into(),
                line,
            },
        );

        let src = property_test_source(item_name);
        let (prog, _) = crate::parse(&src);
        let result = check(&prog, "<test>");
        crate::feature_attrs::reset();
        result
    }

    #[test]
    fn check_accepts_baseline_property_test_declarations() {
        for (item_name, args, line) in [
            ("baseline_one", "samples = 1", 11),
            ("baseline_compact", "samples=7", 12),
            ("baseline_quoted", r#"samples = "42""#, 13),
        ] {
            let result = check_with_property_test_attr(item_name, args, line);
            assert!(
                result.is_ok(),
                "expected valid declaration for {item_name}, got: {result:?}"
            );
        }
    }

    #[test]
    fn check_rejects_malformed_property_test_declarations() {
        let cases = [
            (
                "missing_samples",
                "",
                21,
                "<test>:21:0: error[property_test]: invalid #[property_test] declaration `missing_samples`: missing required `samples` field",
            ),
            (
                "missing_equals",
                "samples 100",
                22,
                "<test>:22:0: error[property_test]: invalid #[property_test] declaration `missing_equals`: malformed entry `samples 100`; expected `samples = <integer>`",
            ),
            (
                "trailing_comma",
                "samples = 100,",
                23,
                "<test>:23:0: error[property_test]: invalid #[property_test] declaration `trailing_comma`: malformed entry ``; expected `samples = <integer>`",
            ),
            (
                "duplicate_samples",
                "samples = 100, samples = 200",
                24,
                "<test>:24:0: error[property_test]: invalid #[property_test] declaration `duplicate_samples`: duplicate `samples` field",
            ),
            (
                "zero_samples",
                "samples = 0",
                25,
                "<test>:25:0: error[property_test]: invalid #[property_test] declaration `zero_samples`: `samples` must be greater than zero",
            ),
            (
                "nonnumeric_samples",
                "samples = nope",
                26,
                "<test>:26:0: error[property_test]: invalid #[property_test] declaration `nonnumeric_samples`: `samples` must be a positive integer",
            ),
            (
                "unknown_field",
                "limit = 10",
                27,
                "<test>:27:0: error[property_test]: invalid #[property_test] declaration `unknown_field`: unknown field `limit`",
            ),
        ];

        for (item_name, args, line, expected) in cases {
            let err = check_with_property_test_attr(item_name, args, line)
                .expect_err("expected malformed property_test declaration");
            assert_eq!(err, expected);
        }
    }

    #[test]
    fn minimum_samples_value_accepted() {
        let result = check_with_property_test_attr("min_test", "samples = 1", 40);
        assert!(result.is_ok(), "minimum samples value should be accepted");
    }

    #[test]
    fn large_samples_value_accepted() {
        let result = check_with_property_test_attr("large_test", "samples = 10000", 41);
        assert!(result.is_ok(), "large samples value should be accepted");
    }

    #[test]
    fn whitespace_variations_handled() {
        for (name, args) in [
            ("ws1", "samples=1"),
            ("ws2", "samples = 1"),
            ("ws3", "  samples  =  1  "),
            ("ws4", "samples=  42  "),
        ] {
            let result = check_with_property_test_attr(name, args, 50);
            assert!(
                result.is_ok(),
                "whitespace variation for {name} should be handled"
            );
        }
    }

    #[test]
    fn quoted_samples_with_various_quotes() {
        for (name, args) in [("q1", r#"samples = "5""#), ("q2", r#"samples="99""#)] {
            let result = check_with_property_test_attr(name, args, 51);
            assert!(result.is_ok(), "quoted samples for {name} should work");
        }
    }

    #[test]
    fn error_message_includes_item_name() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "bad_func",
            crate::feature_attrs::AttrRecord {
                name: "property_test".into(),
                args: "".into(),
                line: 60,
            },
        );
        let src = "fn bad_func(int x) requires x > 0 ensures result > 0 { return x + 1; }";
        let (prog, _) = crate::parse(src);
        let err = crate::property_tests::check(&prog, "<test>").expect_err("expected error");
        assert!(
            err.contains("bad_func"),
            "error message should include function name"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn negative_samples_rejected() {
        let result = check_with_property_test_attr("neg_test", "samples = -5", 61);
        assert!(result.is_err(), "negative samples should be rejected");
    }

    #[test]
    fn multiple_whitespace_patterns() {
        let result = check_with_property_test_attr("multi_ws", "  samples  =  42  ", 62);
        assert!(result.is_ok(), "complex whitespace should be handled");
    }
}
