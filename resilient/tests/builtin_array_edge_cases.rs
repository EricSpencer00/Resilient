//! Edge-case tests for array builtins: pad_left/pad_right, rotate, slice, dedup,
//! flatten, chunking, stats (min/max/sum/mean), argmin/argmax, binary_search, set helpers.

#[test]
fn test_array_pad_left_empty() {
    let code = r#"
fn main() {
    let arr = [];
    let padded = array_pad_left(arr, 3, 0);
    println(padded);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(
        result.stdout.contains("[0, 0, 0]"),
        "Got: {}",
        result.stdout
    );
}

#[test]
fn test_array_pad_left_partial() {
    let code = r#"
fn main() {
    let arr = [1, 2];
    let padded = array_pad_left(arr, 5, 0);
    println(padded);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(
        result.stdout.contains("[0, 0, 0, 1, 2]"),
        "Got: {}",
        result.stdout
    );
}

#[test]
fn test_array_pad_left_no_padding_needed() {
    let code = r#"
fn main() {
    let arr = [1, 2, 3];
    let padded = array_pad_left(arr, 2, 0);
    println(padded);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(
        result.stdout.contains("[1, 2, 3]"),
        "Got: {}",
        result.stdout
    );
}

#[test]
fn test_array_pad_right_empty() {
    let code = r#"
fn main() {
    let arr = [];
    let padded = array_pad_right(arr, 3, 99);
    println(padded);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(
        result.stdout.contains("[99, 99, 99]"),
        "Got: {}",
        result.stdout
    );
}

#[test]
fn test_array_pad_right_partial() {
    let code = r#"
fn main() {
    let arr = [1, 2];
    let padded = array_pad_right(arr, 4, 5);
    println(padded);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(
        result.stdout.contains("[1, 2, 5, 5]"),
        "Got: {}",
        result.stdout
    );
}

#[test]
fn test_array_rotate_left_empty() {
    let code = r#"
fn main() {
    let arr = [];
    let rotated = array_rotate_left(arr, 1);
    println(rotated);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("[]"), "Got: {}", result.stdout);
}

#[test]
fn test_array_rotate_left_single() {
    let code = r#"
fn main() {
    let arr = [1, 2, 3, 4];
    let rotated = array_rotate_left(arr, 1);
    println(rotated);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(
        result.stdout.contains("[2, 3, 4, 1]"),
        "Got: {}",
        result.stdout
    );
}

#[test]
fn test_array_rotate_left_wraparound() {
    let code = r#"
fn main() {
    let arr = [1, 2, 3];
    let rotated = array_rotate_left(arr, 5);
    println(rotated);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(
        result.stdout.contains("[3, 1, 2]"),
        "Got: {}",
        result.stdout
    );
}

#[test]
fn test_array_rotate_right_single() {
    let code = r#"
fn main() {
    let arr = [1, 2, 3, 4];
    let rotated = array_rotate_right(arr, 1);
    println(rotated);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(
        result.stdout.contains("[4, 1, 2, 3]"),
        "Got: {}",
        result.stdout
    );
}

#[test]
fn test_array_slice_empty() {
    let code = r#"
fn main() {
    let arr = [1, 2, 3, 4, 5];
    let sliced = slice(arr, 2, 2);
    println(sliced);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("[]"), "Got: {}", result.stdout);
}

#[test]
fn test_array_slice_normal() {
    let code = r#"
fn main() {
    let arr = [1, 2, 3, 4, 5];
    let sliced = slice(arr, 1, 4);
    println(sliced);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(
        result.stdout.contains("[2, 3, 4]"),
        "Got: {}",
        result.stdout
    );
}

#[test]
fn test_array_slice_full_range() {
    let code = r#"
fn main() {
    let arr = [1, 2, 3];
    let sliced = slice(arr, 0, 3);
    println(sliced);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(
        result.stdout.contains("[1, 2, 3]"),
        "Got: {}",
        result.stdout
    );
}

#[test]
fn test_array_dedup_empty() {
    let code = r#"
fn main() {
    let arr = [];
    let deduped = array_dedup(arr);
    println(deduped);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("[]"), "Got: {}", result.stdout);
}

#[test]
fn test_array_dedup_adjacent() {
    let code = r#"
fn main() {
    let arr = [1, 1, 2, 2, 2, 3, 1, 1];
    let deduped = array_dedup(arr);
    println(deduped);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(
        result.stdout.contains("[1, 2, 3, 1]"),
        "Got: {}",
        result.stdout
    );
}

#[test]
fn test_array_dedup_no_adjacent_duplicates() {
    let code = r#"
fn main() {
    let arr = [1, 2, 3, 4];
    let deduped = array_dedup(arr);
    println(deduped);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(
        result.stdout.contains("[1, 2, 3, 4]"),
        "Got: {}",
        result.stdout
    );
}

#[test]
fn test_array_unique_empty() {
    let code = r#"
fn main() {
    let arr = [];
    let unique = array_unique(arr);
    println(unique);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("[]"), "Got: {}", result.stdout);
}

#[test]
fn test_array_unique_duplicates() {
    let code = r#"
fn main() {
    let arr = [3, 1, 2, 1, 3, 2];
    let unique = array_unique(arr);
    println(unique);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(
        result.stdout.contains("[3, 1, 2]"),
        "Got: {}",
        result.stdout
    );
}

#[test]
fn test_array_flatten_empty() {
    let code = r#"
fn main() {
    let arr = [];
    let flat = array_flatten(arr);
    println(flat);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("[]"), "Got: {}", result.stdout);
}

#[test]
fn test_array_flatten_nested() {
    let code = r#"
fn main() {
    let arr = [[1, 2], [3, 4], [5]];
    let flat = array_flatten(arr);
    println(flat);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(
        result.stdout.contains("[1, 2, 3, 4, 5]"),
        "Got: {}",
        result.stdout
    );
}

#[test]
fn test_array_flatten_with_empty_inner() {
    let code = r#"
fn main() {
    let arr = [[1], [], [2, 3]];
    let flat = array_flatten(arr);
    println(flat);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(
        result.stdout.contains("[1, 2, 3]"),
        "Got: {}",
        result.stdout
    );
}

#[test]
fn test_array_chunk_normal() {
    let code = r#"
fn main() {
    let arr = [1, 2, 3, 4, 5];
    let chunked = array_chunk(arr, 2);
    println(chunked);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(
        result.stdout.contains("[[1, 2], [3, 4], [5]]"),
        "Got: {}",
        result.stdout
    );
}

#[test]
fn test_array_chunk_exact_fit() {
    let code = r#"
fn main() {
    let arr = [1, 2, 3, 4];
    let chunked = array_chunk(arr, 2);
    println(chunked);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(
        result.stdout.contains("[[1, 2], [3, 4]]"),
        "Got: {}",
        result.stdout
    );
}

#[test]
fn test_array_chunk_empty() {
    let code = r#"
fn main() {
    let arr = [];
    let chunked = array_chunk(arr, 2);
    println(chunked);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("[]"), "Got: {}", result.stdout);
}

#[test]
fn test_array_window_empty() {
    let code = r#"
fn main() {
    let arr = [];
    let windows = array_window(arr, 2);
    println(windows);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("[]"), "Got: {}", result.stdout);
}

#[test]
fn test_array_window_normal() {
    let code = r#"
fn main() {
    let arr = [1, 2, 3, 4];
    let windows = array_window(arr, 2);
    println(windows);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(
        result.stdout.contains("[[1, 2], [2, 3], [3, 4]]"),
        "Got: {}",
        result.stdout
    );
}

#[test]
fn test_array_min_single() {
    let code = r#"
fn main() {
    let arr = [5];
    let m = array_min(arr);
    println(m);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("5"), "Got: {}", result.stdout);
}

#[test]
fn test_array_min_multiple() {
    let code = r#"
fn main() {
    let arr = [3, 1, 4, 1, 5, 9];
    let m = array_min(arr);
    println(m);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("1"), "Got: {}", result.stdout);
}

#[test]
fn test_array_max_single() {
    let code = r#"
fn main() {
    let arr = [5];
    let m = array_max(arr);
    println(m);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("5"), "Got: {}", result.stdout);
}

#[test]
fn test_array_max_multiple() {
    let code = r#"
fn main() {
    let arr = [3, 1, 4, 1, 5, 9];
    let m = array_max(arr);
    println(m);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("9"), "Got: {}", result.stdout);
}

#[test]
fn test_array_sum_empty() {
    let code = r#"
fn main() {
    let arr = [];
    let s = array_sum(arr);
    println(s);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("0"), "Got: {}", result.stdout);
}

#[test]
fn test_array_sum_normal() {
    let code = r#"
fn main() {
    let arr = [1, 2, 3, 4, 5];
    let s = array_sum(arr);
    println(s);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("15"), "Got: {}", result.stdout);
}

#[test]
fn test_array_product_empty() {
    let code = r#"
fn main() {
    let arr = [];
    let p = array_product(arr);
    println(p);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("1"), "Got: {}", result.stdout);
}

#[test]
fn test_array_product_normal() {
    let code = r#"
fn main() {
    let arr = [2, 3, 4];
    let p = array_product(arr);
    println(p);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("24"), "Got: {}", result.stdout);
}

#[test]
fn test_array_average_single() {
    let code = r#"
fn main() {
    let arr = [5];
    let avg = array_average(arr);
    println(avg);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("5"), "Got: {}", result.stdout);
}

#[test]
fn test_array_average_multiple() {
    let code = r#"
fn main() {
    let arr = [2, 4, 6];
    let avg = array_average(arr);
    println(avg);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("4"), "Got: {}", result.stdout);
}

#[test]
fn test_array_median_odd_length() {
    let code = r#"
fn main() {
    let arr = [3, 1, 2];
    let med = array_median(arr);
    println(med);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("2"), "Got: {}", result.stdout);
}

#[test]
fn test_array_median_even_length() {
    let code = r#"
fn main() {
    let arr = [1, 2, 3, 4];
    let med = array_median(arr);
    println(med);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("2.5"), "Got: {}", result.stdout);
}

#[test]
fn test_array_min_float_basic() {
    let code = r#"
fn main() {
    let arr = [3.5, 1.2, 4.1];
    let m = array_min_float(arr);
    println(m);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("1.2"), "Got: {}", result.stdout);
}

#[test]
fn test_array_max_float_basic() {
    let code = r#"
fn main() {
    let arr = [3.5, 1.2, 4.1];
    let m = array_max_float(arr);
    println(m);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("4.1"), "Got: {}", result.stdout);
}

#[test]
fn test_array_sum_float_empty() {
    let code = r#"
fn main() {
    let arr = [];
    let s = array_sum_float(arr);
    println(s);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("0"), "Got: {}", result.stdout);
}

#[test]
fn test_array_sum_float_normal() {
    let code = r#"
fn main() {
    let arr = [1.5, 2.5, 3.0];
    let s = array_sum_float(arr);
    println(s);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("7"), "Got: {}", result.stdout);
}

#[test]
fn test_array_argmax_float_single() {
    let code = r#"
fn main() {
    let arr = [5.0];
    let idx = array_argmax_float(arr);
    println(idx);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("0"), "Got: {}", result.stdout);
}

#[test]
fn test_array_argmax_float_multiple() {
    let code = r#"
fn main() {
    let arr = [1.5, 4.2, 2.1, 3.9];
    let idx = array_argmax_float(arr);
    println(idx);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("1"), "Got: {}", result.stdout);
}

#[test]
fn test_array_argmin_float_single() {
    let code = r#"
fn main() {
    let arr = [5.0];
    let idx = array_argmin_float(arr);
    println(idx);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("0"), "Got: {}", result.stdout);
}

#[test]
fn test_array_argmin_float_multiple() {
    let code = r#"
fn main() {
    let arr = [3.0, 1.5, 4.2, 2.1];
    let idx = array_argmin_float(arr);
    println(idx);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("1"), "Got: {}", result.stdout);
}

#[test]
fn test_array_binary_search_found() {
    let code = r#"
fn main() {
    let arr = [1, 3, 5, 7, 9];
    let idx = array_binary_search(arr, 5);
    match idx {
        Ok(i) => println(i),
        Err(_) => println("not found"),
    }
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("2"), "Got: {}", result.stdout);
}

#[test]
fn test_array_binary_search_not_found() {
    let code = r#"
fn main() {
    let arr = [1, 3, 5, 7, 9];
    let idx = array_binary_search(arr, 4);
    match idx {
        Ok(_) => println("found"),
        Err(_) => println("not found"),
    }
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(
        result.stdout.contains("not found"),
        "Got: {}",
        result.stdout
    );
}

#[test]
fn test_array_binary_search_float_found() {
    let code = r#"
fn main() {
    let arr = [1.0, 2.5, 4.0, 6.5];
    let idx = array_binary_search_float(arr, 4.0);
    match idx {
        Ok(i) => println(i),
        Err(_) => println("not found"),
    }
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("2"), "Got: {}", result.stdout);
}

#[test]
fn test_set_union_empty() {
    let code = r#"
fn main() {
    let s1 = set_new();
    let s2 = set_new();
    let u = set_union(s1, s2);
    println(set_len(u));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("0"), "Got: {}", result.stdout);
}

#[test]
fn test_set_union_basic() {
    let code = r#"
fn main() {
    let s1 = set_new();
    let s1 = set_insert(s1, 1);
    let s1 = set_insert(s1, 2);
    let s2 = set_new();
    let s2 = set_insert(s2, 2);
    let s2 = set_insert(s2, 3);
    let u = set_union(s1, s2);
    println(set_len(u));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("3"), "Got: {}", result.stdout);
}

#[test]
fn test_set_intersection_empty() {
    let code = r#"
fn main() {
    let s1 = set_new();
    let s2 = set_new();
    let i = set_intersection(s1, s2);
    println(set_len(i));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("0"), "Got: {}", result.stdout);
}

#[test]
fn test_set_intersection_basic() {
    let code = r#"
fn main() {
    let s1 = set_new();
    let s1 = set_insert(s1, 1);
    let s1 = set_insert(s1, 2);
    let s1 = set_insert(s1, 3);
    let s2 = set_new();
    let s2 = set_insert(s2, 2);
    let s2 = set_insert(s2, 3);
    let s2 = set_insert(s2, 4);
    let i = set_intersection(s1, s2);
    println(set_len(i));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("2"), "Got: {}", result.stdout);
}

#[test]
fn test_set_difference_empty() {
    let code = r#"
fn main() {
    let s1 = set_new();
    let s2 = set_new();
    let d = set_difference(s1, s2);
    println(set_len(d));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("0"), "Got: {}", result.stdout);
}

#[test]
fn test_set_difference_basic() {
    let code = r#"
fn main() {
    let s1 = set_new();
    let s1 = set_insert(s1, 1);
    let s1 = set_insert(s1, 2);
    let s1 = set_insert(s1, 3);
    let s2 = set_new();
    let s2 = set_insert(s2, 2);
    let s2 = set_insert(s2, 4);
    let d = set_difference(s1, s2);
    println(set_len(d));
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("2"), "Got: {}", result.stdout);
}

#[test]
fn test_set_is_subset_empty_of_empty() {
    let code = r#"
fn main() {
    let s1 = set_new();
    let s2 = set_new();
    let is_sub = set_is_subset(s1, s2);
    println(is_sub);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("true"), "Got: {}", result.stdout);
}

#[test]
fn test_array_is_empty_error() {
    let code = r#"
fn main() {
    let arr = [];
    let m = array_min(arr);
    println(m);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(!result.ok, "Should have failed on empty array");
    assert!(!result.errors.is_empty(), "Expected errors");
}

#[test]
fn test_array_sort_already_sorted() {
    let code = r#"
fn main() {
    let arr = [1, 2, 3, 4, 5];
    let sorted = array_sort(arr);
    println(sorted);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(
        result.stdout.contains("[1, 2, 3, 4, 5]"),
        "Got: {}",
        result.stdout
    );
}

#[test]
fn test_array_sort_reverse() {
    let code = r#"
fn main() {
    let arr = [5, 4, 3, 2, 1];
    let sorted = array_sort(arr);
    println(sorted);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(
        result.stdout.contains("[1, 2, 3, 4, 5]"),
        "Got: {}",
        result.stdout
    );
}

#[test]
fn test_array_sort_desc_already_sorted() {
    let code = r#"
fn main() {
    let arr = [5, 4, 3, 2, 1];
    let sorted = array_sort_desc(arr);
    println(sorted);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(
        result.stdout.contains("[5, 4, 3, 2, 1]"),
        "Got: {}",
        result.stdout
    );
}

#[test]
fn test_array_pairs_empty() {
    let code = r#"
fn main() {
    let arr = [];
    let pairs = array_pairs(arr);
    println(pairs);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("[]"), "Got: {}", result.stdout);
}

#[test]
fn test_array_pairs_single() {
    let code = r#"
fn main() {
    let arr = [1];
    let pairs = array_pairs(arr);
    println(pairs);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("[]"), "Got: {}", result.stdout);
}

#[test]
fn test_array_intersperse_empty() {
    let code = r#"
fn main() {
    let arr = [];
    let interspersed = array_intersperse(arr, 0);
    println(interspersed);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("[]"), "Got: {}", result.stdout);
}

#[test]
fn test_array_intersperse_single() {
    let code = r#"
fn main() {
    let arr = [1];
    let interspersed = array_intersperse(arr, 0);
    println(interspersed);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("[1]"), "Got: {}", result.stdout);
}

#[test]
fn test_array_intersperse_multiple() {
    let code = r#"
fn main() {
    let arr = [1, 2, 3];
    let interspersed = array_intersperse(arr, 0);
    println(interspersed);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(
        result.stdout.contains("[1, 0, 2, 0, 3]"),
        "Got: {}",
        result.stdout
    );
}

#[test]
fn test_array_product_float_empty() {
    let code = r#"
fn main() {
    let arr = [];
    let p = array_product_float(arr);
    println(p);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("1"), "Got: {}", result.stdout);
}

#[test]
fn test_array_product_float_normal() {
    let code = r#"
fn main() {
    let arr = [2.0, 3.0, 0.5];
    let p = array_product_float(arr);
    println(p);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("3"), "Got: {}", result.stdout);
}

#[test]
fn test_array_average_float_single() {
    let code = r#"
fn main() {
    let arr = [5.5];
    let avg = array_average_float(arr);
    println(avg);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("5.5"), "Got: {}", result.stdout);
}

#[test]
fn test_array_concat_empty() {
    let code = r#"
fn main() {
    let a1 = [];
    let a2 = [];
    let concat = array_concat(a1, a2);
    println(concat);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(result.stdout.contains("[]"), "Got: {}", result.stdout);
}

#[test]
fn test_array_concat_basic() {
    let code = r#"
fn main() {
    let a1 = [1, 2];
    let a2 = [3, 4];
    let concat = array_concat(a1, a2);
    println(concat);
}
main();
"#;
    let result = resilient::run_program(code);
    assert!(result.ok, "Failed: {:?}", result.errors);
    assert!(
        result.stdout.contains("[1, 2, 3, 4]"),
        "Got: {}",
        result.stdout
    );
}
