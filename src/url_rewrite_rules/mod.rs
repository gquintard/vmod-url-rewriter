//! Ordered, first-match-wins string rewrite rules (exact, wildcard, regex), loaded from YAML.
//!
//! ```
//! use vmod_url_rewriter::url_rewrite_rules::Rules;
//!
//! let rules = Rules::from_yaml(
//!     "- {type: exact, match: example.com/index.html, output: 'https://trololo.com/tralala.php'}",
//! )
//! .unwrap();
//! let m = rules.lookup("example.com/index.html").unwrap();
//! assert_eq!(m.output, "https://trololo.com/tralala.php");
//! assert_eq!(m.index, 1);
//! ```

use std::borrow::Cow;
use std::fmt;

use serde::Deserialize;

mod captures;
mod glob;
mod load;
mod set;
mod syntax;

/// The kind of a rule, as written in the `type` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Exact,
    Wildcard,
    Regex,
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Kind::Exact => "exact",
            Kind::Wildcard => "wildcard",
            Kind::Regex => "regex",
        })
    }
}

#[derive(Debug)]
pub(crate) struct Rule {
    pub kind: Kind,
    /// The `match` field, as written.
    pub pattern: String,
    pub output: String,
    /// For a `regex` rule whose `output` has a `$`, expands its captures into `output`.
    pub captures: Option<captures::LazyExpander>,
}

/// The rule that matched a lookup, and what it produced.
#[derive(Debug, PartialEq, Eq)]
pub struct Match<'a> {
    /// 1-based position of the rule in the file.
    pub index: usize,
    pub kind: Kind,
    /// The rule's `match` field.
    pub pattern: &'a str,
    /// The rewritten string.
    pub output: Cow<'a, str>,
}

/// An ordered list of rules, where the first matching rule wins.
///
/// All rules are compiled at load time into one DFA, so a lookup scans `s` once whatever the
/// number of rules.
#[derive(Debug)]
pub struct Rules {
    /// Pattern `i` is `rules[i]`.
    set: set::PatternMatcher,
    rules: Vec<Rule>,
}

impl Rules {
    /// Parse and validate a YAML rule list.
    ///
    /// # Errors
    ///
    /// [`LoadError::Yaml`] if `text` is not a list of `{type, match, output}` objects,
    /// [`LoadError::Rules`] listing every invalid rule otherwise, [`LoadError::Compile`] if the
    /// rules are too big to compile together.
    pub fn from_yaml(text: &str) -> Result<Rules, LoadError> {
        Rules::from_yaml_with_limit(text, Rules::DEFAULT_BUILD_LIMIT)
    }

    /// Default of [`Rules::from_yaml_with_limit`]'s `build_limit`: 1 GiB.
    pub const DEFAULT_BUILD_LIMIT: usize = 1 << 30;

    /// Like [`Rules::from_yaml`], with at most `build_limit` bytes to build the DFA of all rules
    /// (the result is much smaller). Rules needing more fail with [`RuleErrorKind::TooComplex`]
    /// if they are too complex on their own, [`LoadError::Compile`] otherwise.
    ///
    /// # Errors
    ///
    /// As [`Rules::from_yaml`].
    pub fn from_yaml_with_limit(text: &str, build_limit: usize) -> Result<Rules, LoadError> {
        let (set, rules) = load::load(text, build_limit)?;
        Ok(Rules { set, rules })
    }

    /// Return the output of the first rule matching `s`, or `None`.
    #[must_use]
    pub fn lookup<'a>(&'a self, s: &str) -> Option<Match<'a>> {
        let i = self.set.first(s)?;
        let rule = &self.rules[i];
        let output = match &rule.captures {
            Some(expander) => Cow::Owned(expander.expand(&rule.pattern, s, &rule.output)?),
            None => Cow::Borrowed(rule.output.as_str()),
        };
        Some(Match {
            index: i + 1,
            kind: rule.kind,
            pattern: &rule.pattern,
            output,
        })
    }

    /// Number of rules.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    /// Whether there are no rules.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
}

