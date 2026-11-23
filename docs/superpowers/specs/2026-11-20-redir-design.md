# vmod-redir: design

## Intent

Move thousands of redirects into Varnish. The historical `vmod-redirect` did this but is no longer
maintained. `vmod-redir` is a Rust replacement, built on varnish-rs: it loads a list of rewrite
rules and, given a string, returns the rewritten string. VCL uses that result to send a redirect.

The vmod only manipulates text. It knows nothing about HTTP, status codes, hosts or URLs.

## Goals

- Three rule types, from the requester's scenarios:
  - **exact**: `example.com/index.html` → `https://trololo.com/tralala.php`
  - **wildcard**: `example.com/trolo/*` → `https://trololo.com`
  - **regex**: `example.com/index(fr|en)$` → `https://trololo.com/index$1`
- `output` is the complete redirect target: VCL copies it into `Location` untouched, so it knows as
  little as possible about the rules (requester's call).
- Rules come from a YAML file: an array of `{type, match, output}` objects.
- First matching rule wins, in file order.
- Handles thousands of rules.
- Bad rule files are rejected loudly at VCL load time, never half-loaded.
- Every lookup leaves a log record saying which rule matched, or that none did.

## Non-goals

- Status codes (301/302/308...): VCL picks them.
- Deciding what string to match: VCL builds it (e.g. `req.http.host + req.url`), so whether the
  host or the query string is included is the VCL writer's choice.
- Reloading the rules without a VCL reload.
- CSV/JSON input, or adding rules from VCL calls.
- An indexed/compiled matcher. Linear scan was chosen; speed-ups are a later concern.
- Detecting rules that can never match because an earlier rule hides them. Keep the code simple
  and trust the user (requester's call); the per-lookup log shows which rule actually won.

## Anti-goals

- Rules matching in a different order than the file says.
- A VCL that loads with only part of the rules.

## Constraints

- Rust, varnish-rs (`varnish = "0.8"`), Varnish 9.0 / 9.1, same as `vmod-acls`.
- Follow `~/work/vmod-acls` conventions: cargo workspace (edition 2024, BSD-3-Clause, pedantic
  clippy), a `justfile` copied from it, `API.md` generated from doc comments
  (`#[varnish::vmod(docs = "API.md")]`), VTC tests via `varnish::run_vtc_tests!("tests/*.vtc")`.

## Approaches considered

| Approach | Trade-off |
|---|---|
| **A. Linear scan (picked)** | Check rules in file order, stop at the first hit. Simplest and obviously correct. Cost grows with rule count: a miss runs every rule. |
| B. Indexed | Hash map for exact, prefix tree for wildcards, `RegexSet` for regexes, keep the lowest position. Flat cost, more code. |
| C. One giant regex | Alternation already prefers earlier alternatives, but thousands of capture groups blow regex size limits and slow lookups. |

The requester picked A. The matching crate's public API must not expose the scan, so B can be
swapped in later without touching the vmod or the VCL API.

## Decided

### Layout (builder's call, approved as part of the approach)

```
vmod-redir/
  Cargo.toml          # workspace
  justfile
  redir-rules/        # pure Rust: parse YAML, match, rewrite. No varnish dependency.
  vmod_redir/         # thin vmod, VCL module name `redir`
    API.md
    tests/*.vtc
```

### Rule file

```yaml
- type: exact
  match: example.com/index.html
  output: https://trololo.com/tralala.php
- type: wildcard
  match: example.com/trolo/*
  output: https://trololo.com
- type: regex
  match: example\.com/index(fr|en)$
  output: https://trololo.com/index$1
```

- `type`, `match`, `output` are all required strings. Unknown keys are an error.
- An empty file or an empty array is valid and matches nothing.
- An empty `match` or an empty `output` is an error (my call: both are almost certainly mistakes,
  and VCL can't tell an empty output from no match).
- `match: "*"` is a valid wildcard matching everything, typically a last-rule fallback (my call).
- A regex whose expanded output is empty (e.g. `$1` with an empty group) still counts as a match
  and returns `""`. VCL sees that the same as no match; the log record tells them apart (my call).

### Matching (the input string is called `s`)

- **exact**: matches when `s == match`. `*` has no special meaning.
- **wildcard**: glob-style. `match` may contain any number of `*`, anywhere; each matches any run
  of characters, including none. Everything else is literal and the whole of `s` must be matched
  (so `example.com/trolo/*` matches `example.com/trolo/` and `example.com/trolo/a/b.html`, but not
  `example.com/trolo`; `*.example.com/old/*` matches any subdomain). There is no escape: a literal
  `*` needs an `exact` or `regex` rule. The output is returned as-is; what the stars matched is
  dropped (requester's call, "for now": if carrying text over comes later, stars would become `$1`,
  `$2`... like regex captures).
- **regex**: Rust `regex` crate syntax, used as written, i.e. unanchored search like VCL's `~`
  (write `^`/`$` to anchor). The result is `output` with `$1`, `${1}`, `${name}` filled from the
  captures (`Captures::expand` semantics: `$$` is a literal `$`, unknown groups expand to empty). The
  result is the expanded `output` only, not a substitution inside `s`.
- Rules are tried in file order and the first match wins (requester's call).

### Errors

Any invalid rule fails the whole load: an unknown type, a missing or empty field, an invalid regex,
or a file that is unreadable or not valid YAML. The error names the file, the rule's position
(1-based index, plus the YAML line if the parser provides it) and the reason, and it ends up as the
VCL load error.

The load reports every invalid rule at once, not just the first, so a large migrated file can be
fixed in one pass (my call).

### Logging

Every `.lookup()` writes one `VCL_Log` record (visible in `varnishlog` by default, like
`std.log()`), prefixed with the object name:

- hit: `rules: rule #3 (wildcard "example.com/trolo/*") matched "example.com/trolo/a.html", output "https://trololo.com"`
- miss: `rules: no rule matched "example.com/nope"`

At load, one record gives the number of rules and the file.

### VCL API

```vcl
import redir;

sub vcl_init {
    new rules = redir.rules("/etc/varnish/redirects.yaml");
}

sub vcl_recv {
    set req.http.x-redir = rules.lookup(req.http.host + req.url);
    if (req.http.x-redir != "") {
        return (synth(301));
    }
}

sub vcl_synth {
    if (resp.status == 301 && req.http.x-redir != "") {
        set resp.http.location = req.http.x-redir;
        return (deliver);
    }
}
```

- `redir.rules(STRING path)`: object constructor. It reads and validates the file once, in
  `vcl_init`.
- `.lookup(STRING s)`: returns the rewritten string, or NULL when no rule matches (NULL input
  also gives NULL). Assigned to a header, NULL becomes the empty string, which is still true in a
  boolean test, hence the `!= ""` checks above.

## Left to the builder

- The YAML crate (`serde_yaml` is unmaintained; pick a maintained serde-compatible one) and whether
  it gives line numbers.
- Extra convenience methods (e.g. a `.found(s)` boolean, or a rule count for logging) if they fall
  out naturally.
- The test plan: unit tests in `redir-rules` (each type, order, errors, capture expansion) and VTC
  tests in `vmod_redir` (load ok, load failure, lookup hit/miss, log records). A small criterion
  bench with a few thousand rules, to know what linear scan costs.
- README with a version-matching table, as in `vmod-acls`.
