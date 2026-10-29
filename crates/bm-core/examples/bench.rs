//! Speed benchmark: random legal moves on every thread for a few seconds,
//! with and without building the bot's input after each move.
//!
//!     cargo run --release -p bm-core --example bench -- [seconds] [threads, 0 = all] [out.json]

use std::time::{Duration, Instant};

use bm_core::env::{random_move, Env, XorShift};

fn run(threads: usize, secs: f64, observe: bool) -> (u64, u64, f64) {
    let stop = Duration::from_secs_f64(secs);
    let results: Vec<(u64, u64)> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                s.spawn(move || {
                    let mut env = Env::new(1000 + t as u64, 1);
                    let mut rng = XorShift(0x9E37_79B9_7F4A_7C15 ^ (t as u64 + 1));
                    let mut obs = Vec::new();
                    let (mut moves, start) = (0u64, Instant::now());
                    while start.elapsed() < stop {
                        for _ in 0..256 {
                            let (k, a) = random_move(&env.run, &mut rng);
                            env.step(k as i64, a);
                            if observe {
                                env.observe(&mut obs);
                            }
                            moves += 1;
                        }
                    }
                    (moves, env.episodes)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let moves: u64 = results.iter().map(|r| r.0).sum();
    let eps: u64 = results.iter().map(|r| r.1).sum();
    (moves, eps, secs)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let secs: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5.0);
    let threads: usize = match args.get(2).and_then(|s| s.parse().ok()) {
        Some(t) if t > 0 => t,
        _ => std::thread::available_parallelism().map_or(1, |n| n.get()),
    };
    let out = args.get(3);
    let (m1, e1, _) = run(threads, secs, false);
    let (m2, e2, _) = run(threads, secs, true);
    let (single, _, _) = run(1, secs.min(3.0), false);
    let rate = |m: u64, s: f64| m as f64 / s;
    let report = format!(
        "{{\n \"threads\": {threads},\n \"seconds\": {secs},\n \"moves_per_s\": {:.0},\n \"moves_per_s_with_obs\": {:.0},\n \"runs_per_s\": {:.0},\n \"runs_per_s_with_obs\": {:.0},\n \"moves_per_s_one_thread\": {:.0},\n \"target_moves_per_s\": 200000,\n \"passes\": {}\n}}",
        rate(m1, secs),
        rate(m2, secs),
        rate(e1, secs),
        rate(e2, secs),
        rate(single, secs.min(3.0)),
        rate(m1, secs) >= 200_000.0
    );
    println!("{report}");
    if let Some(p) = out {
        if let Some(dir) = std::path::Path::new(p).parent() {
            std::fs::create_dir_all(dir).expect("make the report's folder");
        }
        std::fs::write(p, report + "\n").expect("write report");
    }
}
