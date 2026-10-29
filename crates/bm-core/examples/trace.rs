//! Print a run played by a simple greedy bot, move by move.
use bm_core::blinds::BlindSlot;
use bm_core::{Phase, Run, RunConfig};

fn main() {
    let seed = std::env::args().nth(1).unwrap_or_else(|| "ABCDEFGH".into());
    let mut run = Run::new(RunConfig::new(&seed));
    println!(
        "seed {seed}: boss {:?}, tags {:?}, orbital {:?}",
        run.boss, run.tags_offered, run.orbital
    );
    for _ in 0..400 {
        match run.phase {
            Phase::GameOver => {
                println!("GAME OVER at ante {}", run.ante);
                break;
            }
            Phase::BlindSelect => {
                let s = run.on_deck();
                run.select_blind().unwrap();
                let b = run.blind.as_ref().unwrap();
                println!(
                    "-- ante {} {:?} ({:?}), target {}",
                    run.ante, s, b.kind, b.chips
                );
                let _ = BlindSlot::Small;
            }
            Phase::Round => {
                let hand: Vec<String> = run.hand_cards().iter().map(|c| c.to_string()).collect();
                // greedy: try every subset of up to 5 cards, play the best score
                let n = run.hand.len();
                let mut best = (f64::MIN, vec![0usize]);
                for mask in 1u32..(1 << n) {
                    if mask.count_ones() > 5 {
                        continue;
                    }
                    let pos: Vec<usize> = (0..n).filter(|i| mask & (1 << i) != 0).collect();
                    let mut sim = run.clone();
                    let before = sim.round_chips;
                    if let Ok(s) = sim.play(&pos) {
                        let gain = s.score;
                        let _ = before;
                        if gain > best.0 {
                            best = (gain, pos);
                        }
                    }
                }
                let s = run.play(&best.1).unwrap();
                println!(
                    "   hand [{}] plays {:?} -> {:?} {} x {} = {} (total {}, money {})",
                    hand.join(" "),
                    best.1,
                    s.hand,
                    s.chips,
                    s.mult,
                    s.score,
                    run.round_chips,
                    run.money
                );
            }
            Phase::RoundEval => {
                let c = run.cash_out.clone().unwrap();
                let t = run.cash_out().unwrap();
                println!("   cash out {t} ({c:?}) -> money {}", run.money);
            }
            Phase::Shop => run.leave_shop().unwrap(),
            Phase::Pack => run.pack_skip().unwrap(),
        }
    }
}
