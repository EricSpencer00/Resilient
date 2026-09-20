use resilient::run_program;

#[test]
fn heap_operations_preserve_tagged_capacity_guard() {
    let result = run_program(
        r#"
let heap = heap_new();
let heap = heap_push(heap, 3);
let heap = heap_push(heap, 1);
let (value, heap) = heap_pop(heap);
println(to_string(value));
println(to_string(heap_len(heap)));
"#,
    );

    assert!(result.ok, "heap operations failed: {:?}", result.errors);
    let lines: Vec<_> = result.stdout.lines().collect();
    assert_eq!(lines, ["1", "1"]);
}
