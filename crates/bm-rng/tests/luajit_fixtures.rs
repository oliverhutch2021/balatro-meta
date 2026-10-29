//! Value-for-value checks against LuaJIT.
//!
//! `tests/fixtures/rng/reference.tsv` is produced by running
//! `reference/rng_reference.lua` under LuaJIT 2.1 (`make rng-fixtures`). Every
//! value is compared bit for bit.

use bm_rng::{pseudohash, round13, shuffle, LuaRandom, Streams};

const KEYS: &[&str] = &[
    "shuffle",
    "nr1",
    "nr2",
    "cdt1",
    "rarity1sho",
    "Joker11sho",
    "edisho1",
    "lucky_mult",
    "lucky_money",
    "wheel_of_fortune",
    "boss",
    "Tag1",
    "stdset1",
    "front",
    "erratic",
    "misprint",
    "glass",
    "hook",
    "cavendish",
    "gros_michel",
    "Voucher1",
    "soul_Tarot1",
    "",
    "x",
];

/// The committed fixture, or a bigger one named by `BM_RNG_FIXTURE`
/// (`make rng-stress`).
fn fixture() -> String {
    let default = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/rng/reference.tsv"
    );
    let path = std::env::var("BM_RNG_FIXTURE").unwrap_or_else(|_| default.to_string());
    std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!("missing {path}: run `make rng-fixtures` to create the LuaJIT fixtures")
    })
}

fn num(s: &str) -> f64 {
    let t = s.trim();
    if t.contains("nan") {
        f64::NAN
    } else if t == "inf" {
        f64::INFINITY
    } else if t == "-inf" {
        f64::NEG_INFINITY
    } else {
        t.parse::<f64>()
            .unwrap_or_else(|_| panic!("bad number {t:?}"))
    }
}

fn nums(s: &str) -> Vec<f64> {
    s.split(',').map(num).collect()
}

fn same(a: f64, b: f64) -> bool {
    (a.is_nan() && b.is_nan()) || a.to_bits() == b.to_bits()
}

#[test]
fn matches_luajit_value_for_value() {
    let text = fixture();
    let mut counts = std::collections::BTreeMap::<&str, usize>::new();
    for (lineno, line) in text.lines().enumerate() {
        let f: Vec<&str> = line.split('\t').collect();
        let kind = f[0];
        *counts.entry(kind).or_default() += 1;
        let at = || format!("line {}: {line}", lineno + 1);
        match kind {
            "RAND" => {
                let mut rs = LuaRandom::new(num(f[1]));
                for want in nums(f[2]) {
                    let got = rs.random();
                    assert!(same(got, want), "{} got {got:?}", at());
                }
            }
            "RANDN" => {
                let mut rs = LuaRandom::new(num(f[1]));
                let m = num(f[2]);
                for want in nums(f[3]) {
                    let got = rs.random_upto(m);
                    assert!(same(got, want), "{} got {got:?}", at());
                }
            }
            "RANDAB" => {
                let mut rs = LuaRandom::new(num(f[1]));
                let (a, b) = (num(f[2]), num(f[3]));
                for want in nums(f[4]) {
                    let got = rs.random_range(a, b);
                    assert!(same(got, want), "{} got {got:?}", at());
                }
            }
            "HASH" => {
                let got = pseudohash(f[1]);
                assert!(same(got, num(f[2])), "{} got {got:?}", at());
            }
            "R13" => {
                let got = round13(num(f[1]));
                assert!(same(got, num(f[3])), "{} got {got:?}", at());
            }
            "SEQ" => {
                let mut st = Streams::new(f[1]);
                let want = nums(f[2]);
                let mut i = 0;
                for _ in 0..3 {
                    for k in KEYS {
                        let got = st.pseudoseed(k);
                        assert!(
                            same(got, want[i]),
                            "{} value {i} key {k:?} got {got:?}",
                            at()
                        );
                        i += 1;
                    }
                }
                assert_eq!(i, want.len());
            }
            "PR" => {
                let mut st = Streams::new(f[1]);
                let want = nums(f[2]);
                let mut i = 0;
                for k in KEYS {
                    let got = st.random(k);
                    assert!(same(got, want[i]), "{} key {k:?} got {got:?}", at());
                    let got = st.random_range(k, 1, 100) as f64;
                    assert!(same(got, want[i + 1]), "{} key {k:?} got {got:?}", at());
                    i += 2;
                }
            }
            "SHUF" => {
                // Streams are independent, so each key's first draw can be
                // replayed on fresh streams.
                let mut st = Streams::new(f[1]);
                let mut rng = st.generator(f[2]);
                let mut list: Vec<u32> = (1..=52).collect();
                shuffle(&mut list, &mut rng);
                let want: Vec<u32> = f[3].split(',').map(|x| x.parse().unwrap()).collect();
                assert_eq!(list, want, "{}", at());
            }
            other => panic!("unknown fixture kind {other}"),
        }
    }
    // Make sure the fixture actually exercised every kind.
    for k in [
        "RAND", "RANDN", "RANDAB", "HASH", "R13", "SEQ", "PR", "SHUF",
    ] {
        assert!(
            counts.get(k).copied().unwrap_or(0) > 0,
            "no {k} lines in the fixture"
        );
    }
}
