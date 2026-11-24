/// A glob pattern where each `*` matches any run of characters, including none.
#[derive(Debug)]
pub(crate) struct Glob {
    pattern: String,
}

impl Glob {
    pub fn new(pattern: &str) -> Glob {
        Glob {
            pattern: pattern.to_owned(),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.pattern
    }

    pub fn is_match(&self, s: &str) -> bool {
        let mut pieces = self.pattern.split('*');
        // `split` always yields at least one piece.
        let first = pieces.next().unwrap_or_default();
        let Some(last) = pieces.next_back() else {
            return s == first;
        };
        if first.len() + last.len() > s.len() {
            return false;
        }
        let Some(mut rest) = s.strip_prefix(first).and_then(|r| r.strip_suffix(last)) else {
            return false;
        };
        // Leftmost-first is enough: with only `*`, an earlier piece position never hurts later ones.
        for piece in pieces {
            match rest.find(piece) {
                Some(pos) => rest = &rest[pos + piece.len()..],
                None => return false,
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::Glob;

    #[test]
    fn wildcard_trailing() {
        let g = Glob::new("example.com/trolo/*");
        assert!(g.is_match("example.com/trolo/"));
        assert!(g.is_match("example.com/trolo/a/b.html"));
        assert!(!g.is_match("example.com/trolo"));
        assert!(!g.is_match("xexample.com/trolo/"));
    }

    #[test]
    fn wildcard_anywhere() {
        let g = Glob::new("*.example.com/old/*");
        assert!(g.is_match("www.example.com/old/x"));
        assert!(g.is_match(".example.com/old/"));
        assert!(!g.is_match("example.com/old/x"));
        let g = Glob::new("a*b*c");
        assert!(g.is_match("abc"));
        assert!(g.is_match("aXbYbZc"));
        assert!(!g.is_match("abcX"));
        assert!(!g.is_match("ac"));
        let g = Glob::new("ab*ba");
        assert!(!g.is_match("aba"));
        assert!(g.is_match("abba"));
    }

    #[test]
    fn wildcard_stars_only() {
        for p in ["*", "**"] {
            assert!(Glob::new(p).is_match(""));
            assert!(Glob::new(p).is_match("anything"));
        }
    }

    #[test]
    fn wildcard_no_star_is_equality() {
        assert!(Glob::new("a/b").is_match("a/b"));
        assert!(!Glob::new("a/b").is_match("a/bc"));
    }

    #[test]
    fn wildcard_utf8() {
        let g = Glob::new("example.com/é*/ü");
        assert!(g.is_match("example.com/é/ü"));
        assert!(g.is_match("example.com/éàà/ü"));
        assert!(!g.is_match("example.com/e/ü"));
    }
}
