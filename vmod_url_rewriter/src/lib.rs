// Doc comments below document the VCL interface (they generate API.md), not a Rust API.
#![allow(
    clippy::must_use_candidate,
    clippy::missing_errors_doc,
    clippy::self_named_constructors
)]

use url_rewrite_rules::Rules as RuleSet;

varnish::run_vtc_tests!("tests/*.vtc");

/// An ordered list of rewrite rules loaded from a YAML file, where the first matching rule wins.
pub struct Rules {
    name: String,
    rules: RuleSet,
}

/// Rewrite strings with ordered exact, wildcard and regex rules, typically to send redirects.
///
/// The vmod only manipulates text: VCL decides what string to look up, and what to do with the
/// result (status code, `Location` header...).
///
/// ```vcl
/// import url_rewriter;
///
/// sub vcl_init {
///     new rules = url_rewriter.rules("/etc/varnish/redirects.yaml");
/// }
///
/// sub vcl_recv {
///     set req.http.x-redir = rules.lookup(req.http.host + req.url);
///     if (req.http.x-redir != "") {
///         return (synth(301));
///     }
/// }
///
/// sub vcl_synth {
///     if (resp.status == 301 && req.http.x-redir != "") {
///         set resp.http.location = req.http.x-redir;
///         return (deliver);
///     }
/// }
/// ```
///
/// The rules file is a YAML list of `{type, match, output}` objects, tried in order:
///
/// ```yaml
/// - type: exact          # the whole string must be equal to `match`
///   match: example.com/index.html
///   output: https://trololo.com/tralala.php
/// - type: wildcard       # each `*` matches any run of characters, the rest is literal
///   match: example.com/trolo/*
///   output: https://trololo.com
/// - type: regex          # unanchored, `$1`/`${name}` in `output` are replaced by captures
///   match: example\.com/index(fr|en)$
///   output: https://trololo.com/index$1
/// ```
///
/// In a regex `output`, `$1foo` refers to a group named `1foo` (which expands to nothing), write
/// `${1}foo` instead. `$$` is a literal `$`.
#[varnish::vmod(docs = "API.md")]
mod url_rewriter {
    use varnish::vcl::{Ctx, LogTag, VclError};

    use super::{RuleSet, Rules};

    impl Rules {
        /// Load the rules from the YAML file at `path`, used as given: a relative path is
        /// resolved against varnishd's working directory.
        ///
        /// The file is read once, when the VCL loads. An unreadable file, invalid YAML, or any
        /// invalid rule (unknown type, missing or empty field, invalid regex) fails the VCL
        /// load, listing every invalid rule.
        pub fn rules(ctx: &mut Ctx, #[vcl_name] name: &str, path: &str) -> Result<Self, VclError> {
            let text = std::fs::read_to_string(path)
                .map_err(|e| VclError::new(format!("{name}: cannot read {path}: {e}")))?;
            let rules = RuleSet::from_yaml(&text)
                .map_err(|e| VclError::new(format!("{name}: {path}: {e}")))?;
            ctx.log(
                LogTag::VclLog,
                format!("{name}: loaded {} rules from {path}", rules.len()),
            );
            Ok(Self {
                name: name.to_owned(),
                rules,
            })
        }

        /// Return the `output` of the first rule matching `s`, or NULL if none does (or if `s`
        /// is NULL). Assigned to a header, NULL becomes an empty string.
        ///
        /// Every call logs a `VCL_Log` record telling which rule matched, or that none did:
        /// `rules: rule #2 (wildcard "example.com/trolo/*") matched "example.com/trolo/a",
        /// output "https://trololo.com"`.
        pub fn lookup(&self, ctx: &mut Ctx, #[required] s: Option<&str>) -> Option<String> {
            let name = &self.name;
            let Some(s) = s else {
                ctx.log(LogTag::VclLog, format!("{name}: no rule matched NULL"));
                return None;
            };
            let Some(m) = self.rules.lookup(s) else {
                ctx.log(LogTag::VclLog, format!("{name}: no rule matched \"{s}\""));
                return None;
            };
            ctx.log(
                LogTag::VclLog,
                format!(
                    "{name}: rule #{} ({} \"{}\") matched \"{s}\", output \"{}\"",
                    m.index, m.kind, m.pattern, m.output
                ),
            );
            Some(m.output.into_owned())
        }
    }
}
