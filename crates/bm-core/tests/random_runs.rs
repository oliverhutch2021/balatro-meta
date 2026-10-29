//! Random play through whole runs: no panics, cards are conserved, and the
//! same seed and moves always give the same run.

use bm_core::blinds::SlotState;
use bm_core::bm_rng::SeedSampler;
use bm_core::joker_data::JokerKind;
use bm_core::{Phase, Run, RunConfig};

/// A tiny deterministic generator for the test's own choices.
struct Pick(u64);
impl Pick {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }
}

fn check_conservation(run: &Run) {
    let mut all: Vec<u32> = run
        .deck
        .iter()
        .chain(&run.hand)
        .chain(&run.discard)
        .chain(&run.play)
        .copied()
        .collect();
    all.sort_unstable();
    let mut pc = run.playing_cards.clone();
    pc.sort_unstable();
    assert_eq!(all, pc, "cards lost or duplicated between areas");
}

/// Play a run with random legal moves; returns a fingerprint of the run.
fn random_run(
    seed: &str,
    choices: u64,
    jokers: &[JokerKind],
) -> (Phase, i32, i64, f64, Vec<String>) {
    let mut run = Run::new(RunConfig::new(seed));
    for &j in jokers {
        run.add_joker(j);
    }
    let mut pick = Pick(choices | 1);
    let mut log = Vec::new();
    for _ in 0..2000 {
        check_conservation(&run);
        match run.phase {
            Phase::GameOver => break,
            Phase::BlindSelect => {
                if run.on_deck() != bm_core::blinds::BlindSlot::Boss && pick.below(4) == 0 {
                    run.skip_blind().unwrap();
                    log.push(format!("skip {:?}", run.tags.last().map(|t| t.kind)));
                } else {
                    run.select_blind().unwrap();
                    log.push(format!(
                        "select a{} {:?}",
                        run.ante,
                        run.blind.as_ref().map(|b| b.kind)
                    ));
                }
            }
            Phase::Round => {
                let n = run.hand.len();
                let k = 1 + pick.below(n.min(5));
                let mut pos: Vec<usize> = (0..n).collect();
                for i in 0..k {
                    let j = i + pick.below(n - i);
                    pos.swap(i, j);
                }
                pos.truncate(k);
                if let Some(f) = run.forced_card() {
                    if !pos.contains(&f) {
                        pos[0] = f;
                    }
                }
                if run.discards_left > 0 && pick.below(3) == 0 {
                    run.discard(&pos).unwrap();
                    log.push(format!("discard {k}"));
                } else {
                    let s = run.play(&pos).unwrap();
                    log.push(format!("play {:?} {}", s.hand, s.score));
                }
            }
            Phase::RoundEval => {
                let t = run.cash_out().unwrap();
                log.push(format!("cash out {t}"));
            }
            Phase::Shop => run.leave_shop().unwrap(),
            Phase::Pack => run.pack_skip().unwrap(),
        }
        if run.won {
            break;
        }
    }
    (run.phase, run.ante, run.money, run.round_chips, log)
}

#[test]
fn random_runs_finish_and_conserve_cards() {
    let mut seeds = SeedSampler::new(7);
    let mut lost = 0;
    for i in 0..300 {
        let seed = seeds.next_seed();
        let (phase, ante, _, _, log) = random_run(&seed, i + 1, &[]);
        assert!(!log.is_empty());
        if phase == Phase::GameOver {
            lost += 1;
        }
        assert!(ante >= 1);
    }
    // Random play loses almost every run.
    assert!(lost > 250, "random play lost only {lost} of 300 runs");
}

