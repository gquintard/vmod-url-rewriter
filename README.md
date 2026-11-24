# vmod_url_rewriter

This is a vmod for [varnish](http://varnish-cache.org/) to rewrite strings with thousands of ordered rules loaded from a YAML file, typically to send redirects. It is meant as a replacement for the unmaintained `vmod-redirect`.

The vmod only manipulates text: a string goes in (usually `req.http.host + req.url`), the output of the first matching rule comes out. Status codes and headers are left to the VCL.

The full VCL API is described in [API.md](API.md).

The matching logic lives in its own module, [`url_rewrite_rules`](src/url_rewrite_rules), independent from Varnish.

Don't hesitate to open github issues if something is unclear or impractical. You can also join us on [discord](https://discord.com/invite/EuwdvbZR6d).

## Version matching

| vmod-url-rewriter | varnish |
| :--------- | :-----: |
| 0.1.0      | 9.0, 9.1 |

## VCL Example

``` vcl
import url_rewriter;

sub vcl_init {
	new rules = url_rewriter.rules("/etc/varnish/redirects.yaml");
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

`lookup()` returns NULL when no rule matches, and assigning NULL to a header sets it to `""`, hence the `!= ""` test. `output` is used as the full `Location`: the VCL doesn't need to know anything about the rules.

## Rules

The rules file is a YAML list of `{type, match, output}` objects:

``` yaml
# strict: the whole string must be equal to `match`
- type: exact
  match: example.com/index.html
  output: https://trololo.com/tralala.php
# wildcard: each `*` matches any run of characters (including none), the rest is literal
- type: wildcard
  match: example.com/trolo/*
  output: https://trololo.com
# regex: unanchored, `$1`/`${name}` in `output` are replaced by the captures
- type: regex
  match: example\.com/index(fr|en)$
  output: https://trololo.com/index$1
```

- Rules are tried **in file order, the first matching rule wins**. Rules hidden by earlier ones are not detected: check the log to see which rule actually matched.
- A wildcard rule drops what the `*` matched: `example.com/trolo/a/b.html` gives `https://trololo.com`. A lone `*` matches everything.
- Regexes use the [`regex`](https://docs.rs/regex) syntax. In `output`, `$1foo` refers to a group named `1foo` (which expands to nothing): write `${1}foo` instead. `$$` is a literal `$`.
- An invalid file fails the VCL load, with every invalid rule listed: unknown `type` or keys, missing or empty `match`/`output`, invalid regex. An empty file is valid and holds no rules.

## Logging

Every load and lookup leaves a `VCL_Log` record, visible with `varnishlog -i VCL_Log`:

```
rules: loaded 3 rules from /etc/varnish/redirects.yaml
rules: rule #3 (regex "example\.com/index(fr|en)$") matched "example.com/indexfr", output "https://trololo.com/indexfr"
rules: no rule matched "example.com/other"
```

## Performance

At load time, every rule becomes one regex: `exact` is escaped and anchored, `wildcard` is
escaped with each `*` turned into `(?s:.*)` and anchored, `regex` is used as-is. All of them are
compiled into one lazy DFA ([`regex-automata`](https://docs.rs/regex-automata)), which an
overlapping search runs over the string once, reporting only the rules that matched; the
lowest-numbered one wins. Only then does a `regex` rule run on its own, to expand its captures.
Lookup time thus barely depends on the number of rules or on the position of the matching rule.

Each concurrent lookup keeps its own DFA cache, filled as states are built, up to 16 MiB or the
lazy DFA's minimum if bigger: thousands of `regex` rules using Unicode classes such as `\w` raise
that minimum (~105 MiB per cache with 20 000 such rules). A Unicode `\b` meeting non-ASCII text
makes the DFA give up: that lookup falls back to the slower PikeVM.

Memory mostly goes to `regex` rules, each compiled a second time on its own for its captures
(without a one-pass DFA, which costs ~670 KiB per rule with `\w`). With
`^example\.com/re{i}/(\w+)$` rules only, the RSS after load is ~110 KiB per rule (2.2 GiB for
20 000 rules, loaded in 14 s); `(?-u:\w)` or `[a-z]` instead of `\w` is far cheaper.

`just bench` (criterion) with 5 000 rules: 80% exact `example.com/page{i}.html`, 15% wildcard
`example.com/dir{i}/*`, 5% regex `^example\.com/re{i}/(\w+)$`.
The miss looks up `example.com/page9999.html`. Compared with the previous linear scan over the
rules (lazy DFA numbers measured on a busier machine):

| benchmark          | linear scan | lazy DFA |
| :----------------- | ----------: | -------: |
| `load/5000`        | 213 ms      | 210 ms   |
| `lookup/hit_first` | 302 ns      | 479 ns   |
| `lookup/hit_last`  | 236 µs      | 104 ns   |
| `lookup/miss`      | 230 µs      | 109 ns   |

## Requirements

You'll need:
- `cargo` (and the accompanying `rust` package, 1.90 or newer)
- `clang`
- the `varnish` development libraries/headers ([depends on the `varnish` crate you are using](https://github.com/varnish-rs/varnish-rs#versions))

## Build and test

With `cargo` only:

``` bash
cargo build --release
cargo test --release
```

Build before testing: the VTC tests load the `.so` file produced by `cargo build`.

The vmod file will be found at `target/release/libvmod_url_rewriter.so`.