/// Why a rule file was rejected.
#[derive(Debug, PartialEq, Eq)]
pub enum LoadError {
    /// Not valid YAML, or not a list of `{type, match, output}` objects.
    Yaml(String),
    /// Some rules are invalid.
    Rules(Vec<RuleError>),
    /// The rules are valid one by one, but their DFA is too big to precompute.
    Compile(String),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Yaml(m) => write!(f, "invalid YAML: {m}"),
            LoadError::Compile(m) => write!(f, "cannot compile rules: {m}"),
            LoadError::Rules(errors) => {
                for (i, e) in errors.iter().enumerate() {
                    if i > 0 {
                        f.write_str("\n")?;
                    }
                    write!(f, "{e}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for LoadError {}

/// One invalid rule.
#[derive(Debug, PartialEq, Eq)]
pub struct RuleError {
    /// 1-based position of the rule in the file.
    pub index: usize,
    pub kind: RuleErrorKind,
}

impl fmt::Display for RuleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "rule #{}: {}", self.index, self.kind)
    }
}

/// What is wrong with a rule.
#[derive(Debug, PartialEq, Eq)]
pub enum RuleErrorKind {
    /// `match` is missing, null or empty.
    EmptyMatch,
    /// `output` is missing, null or empty.
    EmptyOutput,
    InvalidRegex(String),
    /// A `regex` whose DFA is too big to precompute, such as `(a|b)*a(a|b){30}`.
    TooComplex,
    /// Not a `{type, match, output}` object: unknown `type` or key, wrong value type...
    Invalid(String),
}

impl fmt::Display for RuleErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RuleErrorKind::EmptyMatch => f.write_str("empty match"),
            RuleErrorKind::EmptyOutput => f.write_str("empty output"),
            RuleErrorKind::InvalidRegex(e) => write!(f, "invalid regex: {e}"),
            RuleErrorKind::TooComplex => f.write_str("regex too complex to precompute"),
            RuleErrorKind::Invalid(e) => write!(f, "invalid rule: {e}"),
        }
    }
}

#[cfg(test)]
pub(crate) mod test_util {
    use crate::url_rewrite_rules::Rules;

    pub fn out(r: &Rules, s: &str) -> Option<String> {
        r.lookup(s).map(|m| m.output.into_owned())
    }
}

#[cfg(test)]
mod tests {
    use crate::url_rewrite_rules::test_util::out;
    use crate::url_rewrite_rules::{Kind, LoadError, RuleErrorKind, Rules};

    #[test]
    fn wildcard_output_drops_tail() {
        let r = Rules::from_yaml(
            "- {type: wildcard, match: 'example.com/trolo/*', output: trololo.com}",
        )
        .unwrap();
        assert_eq!(
            out(&r, "example.com/trolo/a/b.html").as_deref(),
            Some("trololo.com")
        );
        let m = r.lookup("example.com/trolo/").unwrap();
        assert_eq!(
            (m.index, m.kind, m.pattern),
            (1, Kind::Wildcard, "example.com/trolo/*")
        );
        assert_eq!(r.lookup("example.com/trolo"), None);
    }

    #[test]
    fn regex_output_without_dollar_needs_no_captures() {
        let r = Rules::from_yaml(r"- {type: regex, match: '^/(\w+)$', output: /fixed}").unwrap();
        assert!(r.rules[0].captures.is_none());
        assert_eq!(out(&r, "/abc").as_deref(), Some("/fixed"));
    }

    #[test]
    fn regex_captures() {
        let r = Rules::from_yaml(
            r"- {type: regex, match: 'example\.com/index(fr|en)$', output: 'trololo.com/index$1'}",
        )
        .unwrap();
        assert_eq!(
            out(&r, "example.com/indexfr").as_deref(),
            Some("trololo.com/indexfr")
        );
        assert_eq!(
            out(&r, "www.example.com/indexen").as_deref(),
            Some("trololo.com/indexen")
        );
        assert_eq!(r.lookup("example.com/indexfr/x"), None);
        let m = r.lookup("example.com/indexfr").unwrap();
        assert_eq!(
            (m.index, m.kind, m.pattern),
            (1, Kind::Regex, r"example\.com/index(fr|en)$")
        );
    }

    #[test]
    fn regex_named_and_dollar() {
        let r = Rules::from_yaml(
            r"- {type: regex, match: '^/(?P<lang>[a-z]{2})/', output: '/${lang}/$$'}",
        )
        .unwrap();
        assert_eq!(out(&r, "/fr/page").as_deref(), Some("/fr/$"));
    }

    #[test]
    fn dollar_name_greediness() {
        let r = Rules::from_yaml(
            "- {type: regex, match: '^/(a)', output: 'x$1foo'}\n- {type: regex, match: '^/(b)', output: 'x${1}foo'}",
        )
        .unwrap();
        // `$1foo` refers to a group named "1foo", which doesn't exist.
        assert_eq!(out(&r, "/a").as_deref(), Some("x"));
        assert_eq!(out(&r, "/b").as_deref(), Some("xbfoo"));
    }

