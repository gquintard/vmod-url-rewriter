//! Turn `exact` and `wildcard` patterns into anchored regexes for the rule matcher.

/// A regex matching exactly `pattern`, every character taken literally.
pub(crate) fn exact_to_regex(pattern: &str) -> String {
    format!(r"\A{}\z", regex_syntax::escape(pattern))
}

/// A regex matching the whole string against `pattern`, where each `*` matches any run of
/// characters (newlines included), possibly empty, and everything else is literal.
pub(crate) fn to_regex(pattern: &str) -> String {
    let pieces: Vec<String> = pattern.split('*').map(regex_syntax::escape).collect();
    format!(r"\A{}\z", pieces.join("(?s:.*)"))
}

#[cfg(test)]
mod tests {
    use regex::Regex;

    use super::{exact_to_regex, to_regex};

    fn m(p: &str, s: &str) -> bool {
        Regex::new(&to_regex(p)).unwrap().is_match(s)
    }

    #[test]
    fn wildcard_trailing() {
        let p = "example.com/trolo/*";
        assert!(m(p, "example.com/trolo/"));
        assert!(m(p, "example.com/trolo/a/b.html"));
        assert!(!m(p, "example.com/trolo"));
        assert!(!m(p, "xexample.com/trolo/"));
    }

    #[test]
    fn wildcard_anywhere() {
        let p = "*.example.com/old/*";
        assert!(m(p, "www.example.com/old/x"));
        assert!(m(p, ".example.com/old/"));
        assert!(!m(p, "example.com/old/x"));
        assert!(m("a*b*c", "abc"));
        assert!(m("a*b*c", "aXbYbZc"));
        assert!(!m("a*b*c", "abcX"));
        assert!(!m("a*b*c", "ac"));
        assert!(!m("ab*ba", "aba"));
        assert!(m("ab*ba", "abba"));
    }

    #[test]
    fn wildcard_stars_only() {
        for p in ["*", "**"] {
            assert!(m(p, ""));
            assert!(m(p, "anything"));
        }
    }

    #[test]
    fn wildcard_no_star_is_equality() {
        assert!(m("a/b", "a/b"));
        assert!(!m("a/b", "a/bc"));
    }

    #[test]
    fn wildcard_utf8() {
        let p = "example.com/é*/ü";
        assert!(m(p, "example.com/é/ü"));
        assert!(m(p, "example.com/éàà/ü"));
        assert!(!m(p, "example.com/e/ü"));
    }

    #[test]
    fn wildcard_metachars() {
        assert!(!m("a.c*", "abc"));
        assert!(m("a.c*", "a.cX"));
        assert!(m(r"(x)?+[\$*", r"(x)?+[\$ yes"));
        assert!(!m(r"(x)?+[\$*", "x"));
    }

    #[test]
    fn wildcard_star_matches_newline() {
        assert!(m("a*b", "a\nb"));
        assert!(!m("a*b", "a\nb\n"));
    }

    #[test]
    fn exact_is_literal_and_anchored() {
        let re = Regex::new(&exact_to_regex(r"a.c(\d)$")).unwrap();
        assert!(re.is_match(r"a.c(\d)$"));
        assert!(!re.is_match("abc(1)"));
        assert!(!re.is_match(r"xa.c(\d)$"));
        assert!(!re.is_match("a.c(\\d)$\n"));
    }
}
