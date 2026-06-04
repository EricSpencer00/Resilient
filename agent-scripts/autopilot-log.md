2026-05-30T11:00:25Z | PR #2698 | RES-2697 | default trait method bodies — parse body, register in interpreter, inject at ImplBlock eval
2026-05-30T11:20:44Z | PR #2700 | RES-2699 | fix match arms with block bodies — '{' was parsed as map literal
2026-05-30T11:39:01Z | PR #2702 | RES-2701 | fix false-positive type errors for generic fn-type parameters
2026-05-30T11:50:21Z | PR #2704 | RES-2703 | fix false-positive non-exhaustive Result match with Ok/Err arms
2026-05-31T09:16:13Z | PR #2797 | RES-2800 | Iterator protocol — for-in over closure-based and struct-based custom iterators
2026-05-31T10:00:52Z | PR #2798 | RES-2585 | Regular expression matching builtins — regex_match, regex_find, regex_find_all, regex_captures, regex_replace, regex_replace_all
2026-05-31T11:30:00Z | PR #2800 | RES-2559 | Date/time formatting and parsing builtins — datetime_now, datetime_from_unix, datetime_to_unix, datetime_format, datetime_parse
2026-05-31T12:15:00Z | PR #2802 | RES-2801 | Generic struct type parameter validation — fix soundness hole + compatible() Tuple recursion + Char coerce
2026-05-31T13:00:00Z | PR #2804 | RES-2556 | HTTP client builtins — http_get and http_post with TCP sockets, chunked encoding, 17 tests
2026-05-31T13:30:00Z | PR #2806 | RES-2805 | Fix generic return type erasure — infer concrete types from arguments, closing soundness hole
2026-05-31T14:00:00Z | PR #2808 | RES-2807 | Float32 type gaps + panic elimination — string coercion, unary minus, RwLock/unwrap safety
2026-05-31T17:10:41Z | PR #2811 | RES-2810 | register 72 missing runtime builtins in typechecker (vec/mat/complex/number-theory/stats/graph/csv/rle/reflection/volatile) + pure/impure classification + 20 tests; filed RES-2812 (flaky GPIO test)
2026-05-31T20:15:00Z | PR #2815 | RES-2814 | fix false-positive type errors on generic enum constructors — store enum type params, skip compat check for type param args, resolve destructured bindings to Any
2026-05-31T21:00:00Z | PR #2817 | RES-2816 | fix false-positive type errors for string repetition operator and Bool type alias — string * int accepted, Bool resolves to Type::Bool
2026-05-31T21:30:00Z | PR #2819 | RES-2818 | fix false-positive non-exhaustive match on Option with qualified variant names — Option::Some/Option::None now recognized
2026-05-31T21:45:00Z | PR #2821 | RES-2820 | fix false-positive incompatible match arm types with return statements — return arms push Void, skip Void in compat check
2026-05-31T22:00:00Z | PR #2823 | RES-2822 | fix false-positive return type mismatch for never-type (!) functions — skip mismatch check, defer to never_type::check pass
2026-06-01T16:17:53Z | PR #2833 | RES-2831 | tier-1 soundness: typechecker now rejects indexing non-indexable types (int/float/bool/char/bytes/struct/tuple/option/result/range/fn) at compile time in IndexExpression + IndexAssignment; runtime previously caught these as 'Cannot index'. is_non_indexable helper, arrays/strings/maps(Any)/inference-vars stay permissive. 5 smoke tests; full suite green.
