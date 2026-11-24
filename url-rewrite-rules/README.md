# url-rewrite-rules

Ordered string rewrite rules (exact, wildcard, regex) loaded from YAML, where **the first matching rule wins**.

```rust
use url_rewrite_rules::Rules;

let rules = Rules::from_yaml(r"
- {type: exact, match: example.com/index.html, output: 'https://trololo.com/tralala.php'}
- {type: wildcard, match: 'example.com/trolo/*', output: 'https://trololo.com'}
- {type: regex, match: 'example\.com/index(fr|en)$', output: 'https://trololo.com/index$1'}
").unwrap();

let m = rules.lookup("example.com/indexfr").unwrap();
assert_eq!(m.index, 3);
assert_eq!(m.output, "https://trololo.com/indexfr");
assert!(rules.lookup("example.com/other").is_none());
```

- `exact`: the whole string must be equal to `match`.
- `wildcard`: each `*` matches any run of characters (including none), the rest is literal. What the
  `*` matched is dropped, `output` is returned as-is.
- `regex`: [`regex`](https://docs.rs/regex) syntax, unanchored; `output` is expanded with
  `Captures::expand` (`$1`, `${name}`, `$$`).
- `from_yaml` validates every rule and reports all invalid ones at once (`LoadError`).
- Lookups are a linear scan over the rules, in order.

Used by [`vmod_url_rewriter`](../vmod_url_rewriter) to expose these rules to Varnish VCL.

## Benchmarks

`just bench` (criterion) with 5 000 rules: 80% exact `example.com/page{i}.html`, 15% wildcard
`example.com/dir{i}/*`, 5% regex `^example\.com/re{i}/(\w+)$`.
The miss looks up `example.com/page9999.html`: a miss scans every rule, like a hit on the last one.

| benchmark          | time     |
| :----------------- | -------: |
| `load/5000`        | 117 ms   |
| `lookup/hit_first` | 127 ns   |
| `lookup/hit_last`  | 109 µs   |
| `lookup/miss`      | 111 µs   |
