//! How far the greedy bot gets: the ante each run ends at.
use bm_core::bots::greedy_move;
use bm_core::env::apply;
use bm_core::{Phase, Run, RunConfig};

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(200);
    let mut seeds = bm_core::bm_rng::SeedSampler::new(77);
    let mut ends = std::collections::BTreeMap::new();
    let t = std::time::Instant::now();
    let mut moves = 0u64;
    for i in 0..n {
        let mut run = Run::new(RunConfig::new(&seeds.next_seed()));
        let mut step = 0u64;
        while run.phase != Phase::GameOver && !run.won && step < 2000 {
            let (k, a) = greedy_move(&run, (i as u64) << 20 | step);
            apply(&mut run, k, a).unwrap();
            step += 1;
        }
        moves += step;
        *ends.entry(if run.won { 9 } else { run.ante }).or_insert(0) += 1;
    }
    println!(
        "runs end at ante (9 = won): {ends:?}; {moves} moves in {:.1}s",
        t.elapsed().as_secs_f64()
    );
}
