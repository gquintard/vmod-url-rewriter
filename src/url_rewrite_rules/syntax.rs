//! Parse rule patterns into [`Hir`], with ASCII-only Perl classes and word boundaries.
//!
//! Unicode `\w`, `\d`, `\s` and `\b` would make the precomputed DFA much bigger and slower to
//! build (~300 KiB and ~2.5 ms per rule using `\w`). They are rewritten to their ASCII
//! meaning, whatever the flags: `\w` is `[0-9A-Za-z_]`, `\d` is `[0-9]`, `\s` is
//! `[\t\n\v\f\r ]`, and `\b` and friends use ASCII word characters. Everything else (`.`,
//! literals, `\p{..}`, case folding) stays Unicode-aware.

use regex_syntax::ast::{
    self, AssertionKind, Ast, ClassAscii, ClassAsciiKind, ClassBracketed, ClassPerl, ClassPerlKind,
    ClassSet, ClassSetItem, Flag, Flags, FlagsItem, FlagsItemKind, GroupKind,
};
use regex_syntax::hir::{Hir, translate::Translator};

/// Parse `pattern` with the [`regex`](https://docs.rs/regex) syntax.
///
/// The error is meant for humans, pointing at the faulty part of `pattern`.
pub(crate) fn parse(pattern: &str) -> Result<Hir, String> {
    let mut tree = ast::parse::Parser::new()
        .parse(pattern)
        .map_err(|e| e.to_string())?;
    asciify(&mut tree);
    // The rewritten nodes keep their original spans, so errors still point into `pattern`.
    Translator::new()
        .translate(pattern, &tree)
        .map_err(|e| e.to_string())
}

fn ascii_kind(kind: &ClassPerlKind) -> ClassAsciiKind {
    match kind {
        ClassPerlKind::Digit => ClassAsciiKind::Digit,
        ClassPerlKind::Space => ClassAsciiKind::Space,
        ClassPerlKind::Word => ClassAsciiKind::Word,
    }
}

/// `\w` → `[[:word:]]`, `\W` → `[^[:word:]]`, and alike.
fn perl_to_bracketed(c: &ClassPerl) -> ClassBracketed {
    ClassBracketed {
        span: c.span,
        negated: c.negated,
        kind: ClassSet::Item(ClassSetItem::Ascii(ClassAscii {
            span: c.span,
            kind: ascii_kind(&c.kind),
            negated: false,
        })),
    }
}

fn asciify(tree: &mut Ast) {
    match tree {
        Ast::ClassPerl(c) => *tree = Ast::class_bracketed(perl_to_bracketed(c)),
        Ast::ClassBracketed(c) => asciify_set(&mut c.kind),
        Ast::Assertion(a) if is_word_boundary(&a.kind) => {
            let span = a.span;
            let assertion = std::mem::replace(tree, Ast::empty(span));
            // `(?-u:\b)`
            *tree = Ast::group(ast::Group {
                span,
                kind: GroupKind::NonCapturing(Flags {
                    span,
                    items: vec![
                        FlagsItem {
                            span,
                            kind: FlagsItemKind::Negation,
                        },
                        FlagsItem {
                            span,
                            kind: FlagsItemKind::Flag(Flag::Unicode),
                        },
                    ],
                }),
                ast: Box::new(assertion),
            });
        }
        Ast::Repetition(r) => asciify(&mut r.ast),
        Ast::Group(g) => asciify(&mut g.ast),
        Ast::Alternation(a) => a.asts.iter_mut().for_each(asciify),
        Ast::Concat(c) => c.asts.iter_mut().for_each(asciify),
        Ast::Empty(_)
        | Ast::Flags(_)
        | Ast::Literal(_)
        | Ast::Dot(_)
        | Ast::Assertion(_)
        | Ast::ClassUnicode(_) => {}
    }
}

fn asciify_set(set: &mut ClassSet) {
    match set {
        ClassSet::Item(item) => asciify_item(item),
        ClassSet::BinaryOp(op) => {
            asciify_set(&mut op.lhs);
            asciify_set(&mut op.rhs);
        }
    }
}

fn asciify_item(item: &mut ClassSetItem) {
    match item {
        // Inside brackets, `\W` can't become `[:^word:]`: that is the complement within ASCII.
        ClassSetItem::Perl(c) => *item = ClassSetItem::Bracketed(Box::new(perl_to_bracketed(c))),
        ClassSetItem::Bracketed(c) => asciify_set(&mut c.kind),
        ClassSetItem::Union(u) => u.items.iter_mut().for_each(asciify_item),
        ClassSetItem::Empty(_)
        | ClassSetItem::Literal(_)
        | ClassSetItem::Range(_)
        | ClassSetItem::Ascii(_)
        | ClassSetItem::Unicode(_) => {}
    }
}

fn is_word_boundary(kind: &AssertionKind) -> bool {
    !matches!(
        kind,
        AssertionKind::StartLine
            | AssertionKind::EndLine
            | AssertionKind::StartText
            | AssertionKind::EndText
    )
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn error_points_into_the_original_pattern() {
        let e = parse(r"\w(").unwrap_err();
        assert!(e.contains(r"\w("), "{e}");
        let e = parse(r"\w\p{Nope}").unwrap_err();
        assert!(e.contains(r"\w\p{Nope}") && e.contains("^^^^^^^^"), "{e}");
    }
}
