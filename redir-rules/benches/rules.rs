//! Load and lookup costs for `Rules` with 5 000 rules (80% exact, 15% wildcard, 5% regex).
//!
//! Run with `just bench`, or e.g. `just bench lookup/miss` to filter.

use std::fmt::Write;
use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use redir_rules::Rules;

const N: usize = 5_000;

fn rules_yaml() -> String {
    let mut yaml = String::new();
    for i in 0..N {
        match i % 20 {
            0 => writeln!(
                yaml,
                r"- {{type: regex, match: '^example\.com/re{i}/(\w+)$', output: 'https://new.example.com/re/$1'}}"
            ),
            1..=3 => writeln!(
                yaml,
                "- {{type: wildcard, match: 'example.com/dir{i}/*', output: 'https://new.example.com/dir{i}'}}"
            ),
            _ => writeln!(
                yaml,
                "- {{type: exact, match: 'example.com/page{i}.html', output: 'https://new.example.com/page{i}'}}"
            ),
        }
        .unwrap();
    }
    yaml
}

fn bench(c: &mut Criterion) {
    let yaml = rules_yaml();
    c.bench_function("load/5000", |b| {
        b.iter(|| Rules::from_yaml(black_box(&yaml)).unwrap());
    });

    let rules = Rules::from_yaml(&yaml).unwrap();
    let last = format!("example.com/page{}.html", N - 1);
    assert_eq!(rules.lookup("example.com/re0/x").unwrap().index, 1);
    assert_eq!(rules.lookup(&last).unwrap().index, N);
    assert!(rules.lookup("example.com/nothing").is_none());

    let mut g = c.benchmark_group("lookup");
    g.bench_function("hit_first", |b| {
        b.iter(|| rules.lookup(black_box("example.com/re0/x")));
    });
    g.bench_function("hit_last", |b| b.iter(|| rules.lookup(black_box(&last))));
    g.bench_function("miss", |b| {
        b.iter(|| rules.lookup(black_box("example.com/nothing")));
    });
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
