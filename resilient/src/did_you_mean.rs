// RES-306: did-you-mean suggestions for undefined identifiers.
//
// When the typechecker (or any other diagnostic emitter) hits an
// unknown name, it can call `suggest(target, candidates)` to get a
// short, deduplicated list of in-scope names that are within
// Levenshtein distance 2 of the typo. The caller is then free to
// stitch the result into a diagnostic such as
//   `Undefined variable 'lentgh' at 3:5 — did you mean `len`?`
//
// Design notes:
// - Inputs shorter than 3 chars are skipped to avoid false positives
//   (a 1-char typo of `x` matches every other 1-char binding).
// - At most 3 candidates are returned, sorted by distance ascending
//   then lexicographically. Ties beyond 3 are truncated.
// - The function takes an iterator so callers can stream from
//   multiple sources (locals + builtins + functions) without first
//   materialising into a Vec.

/// Suggest up to 3 in-scope names that are within Levenshtein
/// distance 2 of `target`. Returns owned `String`s for ergonomic
/// insertion into diagnostics. Returns an empty Vec if `target` is
/// shorter than 3 chars or no candidate is close enough.
pub fn suggest<'a>(target: &str, candidates: impl Iterator<Item = &'a str>) -> Vec<String> {
    if target.chars().count() < 3 {
        return Vec::new();
    }

    let mut scored: Vec<(usize, String)> = candidates
        .filter(|c| !c.is_empty() && *c != target)
        .map(|c| (levenshtein(target, c), c.to_string()))
        .filter(|(d, _)| *d <= 2)
        .collect();

    // Stable sort: distance asc, then name asc. Dedup by name in case
    // the caller fed overlapping iterators (e.g. a local that
    // shadows a builtin).
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    scored.dedup_by(|a, b| a.1 == b.1);

    scored.into_iter().take(3).map(|(_, n)| n).collect()
}

/// Plain Levenshtein distance with a rolling two-row buffer. Operates
/// on Unicode scalar values (`char`) so multi-byte identifiers are
/// counted correctly.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let n = a.len();
    let m = b.len();

    if n == 0 {
        return m;
    }
    if m == 0 {
        return n;
    }

    let mut prev: Vec<usize> = (0..=m).collect();
    let mut curr: Vec<usize> = vec![0; m + 1];

    for i in 1..=n {
        curr[0] = i;
        for j in 1..=m {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1) // deletion
                .min(curr[j - 1] + 1) // insertion
                .min(prev[j - 1] + cost); // substitution
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[m]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(slice: &[&'static str]) -> Vec<&'static str> {
        slice.to_vec()
    }

    #[test]
    fn levenshtein_basic() {
        assert_eq!(levenshtein("", ""), 0);
        assert_eq!(levenshtein("a", ""), 1);
        assert_eq!(levenshtein("", "a"), 1);
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("len", "len"), 0);
        assert_eq!(levenshtein("lentgh", "length"), 2);
        assert_eq!(levenshtein("foo", "fob"), 1);
    }

    #[test]
    fn typo_finds_close_match() {
        let pool = names(&["len", "length", "println", "print"]);
        let got = suggest("lentgh", pool.iter().copied());
        assert!(got.contains(&"length".to_string()), "got = {:?}", got);
    }

    #[test]
    fn no_match_returns_empty() {
        let pool = names(&["println", "abs", "min", "max"]);
        let got = suggest("totally_unrelated", pool.iter().copied());
        assert!(got.is_empty(), "got = {:?}", got);
    }

    #[test]
    fn multiple_candidates_returned_sorted() {
        // All within distance 2 of "fob".
        let pool = names(&["foo", "for", "fob", "fop", "fox"]);
        let got = suggest("fox_", pool.iter().copied());
        // distance("fox_","fox") = 1, others = 2; first must be `fox`.
        assert_eq!(got.first().map(String::as_str), Some("fox"));
        assert!(got.len() <= 3);
    }

    #[test]
    fn caps_at_three_candidates() {
        let pool = names(&["foo", "fop", "fob", "for", "fox", "fou", "foz"]);
        let got = suggest("foa", pool.iter().copied());
        assert_eq!(got.len(), 3);
    }

    #[test]
    fn ties_sorted_by_distance_then_name() {
        // distance 1 candidates: "len"  (delete 'g')  — wait, target is "leg"
        // target = "leg":
        //   "len" -> d=1 (sub), "leg" itself filtered, "let" d=1 (sub),
        //   "lez" d=1, etc.
        let pool = names(&["lez", "len", "let"]);
        let got = suggest("leg", pool.iter().copied());
        // All distance 1. Should be sorted alphabetically.
        assert_eq!(got, vec!["len", "let", "lez"]);
    }

    #[test]
    fn short_input_returns_empty() {
        let pool = names(&["abs", "abc", "min"]);
        // 2-char input — too short to suggest reliably.
        let got = suggest("ab", pool.iter().copied());
        assert!(got.is_empty(), "got = {:?}", got);

        // 1-char input — also skipped.
        let got = suggest("a", pool.iter().copied());
        assert!(got.is_empty(), "got = {:?}", got);
    }

    #[test]
    fn distance_three_not_suggested() {
        // "hello" vs "world" — Levenshtein distance is 4.
        let pool = names(&["world"]);
        let got = suggest("hello", pool.iter().copied());
        assert!(got.is_empty(), "got = {:?}", got);

        // Distance exactly 3 must also be excluded.
        // "abcdef" vs "ghidef" — distance is 3 (sub 3 chars).
        let pool = names(&["ghidef"]);
        let got = suggest("abcdef", pool.iter().copied());
        assert!(got.is_empty(), "got = {:?}", got);
    }

    #[test]
    fn target_itself_is_filtered() {
        let pool = names(&["length", "lentgh", "len"]);
        let got = suggest("lentgh", pool.iter().copied());
        // The target name itself must not appear in the suggestions.
        assert!(!got.iter().any(|s| s == "lentgh"));
        // But near matches still come through.
        assert!(got.contains(&"length".to_string()));
    }

    #[test]
    fn empty_candidate_iterator() {
        let got = suggest("foo", std::iter::empty());
        assert!(got.is_empty());
    }

    #[test]
    fn dedups_overlapping_sources() {
        // Simulate locals + builtins both containing "len".
        let pool = vec!["len", "len", "len"];
        let got = suggest("ln_", pool.into_iter());
        // Should appear only once.
        assert_eq!(got.iter().filter(|s| *s == "len").count(), 1);
    }
}
