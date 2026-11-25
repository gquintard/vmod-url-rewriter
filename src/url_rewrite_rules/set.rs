//! Find the lowest-index pattern matching a string, among many, in one pass.
//!
//! All patterns are compiled at load time into one DFA, shared read-only by every thread: a
//! lookup allocates nothing and keeps no per-thread state. An overlapping search reports only the
//! patterns that matched, the lowest index wins.

use std::fmt;

use regex_automata::dfa::{Automaton, OverlappingState, dense, sparse};
use regex_automata::nfa::thompson::{self, WhichCaptures};
use regex_automata::{Input, MatchKind};
use regex_syntax::hir::Hir;

// Thousands of rules need far more than the default NFA size limit (10 MiB).
const NFA_SIZE_LIMIT: usize = 1 << 30;

/// Largest dense DFA kept as is. A bigger one is converted to a sparse DFA, typically 10x smaller
/// for a 3x slower search.
const DENSE_LIMIT: usize = 128 << 20;

enum Dfa {
    Dense(dense::DFA<Vec<u32>>),
    Sparse(sparse::DFA<Vec<u8>>),
}

pub(crate) struct PatternMatcher {
    dfa: Dfa,
}

impl fmt::Debug for PatternMatcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (kind, patterns, bytes) = match &self.dfa {
            Dfa::Dense(d) => ("dense", d.pattern_len(), d.memory_usage()),
            Dfa::Sparse(d) => ("sparse", d.pattern_len(), d.memory_usage()),
        };
        f.debug_struct("PatternMatcher")
            .field("dfa", &kind)
            .field("patterns", &patterns)
            .field("bytes", &bytes)
            .finish()
    }
}

impl PatternMatcher {
    /// Compile `patterns`, failing if building the DFA needs more than `size_limit` bytes.
    pub fn new(patterns: &[Hir], size_limit: usize) -> Result<PatternMatcher, String> {
        PatternMatcher::build(patterns, size_limit, DENSE_LIMIT)
    }

    fn build(
        patterns: &[Hir],
        size_limit: usize,
        dense_limit: usize,
    ) -> Result<PatternMatcher, String> {
        let nfa = thompson::Compiler::new()
            .configure(
                thompson::Config::new()
                    .nfa_size_limit(Some(NFA_SIZE_LIMIT))
                    .which_captures(WhichCaptures::None),
            )
            .build_many_from_hir(patterns)
            .map_err(|e| e.to_string())?;
        let dense = dense::Builder::new()
            .configure(
                dense::Config::new()
                    .match_kind(MatchKind::All)
                    .dfa_size_limit(Some(size_limit))
                    .determinize_size_limit(Some(size_limit)),
            )
            .build_from_nfa(&nfa)
            .map_err(|e| e.to_string())?;
        let dfa = if dense.memory_usage() <= dense_limit {
            Dfa::Dense(dense)
        } else {
            Dfa::Sparse(dense.to_sparse().map_err(|e| e.to_string())?)
        };
        Ok(PatternMatcher { dfa })
    }

    /// Index of the first pattern matching anywhere in `s`.
    pub fn first(&self, s: &str) -> Option<usize> {
        match &self.dfa {
            Dfa::Dense(d) => first(d, s),
            Dfa::Sparse(d) => first(d, s),
        }
    }
}

fn first<A: Automaton>(dfa: &A, s: &str) -> Option<usize> {
    let input = Input::new(s);
    let mut state = OverlappingState::start();
    let mut lowest: Option<usize> = None;
    loop {
        // Only fails on quit bytes, and none are configured: no Unicode word boundary.
        if dfa.try_search_overlapping_fwd(&input, &mut state).is_err() {
            debug_assert!(false, "DFA search failed on {s:?}");
            return None;
        }
        let Some(m) = state.get_match() else {
            return lowest;
        };
        let i = m.pattern().as_usize();
        lowest = Some(lowest.map_or(i, |f| f.min(i)));
    }
}

#[cfg(test)]
mod tests {
    use super::PatternMatcher;
    use crate::url_rewrite_rules::syntax::parse;

    fn matcher(patterns: &[&str]) -> PatternMatcher {
        let hirs: Vec<_> = patterns.iter().map(|p| parse(p).unwrap()).collect();
        PatternMatcher::new(&hirs, 1 << 30).unwrap()
    }

    #[test]
    fn lowest_index_wins_whatever_the_position() {
        let hirs: Vec<_> = ["zzz", "c$", "b", r"\Aabc\z"]
            .iter()
            .map(|p| parse(p).unwrap())
            .collect();
        for dense_limit in [usize::MAX, 0] {
            let m = PatternMatcher::build(&hirs, 1 << 30, dense_limit).unwrap();
            assert_eq!(m.first("abc"), Some(1), "{m:?}");
            assert_eq!(m.first("xbx"), Some(2), "{m:?}");
            assert_eq!(m.first("zzz"), Some(0), "{m:?}");
            assert_eq!(m.first("x"), None, "{m:?}");
        }
    }

    #[test]
    fn many_word_class_patterns_build() {
        let patterns: Vec<String> = (0..2000).map(|i| format!(r"^/re{i}/(\w+)$")).collect();
        let patterns: Vec<&str> = patterns.iter().map(String::as_str).collect();
        let m = matcher(&patterns);
        assert_eq!(m.first("/re1999/x_1"), Some(1999));
        assert_eq!(m.first("/re1999/é"), None);
        assert_eq!(m.first("/re2000/x"), None);
    }

    #[test]
    fn word_boundary_on_non_ascii_text() {
        let m = matcher(&[r"\bx\b", "y"]);
        assert_eq!(m.first("é x é"), Some(0));
        assert_eq!(m.first("éxé"), Some(0));
        assert_eq!(m.first("axb y"), Some(1));
    }
}
