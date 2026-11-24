# vmod_url_rewriter

This is a vmod for [varnish](http://varnish-cache.org/) to rewrite strings with thousands of ordered rules loaded from a YAML file, typically to send redirects. It is meant as a replacement for the unmaintained `vmod-redirect`.

The vmod only manipulates text: a string goes in (usually `req.http.host + req.url`), the output of the first matching rule comes out. Status codes and headers are left to the VCL.

The full VCL API is described in [API.md](vmod_url_rewriter/API.md).

The matching logic lives in its own crate, [`url-rewrite-rules`](url-rewrite-rules), so other Rust projects and vmods can use it too.

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

Lookups are a linear scan. With 5 000 rules, a miss (or a hit on the last rule) costs about 110 µs and loading about 120 ms, see [url-rewrite-rules](url-rewrite-rules/README.md) for details.

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
