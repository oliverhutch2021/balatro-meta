//! The fair-play wall: the bot's input depends only on what the screen
//! shows.

use bm_core::bm_rng::{SeedSampler, Streams};
use bm_core::joker_data::JokerKind;
use bm_core::observe::{encode, fair_copy, screen, OBS_LEN};
use bm_core::{Phase, Run, RunConfig};

struct Pick(u64);
impl Pick {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }
}

/// A run that shows the same screen but has a different seed, different
/// random streams, a different draw order and different creation ids.
fn twin(run: &Run, salt: u64) -> Run {
    let mut t = run.clone();
    let mut s = SeedSampler::new(salt);
    t.cfg.seed = s.next_seed();
    t.rng = Streams::new(&t.cfg.seed);
    // Use the twin's own streams so they are not the original's.
    for k in [
        "boss",
        "Tag1",
        "shuffle",
        "nr1",
        "hook",
        "wheel",
        "glass",
        "lucky_mult",
    ] {
        t.rng.random(k);
    }
    let n = t.deck.len();
    for i in (1..n).rev() {
        let j = s.below((i + 1) as u64) as usize;
        t.deck.swap(i, j);
    }
    let m = t.cards.len();
    for i in (1..m).rev() {
        let j = s.below((i + 1) as u64) as usize;
        let a = t.cards[i].sort_id;
        t.cards[i].sort_id = t.cards[j].sort_id;
        t.cards[j].sort_id = a;
    }
    t
}

fn random_move(run: &mut Run, pick: &mut Pick) {
    match run.phase {
        Phase::BlindSelect => {
            if run.on_deck() != bm_core::blinds::BlindSlot::Boss && pick.below(3) == 0 {
                run.skip_blind().unwrap();
            } else {
                run.select_blind().unwrap();
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
            if pick.below(5) == 0 && n > 1 {
                let (a, b) = (pick.below(n), pick.below(n));
                run.move_hand_card(a, b).unwrap();
            } else if run.discards_left > 0 && pick.below(3) == 0 {
                run.discard(&pos).unwrap();
            } else {
                run.play(&pos).unwrap();
            }
        }
        Phase::RoundEval => {
            run.cash_out().unwrap();
        }
        Phase::Shop => run.leave_shop().unwrap(),
        Phase::Pack => run.pack_skip().unwrap(),
        Phase::GameOver => {}
    }
}

#[test]
fn same_screen_different_seed_gives_identical_input() {
    let mut seeds = SeedSampler::new(42);
    let mut checked = 0;
    let (mut a, mut b) = (Vec::new(), Vec::new());
    for r in 0..200u64 {
        let mut run = Run::new(RunConfig::new(&seeds.next_seed()));
        if r % 2 == 0 {
            for k in [
                JokerKind::Joker,
                JokerKind::TheIdol,
                JokerKind::AncientJoker,
                JokerKind::Misprint,
            ] {
                run.add_joker(k);
            }
        }
        let mut pick = Pick(r * 7919 + 1);
        for step in 0..60 {
            if run.phase == Phase::GameOver {
                break;
            }
            let t = twin(&run, r * 1000 + step);
            assert_eq!(
                screen(&run),
                screen(&t),
                "screens differ: run {r} step {step}"
            );
            encode(&screen(&run), &mut a);
            encode(&screen(&t), &mut b);
            assert_eq!(a.len(), OBS_LEN);
            assert!(
                a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()),
                "inputs differ: run {r} step {step}"
            );
            let f = fair_copy(&run, step);
            assert_eq!(
                screen(&run),
                screen(&f),
                "fair copy shows a different screen"
            );
            checked += 1;
            random_move(&mut run, &mut pick);
        }
    }
    assert!(checked > 2000, "only {checked} positions checked");
}

