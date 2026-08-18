/// Returns the last `n` characters of `s`, or the whole string when `n`
/// exceeds its length.
pub fn tail(s: &str, n: usize) -> &str {
    let cut = s.chars().count().saturating_sub(n);
    match s.char_indices().nth(cut) {
        Some((i, _)) => &s[i..],
        None => s,
    }
}
