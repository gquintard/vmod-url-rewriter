use serde::Deserialize;

use crate::glob::Glob;
use crate::{Kind, LoadError, Matcher, Rule, RuleError, RuleErrorKind};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRule {
    #[serde(rename = "type")]
    kind: Kind,
    #[serde(rename = "match")]
    pattern: String,
    output: String,
}

pub(crate) fn load(text: &str) -> Result<Vec<Rule>, LoadError> {
    // A null document (empty or comment-only file) is zero rules.
    let raw: Option<Vec<RawRule>> =
        serde_norway::from_str(text).map_err(|e| LoadError::Yaml(e.to_string()))?;
    let mut rules = Vec::new();
    let mut errors = Vec::new();
    for (i, r) in raw.unwrap_or_default().into_iter().enumerate() {
        let index = i + 1;
        if r.pattern.is_empty() {
            errors.push(RuleError {
                index,
                kind: RuleErrorKind::EmptyMatch,
            });
        }
        if r.output.is_empty() {
            errors.push(RuleError {
                index,
                kind: RuleErrorKind::EmptyOutput,
            });
        }
        let matcher = match r.kind {
            Kind::Exact => Matcher::Exact(r.pattern),
            Kind::Wildcard => Matcher::Wildcard(Glob::new(&r.pattern)),
            Kind::Regex => unimplemented!(),
        };
        rules.push(Rule {
            matcher,
            output: r.output,
        });
    }
    if errors.is_empty() {
        Ok(rules)
    } else {
        Err(LoadError::Rules(errors))
    }
}

#[cfg(test)]
mod tests {
    use crate::test_util::out;
    use crate::{Kind, LoadError, Match, RuleError, RuleErrorKind, Rules};

    #[test]
    fn empty_documents_are_valid() {
        for t in ["", "   \n", "# only a comment\n", "[]"] {
            assert!(Rules::from_yaml(t).unwrap().is_empty(), "{t:?}");
        }
    }

    #[test]
    fn exact_first_match_wins() {
        let r = Rules::from_yaml(
            "- {type: exact, match: a.com/x, output: one}\n- {type: exact, match: a.com/y, output: two}\n",
        )
        .unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(out(&r, "a.com/y").as_deref(), Some("two"));
        assert_eq!(out(&r, "a.com/x").as_deref(), Some("one"));
        assert_eq!(
            r.lookup("a.com/y"),
            Some(Match {
                index: 2,
                kind: Kind::Exact,
                pattern: "a.com/y",
                output: "two".into()
            })
        );
        assert_eq!(r.lookup("a.com/xx"), None);
        assert_eq!(r.lookup("a.com/"), None);
    }

    #[test]
    fn exact_star_is_literal() {
        let r = Rules::from_yaml("- {type: exact, match: 'a/*', output: o}\n").unwrap();
        assert_eq!(out(&r, "a/*").as_deref(), Some("o"));
        assert_eq!(r.lookup("a/b"), None);
    }

    #[test]
    fn schema_errors_are_yaml_errors() {
        for t in [
            "- {type: glob, match: a, output: b}",
            "- {type: exact, output: b}",
            "- {type: exact, match: a, output: b, code: 301}",
            "type: exact",
        ] {
            assert!(
                matches!(Rules::from_yaml(t), Err(LoadError::Yaml(_))),
                "{t}"
            );
        }
    }

    #[test]
    fn empty_fields_reported_all_at_once() {
        let e = Rules::from_yaml(
            "- {type: exact, match: '', output: o}\n- {type: exact, match: m, output: ''}\n",
        )
        .unwrap_err();
        assert_eq!(
            e,
            LoadError::Rules(vec![
                RuleError {
                    index: 1,
                    kind: RuleErrorKind::EmptyMatch
                },
                RuleError {
                    index: 2,
                    kind: RuleErrorKind::EmptyOutput
                },
            ])
        );
        assert_eq!(e.to_string(), "rule #1: empty match\nrule #2: empty output");
    }
}
