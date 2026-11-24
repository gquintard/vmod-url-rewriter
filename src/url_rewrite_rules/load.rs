use regex_syntax::hir::Hir;
use serde::Deserialize;

use crate::url_rewrite_rules::captures::LazyExpander;
use crate::url_rewrite_rules::set::PatternMatcher;
use crate::url_rewrite_rules::{Kind, LoadError, Rule, RuleError, RuleErrorKind, glob, syntax};

#[derive(Deserialize)]
#[serde(deny_unknown_fields, expecting = "a {type, match, output} object")]
struct RawRule {
    #[serde(rename = "type")]
    kind: Kind,
    #[serde(rename = "match")]
    pattern: Option<String>,
    output: Option<String>,
}

/// Parse and validate `text`, and compile every rule into one matcher, pattern `i` being rule `i`,
/// using at most `size_limit` bytes to build its DFA.
pub(crate) fn load(
    text: &str,
    size_limit: usize,
) -> Result<(PatternMatcher, Vec<Rule>), LoadError> {
    // A null document (empty or comment-only file) is zero rules.
    // Each rule is deserialized on its own, so that all invalid rules are reported.
    let raw: Option<Vec<serde_norway::Value>> =
        serde_norway::from_str(text).map_err(|e| LoadError::Yaml(e.to_string()))?;
    let mut rules = Vec::new();
    let mut hirs = Vec::new();
    let mut errors = Vec::new();
    for (i, v) in raw.unwrap_or_default().into_iter().enumerate() {
        let index = i + 1;
        let r: RawRule = match serde_norway::from_value(v) {
            Ok(r) => r,
            Err(e) => {
                errors.push(RuleError {
                    index,
                    kind: RuleErrorKind::Invalid(e.to_string()),
                });
                continue;
            }
        };
        let pattern = r.pattern.unwrap_or_default();
        let output = r.output.unwrap_or_default();
        if pattern.is_empty() {
            errors.push(RuleError {
                index,
                kind: RuleErrorKind::EmptyMatch,
            });
        }
        if output.is_empty() {
            errors.push(RuleError {
                index,
                kind: RuleErrorKind::EmptyOutput,
            });
        }
        let parsed = match r.kind {
            Kind::Exact => syntax::parse(&glob::exact_to_regex(&pattern)),
            Kind::Wildcard => syntax::parse(&glob::to_regex(&pattern)),
            Kind::Regex => syntax::parse(&pattern),
        };
        let hir = match parsed {
            Ok(hir) => hir,
            Err(e) => {
                errors.push(RuleError {
                    index,
                    kind: RuleErrorKind::InvalidRegex(e),
                });
                continue;
            }
        };
        // Without a `$`, `output` is used as is.
        let captures = (r.kind == Kind::Regex && output.contains('$')).then(LazyExpander::default);
        hirs.push(hir);
        rules.push(Rule {
            kind: r.kind,
            pattern,
            output,
            captures,
        });
    }
    if !errors.is_empty() {
        return Err(LoadError::Rules(errors));
    }
    match PatternMatcher::new(&hirs, size_limit) {
        Ok(matcher) => Ok((matcher, rules)),
        Err(e) => Err(too_complex(&rules, &hirs, size_limit).unwrap_or(LoadError::Compile(e))),
    }
}

/// The `regex` rules that can't be compiled even alone, if any.
fn too_complex(rules: &[Rule], hirs: &[Hir], size_limit: usize) -> Option<LoadError> {
    let errors: Vec<RuleError> = rules
        .iter()
        .zip(hirs)
        .enumerate()
        .filter(|(_, (rule, hir))| {
            rule.kind == Kind::Regex
                && PatternMatcher::new(std::slice::from_ref(*hir), size_limit).is_err()
        })
        .map(|(i, _)| RuleError {
            index: i + 1,
            kind: RuleErrorKind::TooComplex,
        })
        .collect();
    (!errors.is_empty()).then_some(LoadError::Rules(errors))
}

#[cfg(test)]
mod tests {
    use crate::url_rewrite_rules::test_util::out;
    use crate::url_rewrite_rules::{Kind, LoadError, Match, RuleError, RuleErrorKind, Rules};

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
    fn non_list_is_a_yaml_error() {
        for t in ["type: exact", "- [unclosed"] {
            assert!(
                matches!(Rules::from_yaml(t), Err(LoadError::Yaml(_))),
                "{t}"
            );
        }
    }

    #[test]
    fn schema_errors_are_rule_errors() {
        for t in [
            "- {type: glob, match: a, output: b}",
            "- {match: a, output: b}",
            "- {type: exact, match: a, output: b, code: 301}",
            "- foo",
        ] {
            let e = Rules::from_yaml(t).unwrap_err();
            assert!(
                matches!(&e, LoadError::Rules(v) if v.len() == 1 && v[0].index == 1
                    && matches!(v[0].kind, RuleErrorKind::Invalid(_))),
                "{t}: {e:?}"
            );
            assert!(e.to_string().starts_with("rule #1: invalid rule: "), "{e}");
        }
    }

    #[test]
    fn schema_and_regex_errors_reported_together() {
        let e = Rules::from_yaml(
            "- {type: exact, match: a, output: b}\n- {type: regex, match: '(', output: b}\n- {type: glob, match: a, output: b}\n",
        )
        .unwrap_err();
        let LoadError::Rules(v) = e else {
            panic!("{e:?}")
        };
        assert_eq!(v.iter().map(|e| e.index).collect::<Vec<_>>(), [2, 3]);
        assert!(matches!(v[0].kind, RuleErrorKind::InvalidRegex(_)));
        assert!(matches!(v[1].kind, RuleErrorKind::Invalid(_)));
    }

    #[test]
    fn missing_or_null_fields_are_empty() {
        for (t, kind) in [
            ("- {type: exact, output: b}", RuleErrorKind::EmptyMatch),
            (
                "- {type: exact, match: ~, output: b}",
                RuleErrorKind::EmptyMatch,
            ),
            ("- {type: exact, match: a}", RuleErrorKind::EmptyOutput),
            (
                "- {type: exact, match: a, output: ~}",
                RuleErrorKind::EmptyOutput,
            ),
            (
                "- {type: exact, match: a, output: null}",
                RuleErrorKind::EmptyOutput,
            ),
        ] {
            assert_eq!(
                Rules::from_yaml(t).err(),
                Some(LoadError::Rules(vec![RuleError { index: 1, kind }])),
                "{t}"
            );
        }
    }

    #[test]
    fn too_complex_regex_is_rule_error() {
        let text = "- {type: exact, match: a, output: o}\n- {type: regex, match: '(a|b)*a(a|b){16}', output: o}\n- {type: regex, match: 'x', output: o}\n";
        let Err(LoadError::Rules(v)) = Rules::from_yaml_with_limit(text, 1 << 20) else {
            panic!("expected rule errors")
        };
        assert_eq!(
            v,
            [RuleError {
                index: 2,
                kind: RuleErrorKind::TooComplex
            }]
        );
        assert_eq!(v[0].to_string(), "rule #2: regex too complex to precompute");
        assert!(Rules::from_yaml(text).is_ok());
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
