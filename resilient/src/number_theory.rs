//! RES-2655: Number theory builtins.
//!
//! * `prime_factors(n)` — prime factorization (sorted list, with repeats).
//! * `primes_up_to(n)` — sieve of Eratosthenes up to n (inclusive).
//! * `euler_totient(n)` — count of integers 1..=n coprime to n.
//! * `is_perfect(n)` — true iff n equals the sum of its proper divisors.
//! * `divisors(n)` — sorted list of all positive divisors of n.
//! * `digit_sum(n)` — sum of decimal digits of |n|.
//! * `digital_root(n)` — iterated digit_sum until single digit.
//! * `collatz_length(n)` — length of Collatz sequence starting at n.
//! * `is_fibonacci(n)` — true iff n is a Fibonacci number.

use crate::Value;

type RResult<T> = Result<T, String>;

/// `prime_factors(n) -> Array<int>`
///
/// Returns the prime factors of `n` in sorted (non-decreasing) order,
/// including repetition. `n` must be >= 2. Returns `[]` for `n < 2`.
///
/// ```text
/// prime_factors(12)  // == [2, 2, 3]
/// prime_factors(13)  // == [13]
/// prime_factors(1)   // == []
/// ```
pub(crate) fn builtin_prime_factors(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(n)] => {
            let mut n = *n;
            if n < 2 {
                return Ok(Value::Array(vec![]));
            }
            let mut factors = Vec::new();
            let mut d = 2i64;
            while d * d <= n {
                while n % d == 0 {
                    factors.push(Value::Int(d));
                    n /= d;
                }
                d += 1;
            }
            if n > 1 {
                factors.push(Value::Int(n));
            }
            Ok(Value::Array(factors))
        }
        [other] => Err(format!("prime_factors: expected int, got {other}")),
        _ => Err(format!(
            "prime_factors: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `primes_up_to(n) -> Array<int>`
///
/// Returns all prime numbers ≤ `n` via the Sieve of Eratosthenes.
/// `n` must be >= 0. Returns `[]` for `n < 2`.
///
/// ```text
/// primes_up_to(10)  // == [2, 3, 5, 7]
/// primes_up_to(1)   // == []
/// ```
pub(crate) fn builtin_primes_up_to(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(n)] => {
            if *n < 0 {
                return Err(format!("primes_up_to: n must be >= 0, got {n}"));
            }
            let n = *n as usize;
            if n < 2 {
                return Ok(Value::Array(vec![]));
            }
            if n > 10_000_000 {
                return Err(format!("primes_up_to: n ({n}) exceeds limit of 10,000,000"));
            }
            let mut is_prime = vec![true; n + 1];
            is_prime[0] = false;
            is_prime[1] = false;
            let mut i = 2;
            while i * i <= n {
                if is_prime[i] {
                    let mut j = i * i;
                    while j <= n {
                        is_prime[j] = false;
                        j += i;
                    }
                }
                i += 1;
            }
            let primes: Vec<Value> = (2..=n)
                .filter(|&k| is_prime[k])
                .map(|k| Value::Int(k as i64))
                .collect();
            Ok(Value::Array(primes))
        }
        [other] => Err(format!("primes_up_to: expected int, got {other}")),
        _ => Err(format!(
            "primes_up_to: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `euler_totient(n) -> int`
///
/// Counts the integers in `1..=n` that are coprime to `n` (i.e., `gcd(k, n) = 1`).
/// `n` must be >= 1. `euler_totient(1) = 1`.
///
/// ```text
/// euler_totient(6)   // == 2  (1 and 5)
/// euler_totient(7)   // == 6  (1,2,3,4,5,6 — prime)
/// ```
pub(crate) fn builtin_euler_totient(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(n)] => {
            if *n < 1 {
                return Err(format!("euler_totient: n must be >= 1, got {n}"));
            }
            let mut n = *n;
            let mut result = n;
            let mut p = 2i64;
            while p * p <= n {
                if n % p == 0 {
                    while n % p == 0 {
                        n /= p;
                    }
                    result -= result / p;
                }
                p += 1;
            }
            if n > 1 {
                result -= result / n;
            }
            Ok(Value::Int(result))
        }
        [other] => Err(format!("euler_totient: expected int, got {other}")),
        _ => Err(format!(
            "euler_totient: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `divisors(n) -> Array<int>`
///
/// Returns all positive divisors of `n` in sorted ascending order.
/// `n` must be >= 1.
///
/// ```text
/// divisors(12)  // == [1, 2, 3, 4, 6, 12]
/// divisors(7)   // == [1, 7]
/// ```
pub(crate) fn builtin_divisors(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(n)] => {
            if *n < 1 {
                return Err(format!("divisors: n must be >= 1, got {n}"));
            }
            let n = *n;
            let mut divs: Vec<i64> = Vec::new();
            let mut i = 1i64;
            while i * i <= n {
                if n % i == 0 {
                    divs.push(i);
                    if i != n / i {
                        divs.push(n / i);
                    }
                }
                i += 1;
            }
            divs.sort_unstable();
            Ok(Value::Array(divs.into_iter().map(Value::Int).collect()))
        }
        [other] => Err(format!("divisors: expected int, got {other}")),
        _ => Err(format!("divisors: expected 1 argument, got {}", args.len())),
    }
}

/// `is_perfect(n) -> bool`
///
/// Returns true iff `n` equals the sum of its proper divisors (i.e., all
/// divisors except `n` itself). Only positive integers can be perfect.
///
/// ```text
/// is_perfect(6)   // == true  (1 + 2 + 3 = 6)
/// is_perfect(28)  // == true  (1+2+4+7+14 = 28)
/// is_perfect(7)   // == false
/// ```
pub(crate) fn builtin_is_perfect(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(n)] => {
            if *n < 2 {
                return Ok(Value::Bool(false));
            }
            let n = *n;
            let mut sum = 1i64;
            let mut i = 2i64;
            while i * i <= n {
                if n % i == 0 {
                    sum += i;
                    if i != n / i {
                        sum += n / i;
                    }
                }
                i += 1;
            }
            Ok(Value::Bool(sum == n))
        }
        [other] => Err(format!("is_perfect: expected int, got {other}")),
        _ => Err(format!(
            "is_perfect: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `digit_sum(n) -> int`
///
/// Returns the sum of the decimal digits of `|n|`.
///
/// ```text
/// digit_sum(493)  // == 16
/// digit_sum(-9)   // == 9
/// digit_sum(0)    // == 0
/// ```
pub(crate) fn builtin_digit_sum(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(n)] => {
            let mut n = n.unsigned_abs();
            let mut sum = 0u64;
            if n == 0 {
                return Ok(Value::Int(0));
            }
            while n > 0 {
                sum += n % 10;
                n /= 10;
            }
            Ok(Value::Int(sum as i64))
        }
        [other] => Err(format!("digit_sum: expected int, got {other}")),
        _ => Err(format!(
            "digit_sum: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `digital_root(n) -> int`
///
/// Repeatedly sums the digits of `|n|` until a single-digit result (0–9)
/// is reached. `digital_root(0) = 0`.
///
/// ```text
/// digital_root(493)   // == 7  (4+9+3=16, 1+6=7)
/// digital_root(999)   // == 9
/// ```
pub(crate) fn builtin_digital_root(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(n)] => {
            let n = n.unsigned_abs();
            if n == 0 {
                return Ok(Value::Int(0));
            }
            // Formula: digital_root = 1 + (n-1) % 9
            let root = 1 + (n - 1) % 9;
            Ok(Value::Int(root as i64))
        }
        [other] => Err(format!("digital_root: expected int, got {other}")),
        _ => Err(format!(
            "digital_root: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `collatz_length(n) -> int`
///
/// Returns the number of steps to reach 1 via the Collatz sequence:
/// - if n is even: n → n/2
/// - if n is odd:  n → 3n + 1
///
/// `n` must be >= 1. `collatz_length(1) = 0`.
///
/// ```text
/// collatz_length(6)   // == 8  (6→3→10→5→16→8→4→2→1)
/// collatz_length(27)  // == 111
/// ```
pub(crate) fn builtin_collatz_length(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(n)] => {
            if *n < 1 {
                return Err(format!("collatz_length: n must be >= 1, got {n}"));
            }
            let mut n = *n as u64;
            let mut steps = 0u64;
            while n != 1 {
                if n.is_multiple_of(2) {
                    n /= 2;
                } else {
                    n = n.saturating_mul(3).saturating_add(1);
                }
                steps += 1;
                if steps > 10_000_000 {
                    return Err(
                        "collatz_length: exceeded 10,000,000 steps — possible overflow".to_string(),
                    );
                }
            }
            Ok(Value::Int(steps as i64))
        }
        [other] => Err(format!("collatz_length: expected int, got {other}")),
        _ => Err(format!(
            "collatz_length: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `is_fibonacci(n) -> bool`
///
/// Returns true iff `n` is a Fibonacci number (0, 1, 1, 2, 3, 5, 8, ...).
/// Uses the identity: n is Fibonacci iff 5n²+4 or 5n²-4 is a perfect square.
///
/// ```text
/// is_fibonacci(0)   // == true
/// is_fibonacci(8)   // == true
/// is_fibonacci(10)  // == false
/// ```
pub(crate) fn builtin_is_fibonacci(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(n)] => {
            if *n < 0 {
                return Ok(Value::Bool(false));
            }
            let n = *n as u64;
            let is_perfect_square = |x: u64| -> bool {
                let s = (x as f64).sqrt() as u64;
                s * s == x || (s + 1) * (s + 1) == x
            };
            let five_n_sq = 5u64.saturating_mul(n.saturating_mul(n));
            let result = is_perfect_square(five_n_sq.saturating_add(4))
                || five_n_sq >= 4 && is_perfect_square(five_n_sq - 4);
            Ok(Value::Bool(result))
        }
        [other] => Err(format!("is_fibonacci: expected int, got {other}")),
        _ => Err(format!(
            "is_fibonacci: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `int_log(n, base) -> int`
///
/// Returns the integer floor-logarithm of `n` in the given `base`.
/// Both `n` and `base` must be >= 2.
///
/// ```text
/// int_log(8, 2)    // == 3
/// int_log(1000, 10) // == 3
/// ```
pub(crate) fn builtin_int_log(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(n), Value::Int(base)] => {
            if *n < 1 {
                return Err(format!("int_log: n must be >= 1, got {n}"));
            }
            if *base < 2 {
                return Err(format!("int_log: base must be >= 2, got {base}"));
            }
            let mut n = *n;
            let base = *base;
            let mut exp = 0i64;
            while n >= base {
                n /= base;
                exp += 1;
            }
            Ok(Value::Int(exp))
        }
        [a, b] => Err(format!("int_log: expected (int, int), got ({a}, {b})")),
        _ => Err(format!(
            "int_log: expected 2 arguments (n, base), got {}",
            args.len()
        )),
    }
}

/// `count_digits(n) -> int`
///
/// Returns the number of decimal digits in `|n|`. `count_digits(0) = 1`.
///
/// ```text
/// count_digits(0)     // == 1
/// count_digits(100)   // == 3
/// count_digits(-9999) // == 4
/// ```
pub(crate) fn builtin_count_digits(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(n)] => {
            let mut n = n.unsigned_abs();
            if n == 0 {
                return Ok(Value::Int(1));
            }
            let mut count = 0i64;
            while n > 0 {
                n /= 10;
                count += 1;
            }
            Ok(Value::Int(count))
        }
        [other] => Err(format!("count_digits: expected int, got {other}")),
        _ => Err(format!(
            "count_digits: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `int_to_digits(n) -> Array<int>`
///
/// Returns the decimal digits of `|n|` as an array (most-significant first).
/// `int_to_digits(0) = [0]`.
///
/// ```text
/// int_to_digits(123)   // == [1, 2, 3]
/// int_to_digits(-45)   // == [4, 5]
/// ```
pub(crate) fn builtin_int_to_digits(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(n)] => {
            let mut n = n.unsigned_abs();
            if n == 0 {
                return Ok(Value::Array(vec![Value::Int(0)]));
            }
            let mut digits = Vec::new();
            while n > 0 {
                digits.push(Value::Int((n % 10) as i64));
                n /= 10;
            }
            digits.reverse();
            Ok(Value::Array(digits))
        }
        [other] => Err(format!("int_to_digits: expected int, got {other}")),
        _ => Err(format!(
            "int_to_digits: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `int_from_digits(digits) -> int`
///
/// Reconstructs an integer from an array of decimal digits (most-significant
/// first). All elements must be ints in 0–9.
///
/// ```text
/// int_from_digits([1, 2, 3])  // == 123
/// int_from_digits([0])        // == 0
/// ```
pub(crate) fn builtin_int_from_digits(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(digits)] => {
            let mut result = 0i64;
            for (i, d) in digits.iter().enumerate() {
                match d {
                    Value::Int(digit) if *digit >= 0 && *digit <= 9 => {
                        result = result
                            .checked_mul(10)
                            .and_then(|r| r.checked_add(*digit))
                            .ok_or_else(|| format!("int_from_digits: overflow at index {i}"))?;
                    }
                    Value::Int(digit) => {
                        return Err(format!(
                            "int_from_digits: digit at index {i} must be 0-9, got {digit}"
                        ));
                    }
                    other => {
                        return Err(format!(
                            "int_from_digits: expected int digit at index {i}, got {other}"
                        ));
                    }
                }
            }
            Ok(Value::Int(result))
        }
        [other] => Err(format!("int_from_digits: expected Array, got {other}")),
        _ => Err(format!(
            "int_from_digits: expected 1 argument, got {}",
            args.len()
        )),
    }
}

#[cfg(test)]
mod tests {
    use crate::run_program;

    fn run(src: &str) -> crate::RunResult {
        run_program(src)
    }

    // ── prime_factors ─────────────────────────────────────────────────────────

    #[test]
    fn prime_factors_of_12() {
        let r = run("println(prime_factors(12));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("[2, 2, 3]"), "stdout: {}", r.stdout);
    }

    #[test]
    fn prime_factors_of_prime() {
        let r = run("println(prime_factors(13));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("[13]"), "stdout: {}", r.stdout);
    }

    #[test]
    fn prime_factors_of_1() {
        let r = run("println(len(prime_factors(1)));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('0'), "stdout: {}", r.stdout);
    }

    // ── primes_up_to ──────────────────────────────────────────────────────────

    #[test]
    fn primes_up_to_10() {
        let r = run("println(primes_up_to(10));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("[2, 3, 5, 7]"), "stdout: {}", r.stdout);
    }

    #[test]
    fn primes_up_to_1_is_empty() {
        let r = run("println(len(primes_up_to(1)));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('0'), "stdout: {}", r.stdout);
    }

    #[test]
    fn primes_up_to_count() {
        // There are 25 primes <= 100
        let r = run("println(len(primes_up_to(100)));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("25"), "stdout: {}", r.stdout);
    }

    // ── euler_totient ─────────────────────────────────────────────────────────

    #[test]
    fn totient_of_prime() {
        // totient(7) = 6
        let r = run("println(euler_totient(7));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('6'), "stdout: {}", r.stdout);
    }

    #[test]
    fn totient_of_6() {
        // totient(6) = 2 (1 and 5)
        let r = run("println(euler_totient(6));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('2'), "stdout: {}", r.stdout);
    }

    // ── divisors ──────────────────────────────────────────────────────────────

    #[test]
    fn divisors_of_12() {
        let r = run("println(divisors(12));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(
            r.stdout.contains("[1, 2, 3, 4, 6, 12]"),
            "stdout: {}",
            r.stdout
        );
    }

    #[test]
    fn divisors_of_prime() {
        let r = run("println(divisors(7));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("[1, 7]"), "stdout: {}", r.stdout);
    }

    // ── is_perfect ────────────────────────────────────────────────────────────

    #[test]
    fn is_perfect_6() {
        let r = run("println(is_perfect(6));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("true"), "stdout: {}", r.stdout);
    }

    #[test]
    fn is_perfect_28() {
        let r = run("println(is_perfect(28));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("true"), "stdout: {}", r.stdout);
    }

    #[test]
    fn is_not_perfect_7() {
        let r = run("println(is_perfect(7));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("false"), "stdout: {}", r.stdout);
    }

    // ── digit_sum ─────────────────────────────────────────────────────────────

    #[test]
    fn digit_sum_493() {
        let r = run("println(digit_sum(493));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("16"), "stdout: {}", r.stdout);
    }

    #[test]
    fn digit_sum_zero() {
        let r = run("println(digit_sum(0));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('0'), "stdout: {}", r.stdout);
    }

    // ── digital_root ──────────────────────────────────────────────────────────

    #[test]
    fn digital_root_493() {
        // 4+9+3=16, 1+6=7
        let r = run("println(digital_root(493));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('7'), "stdout: {}", r.stdout);
    }

    #[test]
    fn digital_root_999() {
        let r = run("println(digital_root(999));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('9'), "stdout: {}", r.stdout);
    }

    // ── collatz_length ────────────────────────────────────────────────────────

    #[test]
    fn collatz_length_1() {
        let r = run("println(collatz_length(1));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('0'), "stdout: {}", r.stdout);
    }

    #[test]
    fn collatz_length_6() {
        // 6→3→10→5→16→8→4→2→1 = 8 steps
        let r = run("println(collatz_length(6));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('8'), "stdout: {}", r.stdout);
    }

    // ── is_fibonacci ──────────────────────────────────────────────────────────

    #[test]
    fn is_fibonacci_true() {
        let r = run(r#"println(is_fibonacci(0));
println(is_fibonacci(1));
println(is_fibonacci(8));
println(is_fibonacci(13));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "true");
        assert_eq!(lines[1], "true");
        assert_eq!(lines[2], "true");
        assert_eq!(lines[3], "true");
    }

    #[test]
    fn is_fibonacci_false() {
        let r = run(r#"println(is_fibonacci(4));
println(is_fibonacci(10));
println(is_fibonacci(100));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "false");
        assert_eq!(lines[1], "false");
        assert_eq!(lines[2], "false");
    }

    // ── int_log ───────────────────────────────────────────────────────────────

    #[test]
    fn int_log_base_2() {
        let r = run("println(int_log(8, 2));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('3'), "stdout: {}", r.stdout);
    }

    #[test]
    fn int_log_base_10() {
        let r = run("println(int_log(1000, 10));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('3'), "stdout: {}", r.stdout);
    }

    // ── count_digits ──────────────────────────────────────────────────────────

    #[test]
    fn count_digits_zero() {
        let r = run("println(count_digits(0));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('1'), "stdout: {}", r.stdout);
    }

    #[test]
    fn count_digits_100() {
        let r = run("println(count_digits(100));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('3'), "stdout: {}", r.stdout);
    }

    // ── int_to_digits / int_from_digits ───────────────────────────────────────

    #[test]
    fn int_to_digits_basic() {
        let r = run("println(int_to_digits(123));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("[1, 2, 3]"), "stdout: {}", r.stdout);
    }

    #[test]
    fn int_from_digits_basic() {
        let r = run("println(int_from_digits([1, 2, 3]));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("123"), "stdout: {}", r.stdout);
    }

    #[test]
    fn roundtrip_digits() {
        let r = run("let n = 9876; println(int_from_digits(int_to_digits(n)));");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("9876"), "stdout: {}", r.stdout);
    }

    // ── integration: project euler-style problems ──────────────────────────────

    #[test]
    fn sum_of_primes_below_10() {
        // 2+3+5+7 = 17
        let r = run(r#"let ps = primes_up_to(9);
let total = 0;
for p in ps {
    total = total + p;
}
println(total);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("17"), "stdout: {}", r.stdout);
    }

    #[test]
    fn euler_problem_style_digit_sum_of_factorization() {
        // prime_factors(2310) = [2,3,5,7,11]; digit_sum of each factor
        let r = run(r#"let fs = prime_factors(2310);
let total = 0;
for f in fs {
    total = total + digit_sum(f);
}
println(total);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        // 2+3+5+7+(1+1)=2+3+5+7+2=19
        assert!(r.stdout.contains("19"), "stdout: {}", r.stdout);
    }
}