    #[test]
    fn regex_empty_expansion_still_matches() {
        let r = Rules::from_yaml("- {type: regex, match: '^/(x?)$', output: '$1'}").unwrap();
        assert_eq!(out(&r, "/").as_deref(), Some(""));
    }

    #[test]
    fn metachars_are_literal() {
        let r = Rules::from_yaml(r"- {type: exact, match: 'a.c(\d)', output: o}").unwrap();
        assert_eq!(r.lookup("abc1"), None);
        assert_eq!(out(&r, r"a.c(\d)").as_deref(), Some("o"));
    }

    #[test]
    fn first_match_wins_across_kinds() {
        let r = Rules::from_yaml(
            "- {type: wildcard, match: 'a*c', output: one}\n- {type: exact, match: axc, output: two}\n- {type: wildcard, match: 'ab*', output: three}",
        )
        .unwrap();
        assert_eq!(out(&r, "abc").as_deref(), Some("one"));
        assert_eq!(out(&r, "axc").as_deref(), Some("one"));
        assert_eq!(out(&r, "abd").as_deref(), Some("three"));
        let r = Rules::from_yaml(
            "- {type: exact, match: x, output: one}\n- {type: regex, match: b, output: two}\n- {type: wildcard, match: 'ab*', output: three}",
        )
        .unwrap();
        let m = r.lookup("abc").unwrap();
        assert_eq!((m.index, m.kind), (2, Kind::Regex));
    }

    #[test]
    fn regex_flags_do_not_leak() {
        let r = Rules::from_yaml(
            "- {type: regex, match: '(?i)^x$', output: one}\n- {type: exact, match: ABC, output: two}",
        )
        .unwrap();
        assert_eq!(out(&r, "X").as_deref(), Some("one"));
        assert_eq!(r.lookup("abc"), None);
        assert_eq!(out(&r, "ABC").as_deref(), Some("two"));
    }

    #[test]
    fn perl_classes_and_word_boundaries_are_ascii() {
        let r = Rules::from_yaml(
            r"- {type: regex, match: '^/w/(\w+)$', output: '$1'}
- {type: regex, match: '^/d/\d$', output: d}
- {type: regex, match: '^/s/\s$', output: s}
- {type: regex, match: '^/b/x\b', output: b}
- {type: regex, match: '^/c/([\w-]+)$', output: '$1'}
- {type: regex, match: '^/n/(\W)([^\w])$', output: '$1$2'}
- {type: regex, match: '^/u/(?u:\w)$', output: u}
- {type: regex, match: '^/p/(\p{L}+)$', output: '$1'}",
        )
        .unwrap();
        assert_eq!(out(&r, "/w/abc_9").as_deref(), Some("abc_9"));
        assert_eq!(r.lookup("/w/é"), None);
        assert_eq!(r.lookup("/d/٣"), None);
        assert_eq!(r.lookup("/s/\u{a0}"), None);
        assert_eq!(out(&r, "/b/x-").as_deref(), Some("b"));
        assert_eq!(out(&r, "/b/xé").as_deref(), Some("b"));
        assert_eq!(r.lookup("/b/xy"), None);
        assert_eq!(out(&r, "/c/a-b").as_deref(), Some("a-b"));
        assert_eq!(r.lookup("/c/é"), None);
        assert_eq!(out(&r, "/n/éü").as_deref(), Some("éü"));
        assert_eq!(r.lookup("/u/é"), None);
        assert_eq!(out(&r, "/p/éü").as_deref(), Some("éü"));
    }

    #[test]
    fn dot_and_literals_stay_unicode() {
        let r = Rules::from_yaml(r"- {type: regex, match: '^/é(.)$', output: '$1'}").unwrap();
        assert_eq!(out(&r, "/éü").as_deref(), Some("ü"));
    }

    #[test]
    fn compile_error_display() {
        assert_eq!(
            LoadError::Compile("too big".into()).to_string(),
            "cannot compile rules: too big"
        );
    }

    #[test]
    fn invalid_regex_is_rule_error() {
        let e = Rules::from_yaml("- {type: regex, match: '(', output: o}").unwrap_err();
        assert!(
            matches!(&e, LoadError::Rules(v) if v.len() == 1 && v[0].index == 1 && matches!(v[0].kind, RuleErrorKind::InvalidRegex(_))),
            "{e:?}"
        );
    }
}
