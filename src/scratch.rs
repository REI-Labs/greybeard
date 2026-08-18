/// Returns the last `n` characters of `s`, or the whole string when `n`
/// exceeds its length.
pub fn tail(s: &str, n: usize) -> &str {
    &s[s.len() - n..]
}
