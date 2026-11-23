//! Ordered, first-match-wins string rewrite rules (exact, wildcard, regex), loaded from YAML.
//!
//! ```
//! use redir_rules::Rules;
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

mod glob;
mod load;

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
pub(crate) enum Matcher {
    Exact(String),
    Wildcard(glob::Glob),
    Regex(regex::Regex),
}

#[derive(Debug)]
pub(crate) struct Rule {
    pub matcher: Matcher,
    pub output: String,
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
#[derive(Debug)]
pub struct Rules {
    rules: Vec<Rule>,
}

impl Rules {
    /// Parse and validate a YAML rule list.
    ///
    /// # Errors
    ///
    /// [`LoadError::Yaml`] if `text` is not a list of `{type, match, output}` objects,
    /// [`LoadError::Rules`] listing every invalid rule otherwise.
    pub fn from_yaml(text: &str) -> Result<Rules, LoadError> {
        Ok(Rules {
            rules: load::load(text)?,
        })
    }

    /// Return the output of the first rule matching `s`, or `None`.
    #[must_use]
    pub fn lookup<'a>(&'a self, s: &str) -> Option<Match<'a>> {
        self.rules.iter().enumerate().find_map(|(i, rule)| {
            let (kind, pattern, output) = match &rule.matcher {
                Matcher::Wildcard(g) => g.is_match(s).then_some((
                    Kind::Wildcard,
                    g.as_str(),
                    Cow::Borrowed(rule.output.as_str()),
                ))?,
                Matcher::Regex(re) => {
                    let caps = re.captures(s)?;
                    let mut expanded = String::new();
                    caps.expand(&rule.output, &mut expanded);
                    (Kind::Regex, re.as_str(), Cow::Owned(expanded))
                }
                Matcher::Exact(m) => (m == s).then_some((
                    Kind::Exact,
                    m.as_str(),
                    Cow::Borrowed(rule.output.as_str()),
                ))?,
            };
            Some(Match {
                index: i + 1,
                kind,
                pattern,
                output,
            })
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
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Yaml(m) => write!(f, "invalid YAML: {m}"),
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
    EmptyMatch,
    EmptyOutput,
    InvalidRegex(String),
}

impl fmt::Display for RuleErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RuleErrorKind::EmptyMatch => f.write_str("empty match"),
            RuleErrorKind::EmptyOutput => f.write_str("empty output"),
            RuleErrorKind::InvalidRegex(e) => write!(f, "invalid regex: {e}"),
        }
    }
}

#[cfg(test)]
pub(crate) mod test_util {
    use crate::Rules;

    pub fn out(r: &Rules, s: &str) -> Option<String> {
        r.lookup(s).map(|m| m.output.into_owned())
    }
}

#[cfg(test)]
mod tests {
    use crate::test_util::out;
    use crate::{Kind, LoadError, RuleErrorKind, Rules};

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
    fn invalid_regex_is_rule_error() {
        let e = Rules::from_yaml("- {type: regex, match: '(', output: o}").unwrap_err();
        assert!(
            matches!(&e, LoadError::Rules(v) if v.len() == 1 && v[0].index == 1 && matches!(v[0].kind, RuleErrorKind::InvalidRegex(_))),
            "{e:?}"
        );
    }
}