#[test]
fn random_runs_with_jokers() {
    use JokerKind as J;
    let sets: [&[JokerKind]; 6] = [
        &[
            J::Blueprint,
            J::Joker,
            J::Brainstorm,
            J::BaseballCard,
            J::Mime,
        ],
        &[
            J::HangingChad,
            J::SockAndBuskin,
            J::Photograph,
            J::ScaryFace,
            J::Bloodstone,
        ],
        &[
            J::Vampire,
            J::MidasMask,
            J::SpaceJoker,
            J::Misprint,
            J::Hiker,
        ],
        &[
            J::RaisedFist,
            J::Baron,
            J::ShootTheMoon,
            J::ReservedParking,
            J::Dusk,
        ],
        &[
            J::Juggler,
            J::Drunkard,
            J::Troubadour,
            J::MerryAndy,
            J::Stuntman,
        ],
        &[
            J::IceCream,
            J::Seltzer,
            J::Popcorn,
            J::GrosMichel,
            J::MrBones,
        ],
    ];
    let mut seeds = SeedSampler::new(11);
    for (i, set) in sets.iter().enumerate() {
        for k in 0..40 {
            let seed = seeds.next_seed();
            random_run(&seed, (i * 100 + k + 1) as u64, set);
        }
    }
}

#[test]
fn same_seed_same_moves_same_run() {
    let mut seeds = SeedSampler::new(3);
    for i in 0..50 {
        let seed = seeds.next_seed();
        let a = random_run(&seed, i + 9, &[JokerKind::Misprint, JokerKind::Bloodstone]);
        let b = random_run(&seed, i + 9, &[JokerKind::Misprint, JokerKind::Bloodstone]);
        assert_eq!(a.4, b.4);
        assert_eq!((a.0, a.1, a.2), (b.0, b.1, b.2));
    }
}

#[test]
fn run_start_shape() {
    let run = Run::new(RunConfig::new("ABCDEFGH"));
    assert_eq!(run.phase, Phase::BlindSelect);
    assert_eq!(run.deck.len(), 52);
    assert_eq!(run.money, 4);
    assert_eq!(
        (run.hands_left, run.discards_left, run.hand_size()),
        (4, 4, 8)
    );
    assert_eq!(run.slots[0], SlotState::Select);
    assert_eq!(run.boss.min_ante(), 1);
    // Three Orbital hands were fixed when blind select was built.
    assert_eq!(run.orbital.len(), 3);
}

/// Random play with every kind of move (shop, packs, consumables, vouchers),
/// through the environment's move generator: no panics, cards conserved,
/// refused moves change nothing.
#[test]
fn random_runs_with_the_whole_game() {
    use bm_core::env::{apply, random_move, safe_move, XorShift};
    let mut seeds = SeedSampler::new(99);
    let mut rng = XorShift(0x1234_5678_9abc_def1);
    let (mut shops, mut packs, mut used, mut bought) = (0, 0, 0, 0);
    for _ in 0..120 {
        let mut run = Run::new(RunConfig::new(&seeds.next_seed()));
        for _ in 0..3000 {
            if run.phase == Phase::GameOver || run.won {
                break;
            }
            check_conservation(&run);
            // The greedy bot plays the rounds (so runs reach shops); everything
            // else is random.
            let (k, a) = if run.phase == Phase::Round && rng.below(4) != 0 {
                bm_core::bots::greedy_move(&run, rng.below(1 << 30) as u64)
            } else {
                random_move(&run, &mut rng)
            };
            let before_money = run.money;
            let before_phase = run.phase;
            match apply(&mut run, k, a) {
                Ok(()) => {
                    match k {
                        bm_core::env::MoveKind::OpenBooster => packs += 1,
                        bm_core::env::MoveKind::UseConsumable => used += 1,
                        bm_core::env::MoveKind::Buy => bought += 1,
                        _ => {}
                    }
                    if before_phase != Phase::Shop && run.phase == Phase::Shop {
                        shops += 1;
                    }
                }
                Err(_) => {
                    assert_eq!(run.money, before_money, "a refused {k:?} changed money");
                    assert_eq!(run.phase, before_phase);
                    let (k2, a2) = safe_move(&run);
                    apply(&mut run, k2, a2).unwrap();
                }
            }
        }
    }
    println!("shops {shops}, packs {packs}, bought {bought}, used {used}");
    assert!(
        shops > 100 && packs > 20 && bought > 20 && used > 0,
        "shops {shops}, packs {packs}, bought {bought}, used {used}"
    );
}
