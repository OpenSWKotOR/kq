//! A small case-insensitive glob, matching how people already search KotOR
//! resources (`n_*`, `*jedi*`, `k_pdan_*`).
//!
//! Full regex is available on `kq grep`; this is for names only, so `*` and
//! `?` are the whole language and a pattern with neither is a substring test.

/// Match `text` against `pattern`. Case-insensitive; `*` matches any run,
/// `?` matches one character.
pub fn matches(pattern: &str, text: &str) -> bool {
    if pattern.is_empty() {
        return true;
    }
    let p: Vec<char> = pattern.to_ascii_lowercase().chars().collect();
    let t: Vec<char> = text.to_ascii_lowercase().chars().collect();
    if !p.contains(&'*') && !p.contains(&'?') {
        return t.windows(p.len().max(1)).any(|w| w == p.as_slice()) || p.is_empty();
    }
    glob_match(&p, &t)
}

/// Iterative backtracking matcher: linear in the common case, and it cannot
/// blow the stack on a pathological pattern the way recursion would.
fn glob_match(p: &[char], t: &[char]) -> bool {
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark) = (usize::MAX, 0usize);

    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = pi;
            mark = ti;
            pi += 1;
        } else if star != usize::MAX {
            pi = star + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::matches;

    #[test]
    fn bare_text_is_a_substring_search() {
        assert!(matches("jedi", "n_jedimale001"));
        assert!(matches("JEDI", "n_jedimale001"));
        assert!(!matches("sith", "n_jedimale001"));
    }

    #[test]
    fn wildcards_anchor_the_whole_name() {
        assert!(matches("n_*", "n_jedimale001"));
        assert!(matches("*male*", "n_jedimale001"));
        assert!(!matches("n_*", "x_jedimale001"));
        assert!(matches("n_jedimale00?", "n_jedimale001"));
        assert!(!matches("n_jedimale00?", "n_jedimale0012"));
    }

    #[test]
    fn empty_pattern_matches_everything() {
        assert!(matches("", "anything"));
    }

    #[test]
    fn consecutive_stars_do_not_backtrack_forever() {
        assert!(matches(
            "a*******************b",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaab"
        ));
    }
}
