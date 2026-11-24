//! Find the lowest-index pattern matching a string, among many, in one pass.
//!
//! [`regex::RegexSet`] would do, but it reports matches as one flag per pattern: finding the
//! first one means scanning every flag, which dominates with thousands of patterns. An
//! overlapping search on the lazy DFA instead reports only the patterns that matched.

use std::fmt;

use regex_automata::hybrid::dfa::{self, DFA, OverlappingState};
use regex_automata::nfa::thompson::{self, NFA, WhichCaptures, pikevm};
use regex_automata::util::pool::Pool;
use regex_automata::{Input, MatchError, MatchKind, PatternSet};

// Thousands of rules need far more than the default NFA size limit (10 MiB).
const NFA_SIZE_LIMIT: usize = 1 << 30;
// Per thread doing lookups; allocated lazily as DFA states get built.
const DFA_CACHE_CAPACITY: usize = 16 << 20;

type CacheFn<T> = Box<dyn Fn() -> T + Send + Sync>;

pub(crate) struct PatternMatcher {
    dfa: DFA,
    dfa_caches: Pool<dfa::Cache, CacheFn<dfa::Cache>>,
    /// Used when the lazy DFA can't answer, e.g. a Unicode `\b` meeting non-ASCII text.
    pikevm: pikevm::PikeVM,
    pikevm_caches: Pool<pikevm::Cache, CacheFn<pikevm::Cache>>,
}

impl fmt::Debug for PatternMatcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PatternMatcher")
            .field("patterns", &self.dfa.pattern_len())
            .finish_non_exhaustive()
    }
}

impl PatternMatcher {
    /// Compile `patterns`, with the syntax of [`regex::Regex`].
    pub fn new<P: AsRef<str>>(patterns: &[P]) -> Result<PatternMatcher, String> {
        let nfa: NFA = thompson::Compiler::new()
            .configure(
                thompson::Config::new()
                    .nfa_size_limit(Some(NFA_SIZE_LIMIT))
                    .which_captures(WhichCaptures::None),
            )
            .build_many(patterns)
            .map_err(|e| e.to_string())?;
        let dfa = DFA::builder()
            .configure(
                DFA::config()
                    .match_kind(MatchKind::All)
                    .cache_capacity(DFA_CACHE_CAPACITY)
                    .unicode_word_boundary(true),
            )
            .build_from_nfa(nfa.clone())
            .map_err(|e| e.to_string())?;
        let pikevm = pikevm::PikeVM::builder()
            .configure(pikevm::PikeVM::config().match_kind(MatchKind::All))
            .build_from_nfa(nfa)
            .map_err(|e| e.to_string())?;
        let d = dfa.clone();
        let p = pikevm.clone();
        Ok(PatternMatcher {
            dfa_caches: Pool::new(Box::new(move || d.create_cache())),
            dfa,
            pikevm_caches: Pool::new(Box::new(move || p.create_cache())),
            pikevm,
        })
    }

    /// Index of the first pattern matching anywhere in `s`.
    pub fn first(&self, s: &str) -> Option<usize> {
        self.first_dfa(s).unwrap_or_else(|_| self.first_pikevm(s))
    }

    /// Fails if the DFA had to give up.
    fn first_dfa(&self, s: &str) -> Result<Option<usize>, MatchError> {
        let mut cache = self.dfa_caches.get();
        let input = Input::new(s);
        let mut state = OverlappingState::start();
        let mut first: Option<usize> = None;
        loop {
            self.dfa
                .try_search_overlapping_fwd(&mut cache, &input, &mut state)?;
            let Some(m) = state.get_match() else {
                return Ok(first);
            };
            let i = m.pattern().as_usize();
            first = Some(first.map_or(i, |f| f.min(i)));
        }
    }

    fn first_pikevm(&self, s: &str) -> Option<usize> {
        let mut cache = self.pikevm_caches.get();
        let mut set = PatternSet::new(self.pikevm.pattern_len());
        self.pikevm
            .which_overlapping_matches(&mut cache, &Input::new(s), &mut set);
        set.iter().next().map(|p| p.as_usize())
    }
}

#[cfg(test)]
mod tests {
    use super::PatternMatcher;

    #[test]
    fn lowest_index_wins_whatever_the_position() {
        let m = PatternMatcher::new(&["zzz", "c$", "b", r"\Aabc\z"]).unwrap();
        assert_eq!(m.first("abc"), Some(1));
        assert_eq!(m.first("xbx"), Some(2));
        assert_eq!(m.first("zzz"), Some(0));
        assert_eq!(m.first("x"), None);
    }

    #[test]
    fn unicode_word_boundary_falls_back() {
        let m = PatternMatcher::new(&[r"\bé\b", "x"]).unwrap();
        assert_eq!(m.first("a é b"), Some(0));
        assert_eq!(m.first("aéb x"), Some(1));
        assert_eq!(m.first("aéb"), None);
    }
}
