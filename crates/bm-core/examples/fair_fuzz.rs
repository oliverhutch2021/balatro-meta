//! The fair-play fuzz, at length: the teacher must make the same move on a run and on fair copies of it.
//!
//!     cargo run --release -p bm-core --example fair_fuzz -- [--runs N] [--every K] [--shakes M]
//!         [--teacher-options a,b] [--value-net a.bin,b.bin] [--value-weight W] [--threads T] [--seed S]
//!
//! A fair copy (`observe::fair_copy`) shows the same screen and draws everything hidden afresh: the draw order,
//! every random stream and the seed, which cards lie face down, where face-down jokers lie, the ante's voucher
//! before a shop shows it. So a teacher that decides from the screen and from fair copies alone moves the same on
//! both, and one that reads anything hidden, anywhere, sooner or later doesn't. The teacher plays `--runs` runs;
//! at every `--every`-th decision the position is pushed `--shakes` times toward rare situations (`Run::shake`:
//! jokers, consumables, vouchers, modified cards, any boss, face-down cards and jokers) and the teacher's move on
//! each is set against its move on two fair copies, its memory cleared before each. Every difference is printed
//! with the position; the exit code is 1 if there is one. `tests/fair_play.rs` runs a small slice of this.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Mutex;

use bm_core::env::apply;
use bm_core::observe::{fair_copy, screen};
use bm_core::teacher::{new_run, teacher_move};
use bm_core::{learned, Phase, Run, RunConfig};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let opt = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let num = |name: &str, d: u64| opt(name).and_then(|s| s.parse().ok()).unwrap_or(d);
    let (runs, every, shakes, base) = (
        num("--runs", 16),
        num("--every", 3),
        num("--shakes", 2),
        num("--seed", 1),
    );
    if let Some(o) = opt("--teacher-options") {
        let names: Vec<String> = o
            .split(',')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
        bm_core::teacher::set_options(&names).unwrap_or_else(|e| panic!("{e}"));
    }
    if let Some(p) = opt("--value-net") {
        learned::load(&p).unwrap_or_else(|e| panic!("{e}"));
    }
    if let Some(w) = opt("--value-weight") {
        learned::set_weight(w.parse().expect("--value-weight takes a number"));
    }
    let threads = opt("--threads")
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()));
    std::panic::set_hook(Box::new(|_| {}));
    let found = Mutex::new((0u64, Vec::<String>::new(), 0u64));
    std::thread::scope(|s| {
        for t in 0..threads as u64 {
            let found = &found;
            s.spawn(move || {
                for r in (0..runs).filter(|r| r % threads as u64 == t) {
                    let (n, diffs, panics) = check_run(base, r, every, shakes);
                    let mut f = found.lock().unwrap();
                    f.0 += n;
                    f.1.extend(diffs);
                    f.2 += panics;
                }
            });
        }
    });
    let (n, diffs, panics) = found.into_inner().unwrap();
    for d in &diffs {
        println!("{d}");
    }
    println!(
        "{n} positions checked, {} moved differently on a fair copy, {panics} panicked",
        diffs.len()
    );
    if !diffs.is_empty() || panics > 0 {
        std::process::exit(1);
    }
}

/// One teacher-played run, shaken and checked at every `every`-th decision: (positions, differences, panics).
fn check_run(base: u64, r: u64, every: u64, shakes: u64) -> (u64, Vec<String>, u64) {
    let seed = bm_core::env::run_seed(base, r);
    let mut run = Run::new(RunConfig::new(&seed));
    let (mut n, mut diffs, mut panics) = (0, Vec::new(), 0);
    new_run();
    let mut step = 0u64;
    while run.phase != Phase::GameOver && !run.won && step < 3000 {
        let salt = r << 20 | step;
        if step.is_multiple_of(every) {
            for m in 0..shakes {
                let mut pos = run.clone();
                let what = pos.shake(salt ^ m.wrapping_mul(0xA24B_AED4_963E_E407));
                n += 1;
                match catch_unwind(AssertUnwindSafe(|| differences(&pos, salt, step + m))) {
                    Ok(d) if d.is_empty() => {}
                    Ok(d) => diffs.push(format!(
                        "{seed} move {step} ante {} {:?} [{what}]: {}",
                        pos.ante,
                        pos.phase,
                        d.join("; ")
                    )),
                    Err(_) => panics += 1,
                }
            }
            new_run();
        }
        let (k, a) = teacher_move(&run, salt);
        if apply(&mut run, k, a).is_err() {
            break;
        }
        step += 1;
    }
    (n, diffs, panics)
}

/// The teacher's move on `pos` against its moves on two fair copies (its memory cleared before each).
fn differences(pos: &Run, salt: u64, k: u64) -> Vec<String> {
    new_run();
    let real = teacher_move(pos, salt);
    let mut out = Vec::new();
    for c in 0..2u64 {
        let copy = fair_copy(pos, 0xBEEF ^ k.wrapping_mul(31) ^ c);
        if screen(&copy) != screen(pos) {
            out.push("the fair copy shows another screen".into());
        }
        new_run();
        let moved = teacher_move(&copy, salt);
        if moved != real {
            out.push(format!("real {real:?}, copy {moved:?}"));
        }
    }
    out
}
