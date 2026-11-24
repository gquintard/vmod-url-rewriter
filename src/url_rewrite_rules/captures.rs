//! Expand a `regex` rule's captures into its `output`, once the rule is known to match.
//!
//! Search scratch space is allocated per call: per-thread caches don't scale to thousands of
//! Varnish threads. The bounded backtracker needs the least of it on short inputs (a bit per NFA
//! state and input byte); longer inputs fall back to the `PikeVM`.

use regex_automata::nfa::thompson::{self, backtrack::BoundedBacktracker, pikevm::PikeVM};
use regex_syntax::hir::Hir;

#[derive(Debug)]
pub(crate) struct Expander {
    backtracker: BoundedBacktracker,
    pikevm: PikeVM,
}

impl Expander {
    pub fn new(hir: &Hir) -> Result<Expander, String> {
        let nfa = thompson::Compiler::new()
            .build_from_hir(hir)
            .map_err(|e| e.to_string())?;
        Ok(Expander {
            backtracker: BoundedBacktracker::new_from_nfa(nfa.clone())
                .map_err(|e| e.to_string())?,
            pikevm: PikeVM::new_from_nfa(nfa).map_err(|e| e.to_string())?,
        })
    }

    /// `template` with `$1`, `${name}`... replaced by the captures of the leftmost-first match in
    /// `s`, or `None` if there is no match.
    pub fn expand(&self, s: &str, template: &str) -> Option<String> {
        let mut caps = self.pikevm.create_captures();
        if s.len() <= self.backtracker.max_haystack_len() {
            let mut cache = self.backtracker.create_cache();
            // Can't fail: the haystack length was checked.
            self.backtracker
                .try_captures(&mut cache, s, &mut caps)
                .ok()?;
        } else {
            let mut cache = self.pikevm.create_cache();
            self.pikevm.captures(&mut cache, s, &mut caps);
        }
        if !caps.is_match() {
            return None;
        }
        let mut expanded = String::new();
        caps.interpolate_string_into(s, template, &mut expanded);
        Some(expanded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expander(pattern: &str) -> Expander {
        Expander::new(&regex_syntax::parse(pattern).unwrap()).unwrap()
    }

    #[test]
    fn long_input_falls_back_to_pikevm() {
        let e = expander(r"^(a+)(b)$");
        let s = format!("{}b", "a".repeat(e.backtracker.max_haystack_len()));
        assert_eq!(e.expand(&s, "$2").as_deref(), Some("b"));
        assert_eq!(e.expand("aab", "$1-$2").as_deref(), Some("aa-b"));
        assert_eq!(e.expand("ab!", "$1"), None);
    }
}