#[test]
fn fair_copies_have_new_futures() {
    // From the same position, fair copies draw different cards next.
    let mut run = Run::new(RunConfig::new("FAIRCOPY"));
    run.select_blind().unwrap();
    let real_next: Vec<u32> = run.deck.iter().rev().take(5).copied().collect();
    let mut differs = 0;
    for salt in 0..20 {
        let c = fair_copy(&run, salt);
        let next: Vec<u32> = c.deck.iter().rev().take(5).copied().collect();
        if next != real_next {
            differs += 1;
        }
        assert_ne!(c.rng.seed(), run.rng.seed());
    }
    assert!(differs >= 19);
}

#[test]
fn the_input_changes_when_the_screen_does() {
    let mut run = Run::new(RunConfig::new("CHANGES1"));
    let (mut a, mut b) = (Vec::new(), Vec::new());
    encode(&screen(&run), &mut a);
    run.select_blind().unwrap();
    encode(&screen(&run), &mut b);
    assert_ne!(a, b);
    a = b.clone();
    run.play(&[0]).unwrap();
    encode(&screen(&run), &mut b);
    assert_ne!(a, b);
}

/// The same check through shops and packs: buying, rerolling, opening and
/// choosing, with consumables used, all played by the env's move generator
/// (greedy rounds so runs reach later shops).
#[test]
fn shops_and_packs_show_nothing_hidden() {
    use bm_core::env::{apply, random_move as env_move, safe_move, XorShift};
    let mut seeds = SeedSampler::new(4321);
    let mut rng = XorShift(0x7777_1234_5678_9999);
    let (mut a, mut b) = (Vec::new(), Vec::new());
    let (mut shop_steps, mut pack_steps) = (0, 0);
    for r in 0..250u64 {
        let mut run = Run::new(RunConfig::new(&seeds.next_seed()));
        for step in 0..600u64 {
            if run.phase == Phase::GameOver || run.won {
                break;
            }
            match run.phase {
                Phase::Shop => shop_steps += 1,
                Phase::Pack => pack_steps += 1,
                _ => {}
            }
            let t = twin(&run, r * 100_000 + step);
            assert_eq!(
                screen(&run),
                screen(&t),
                "screens differ: run {r} step {step}"
            );
            encode(&screen(&run), &mut a);
            encode(&screen(&t), &mut b);
            assert!(
                a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()),
                "inputs differ: run {r} step {step} ({:?})",
                run.phase
            );
            let f = fair_copy(&run, step);
            assert_eq!(
                screen(&run),
                screen(&f),
                "fair copy shows a different screen"
            );
            let (k, arg) = if run.phase == Phase::Round && rng.below(3) != 0 {
                bm_core::bots::greedy_move(&run, rng.below(1 << 30) as u64)
            } else {
                env_move(&run, &mut rng)
            };
            if apply(&mut run, k, arg).is_err() {
                let (k2, a2) = safe_move(&run);
                apply(&mut run, k2, a2).unwrap();
            }
        }
    }
    assert!(
        shop_steps > 300 && pack_steps > 30,
        "shop {shop_steps}, pack {pack_steps}"
    );
}

#[test]
fn fair_copies_do_not_know_the_next_shop() {
    // From the cash-out screen, the real run's next shop and a fair copy's
    // next shop differ: the copy's streams are new.
    let mut differs = 0;
    for i in 0..10 {
        let mut run = Run::new(RunConfig::new(&format!("NEXTSHP{i}")));
        run.select_blind().unwrap();
        run.blind.as_mut().unwrap().chips = 1.0;
        run.play(&[0]).unwrap();
        assert_eq!(run.phase, Phase::RoundEval);
        let mut copy = fair_copy(&run, i);
        run.cash_out().unwrap();
        copy.cash_out().unwrap();
        if format!("{:?}", run.shop.as_ref().map(|s| &s.row))
            != format!("{:?}", copy.shop.as_ref().map(|s| &s.row))
        {
            differs += 1;
        }
    }
    assert!(
        differs >= 9,
        "only {differs} of 10 copies saw a different shop"
    );
}
