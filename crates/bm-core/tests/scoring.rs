//! Scoring checks. Every expected number is worked out by hand from
//! `rules/scoring.md` (or by an independent formula), not by the engine.

use bm_core::blinds::{BlindKind, Boss};
use bm_core::bm_rng::Streams;
use bm_core::card::{standard_card_order, Edition, Enhancement, Rank, Seal, Suit};
use bm_core::hand_levels::base_values;
use bm_core::joker_data::JokerKind as J;
use bm_core::poker::{detect, DetectRules};
use bm_core::scoring::evaluate_play;
use bm_core::Run;

use Rank::*;
use Suit::*;

const SEED: &str = "TESTSEED";

fn small() -> Run {
    Run::scenario(SEED, BlindKind::Small)
}

/// Play the first `n` hand cards (in hand order) and return the hand score.
fn play_first(run: &mut Run, n: usize) -> f64 {
    let pos: Vec<usize> = (0..n).collect();
    run.play(&pos).unwrap().score
}

fn with_jokers(jokers: &[J]) -> Run {
    let mut run = small();
    for &j in jokers {
        run.add_joker(j);
    }
    run
}

fn pair_of_kings(run: &mut Run) {
    run.add_card_to_hand(Spades, King);
    run.add_card_to_hand(Hearts, King);
}

#[test]
fn plain_pair_and_joker() {
    let mut run = with_jokers(&[J::Joker]);
    pair_of_kings(&mut run);
    run.add_card_to_hand(Clubs, Two);
    // Pair: (10 + 10 + 10) x (2 + 4)
    assert_eq!(play_first(&mut run, 2), 180.0);
}

#[test]
fn blueprint_copies_its_right_neighbour() {
    let mut run = with_jokers(&[J::Blueprint, J::Joker]);
    pair_of_kings(&mut run);
    assert_eq!(play_first(&mut run, 2), 30.0 * 10.0);
    let mut run = with_jokers(&[J::Joker, J::Blueprint]);
    pair_of_kings(&mut run);
    assert_eq!(play_first(&mut run, 2), 30.0 * 6.0);
}

#[test]
fn brainstorm_copies_the_leftmost_joker() {
    let mut run = with_jokers(&[J::Joker, J::Brainstorm]);
    pair_of_kings(&mut run);
    assert_eq!(play_first(&mut run, 2), 30.0 * 10.0);
    // Leftmost itself: copies nothing.
    let mut run = with_jokers(&[J::Brainstorm, J::Joker]);
    pair_of_kings(&mut run);
    assert_eq!(play_first(&mut run, 2), 30.0 * 6.0);
    // A copy loop gives nothing and ends.
    let mut run = with_jokers(&[J::Blueprint, J::Brainstorm]);
    pair_of_kings(&mut run);
    assert_eq!(play_first(&mut run, 2), 30.0 * 2.0);
}

#[test]
fn hanging_chad_retriggers_the_first_card_twice() {
    let mut run = with_jokers(&[J::HangingChad]);
    pair_of_kings(&mut run);
    // 10 + 10*3 + 10 = 50, x2
    assert_eq!(play_first(&mut run, 2), 100.0);
}

#[test]
fn joker_order_decides_add_then_multiply() {
    let mut run = with_jokers(&[J::Joker, J::TheDuo]);
    pair_of_kings(&mut run);
    assert_eq!(play_first(&mut run, 2), 30.0 * ((2.0 + 4.0) * 2.0));
    let mut run = with_jokers(&[J::TheDuo, J::Joker]);
    pair_of_kings(&mut run);
    assert_eq!(play_first(&mut run, 2), 30.0 * (2.0 * 2.0 + 4.0));
}

#[test]
fn baron_with_mime_on_held_kings() {
    let mut run = with_jokers(&[J::Baron, J::Mime]);
    run.add_card_to_hand(Clubs, Two);
    run.add_card_to_hand(Diamonds, King);
    run.add_card_to_hand(Clubs, King);
    // High Card 5 + 2 = 7 chips; each held King X1.5 twice: 1.5^4
    assert_eq!(play_first(&mut run, 1), (7.0f64 * 1.5f64.powi(4)).floor());
}

#[test]
fn raised_fist_uses_the_rightmost_lowest_card() {
    let mut run = with_jokers(&[J::RaisedFist]);
    run.add_card_to_hand(Spades, Ace);
    run.add_card_to_hand(Spades, Five);
    run.add_card_to_hand(Diamonds, Three);
    run.add_card_to_hand(Clubs, Three);
    run.add_card_to_hand(Hearts, Nine);
    // A alone: 5 + 11 = 16 chips; +2 x 3 on one held Three
    assert_eq!(play_first(&mut run, 1), 16.0 * 7.0);
}

#[test]
fn steel_card_with_red_seal_in_hand() {
    let mut run = small();
    run.add_card_to_hand(Spades, Ace);
    run.add_special_card_to_hand(Hearts, King, Enhancement::Steel, Edition::None, Seal::Red);
    assert_eq!(play_first(&mut run, 1), (16.0f64 * 1.5 * 1.5).floor());
    // A plain held card with a Red seal does nothing, so it isn't retriggered.
    let mut run = small();
    run.add_card_to_hand(Spades, Ace);
    run.add_special_card_to_hand(Hearts, Five, Enhancement::None, Edition::None, Seal::Red);
    assert_eq!(play_first(&mut run, 1), 16.0);
}

#[test]
fn joker_editions_wrap_the_joker() {
    for (ed, expect) in [
        (Edition::Polychrome, 30.0 * ((2.0 + 4.0) * 1.5)),
        (Edition::Foil, (30.0 + 50.0) * 6.0),
        (Edition::Holographic, 30.0 * (2.0 + 10.0 + 4.0)),
    ] {
        let mut run = small();
        run.add_joker_with_edition(J::Joker, ed);
        pair_of_kings(&mut run);
        assert_eq!(play_first(&mut run, 2), expect, "{ed:?}");
    }
}

#[test]
fn baseball_card_multiplies_for_each_uncommon() {
    // Hack is Uncommon and does nothing for Kings; Joker is Common.
    let mut run = with_jokers(&[J::Joker, J::Hack, J::BaseballCard]);
    pair_of_kings(&mut run);
    assert_eq!(play_first(&mut run, 2), 30.0 * ((2.0 + 4.0) * 1.5));
}

#[test]
fn the_flint_halves_the_base_rounding_up() {
    let mut run = Run::scenario(SEED, BlindKind::Boss(Boss::Flint));
    pair_of_kings(&mut run);
    // Pair 10 chips x 2 mult -> 5 x 1, then +20 chips
    assert_eq!(play_first(&mut run, 2), 25.0);
}

#[test]
fn the_club_debuffs_clubs_and_matador_pays() {
    let mut run = Run::scenario(SEED, BlindKind::Boss(Boss::Club));
    run.add_joker(J::Matador);
    run.add_card_to_hand(Clubs, King);
    run.add_card_to_hand(Hearts, King);
    let money = run.money;
    // Still a Pair, but the King of Clubs adds nothing.
    assert_eq!(play_first(&mut run, 2), 40.0);
    assert_eq!(run.money, money + 8);
}

#[test]
fn the_psychic_zeroes_short_hands() {
    let mut run = Run::scenario(SEED, BlindKind::Boss(Boss::Psychic));
    run.add_joker(J::Matador);
    pair_of_kings(&mut run);
    let money = run.money;
    let s = run.play(&[0, 1]).unwrap();
    assert!(s.not_allowed);
    assert_eq!(s.score, 0.0);
    assert_eq!(run.money, money + 8);
}

#[test]
fn splash_and_stone_cards_score() {
    let mut run = with_jokers(&[J::Splash]);
    pair_of_kings(&mut run);
    run.add_card_to_hand(Clubs, Two);
    assert_eq!(play_first(&mut run, 3), (30.0 + 2.0) * 2.0);
    let mut run = small();
    pair_of_kings(&mut run);
    run.add_special_card_to_hand(
        Diamonds,
        Five,
        Enhancement::Stone,
        Edition::None,
        Seal::None,
    );
    assert_eq!(play_first(&mut run, 3), (30.0 + 50.0) * 2.0);
}

#[test]
fn hiker_grows_between_retriggers() {
    let mut run = with_jokers(&[J::HangingChad, J::Hiker]);
    run.add_card_to_hand(Spades, King);
    // High Card: 5 + 10 + 15 + 20
    assert_eq!(play_first(&mut run, 1), 50.0);
}

#[test]
fn card_rolls_follow_the_streams_in_order() {
    // Two Lucky Glass Kings: per card lucky_mult then lucky_money, then the
    // glass rolls after the joker loop.
    let mut run = small();
    for s in [Spades, Hearts] {
        run.add_special_card_to_hand(s, King, Enhancement::Lucky, Edition::None, Seal::None);
    }
    let mut rng = Streams::new(SEED);
    let mut mult = 2.0;
    for _ in 0..2 {
        if rng.random("lucky_mult") < 1.0 / 5.0 {
            mult += 20.0;
        }
        let _ = rng.random("lucky_money");
    }
    assert_eq!(play_first(&mut run, 2), 30.0 * mult);

    let mut run = small();
    let a =
        run.add_special_card_to_hand(Spades, King, Enhancement::Glass, Edition::None, Seal::None);
    let b =
        run.add_special_card_to_hand(Hearts, King, Enhancement::Glass, Edition::None, Seal::None);
    let s = run.play(&[0, 1]).unwrap();
    assert_eq!(s.score, 30.0 * 8.0);
    let mut rng = Streams::new(SEED);
    let expect: Vec<u32> = [a, b]
        .into_iter()
        .filter(|_| rng.random("glass") < 0.25)
        .collect();
    assert_eq!(s.destroyed, expect);
    for id in &expect {
        assert!(!run.playing_cards.contains(id));
    }
}

#[test]
fn per_card_jokers() {
    type Case = (J, [(Suit, Rank); 2], f64);
    let cases: [Case; 9] = [
        (
            J::GreedyJoker,
            [(Diamonds, Seven), (Spades, Seven)],
            (10.0 + 14.0) * (2.0 + 3.0),
        ),
        (
            J::ScaryFace,
            [(Diamonds, Jack), (Spades, Jack)],
            (10.0 + 20.0 + 60.0) * 2.0,
        ),
        (
            J::SmileyFace,
            [(Diamonds, Jack), (Spades, Jack)],
            30.0 * (2.0 + 10.0),
        ),
        (
            J::EvenSteven,
            [(Diamonds, Ten), (Spades, Ten)],
            30.0 * (2.0 + 8.0),
        ),
        (
            J::OddTodd,
            [(Diamonds, Ace), (Spades, Ace)],
            (32.0 + 62.0) * 2.0,
        ),
        (
            J::Scholar,
            [(Diamonds, Ace), (Spades, Ace)],
            (32.0 + 40.0) * (2.0 + 8.0),
        ),
        (
            J::Fibonacci,
            [(Diamonds, Eight), (Spades, Eight)],
            26.0 * (2.0 + 16.0),
        ),
        (
            J::WalkieTalkie,
            [(Diamonds, Four), (Spades, Four)],
            (18.0 + 20.0) * (2.0 + 8.0),
        ),
        (
            J::Photograph,
            [(Diamonds, Queen), (Spades, Queen)],
            30.0 * (2.0 * 2.0),
        ),
    ];
    for (j, cards, expect) in cases {
        let mut run = with_jokers(&[j]);
        for (s, r) in cards {
            run.add_card_to_hand(s, r);
        }
        assert_eq!(play_first(&mut run, 2), expect, "{j:?}");
    }
}

#[test]
fn misprint_and_space_joker_use_their_streams() {
    let mut run = with_jokers(&[J::Misprint]);
    pair_of_kings(&mut run);
    let m = Streams::new(SEED).random_range("misprint", 0, 23) as f64;
    assert_eq!(play_first(&mut run, 2), 30.0 * (2.0 + m));

    let mut run = with_jokers(&[J::SpaceJoker]);
    pair_of_kings(&mut run);
    let up = Streams::new(SEED).random("space") < 0.25;
    let (c, m, lc, lm) = base_values(bm_core::poker::HandType::Pair);
    let (c, m) = if up { (c + lc, m + lm) } else { (c, m) };
    assert_eq!(play_first(&mut run, 2), (c + 20.0) * m);
}

#[test]
fn midas_mask_and_vampire_order() {
    // Midas first: the faces turn Gold, then Vampire strips them (+X0.2).
    let mut run = with_jokers(&[J::MidasMask, J::Vampire]);
    pair_of_kings(&mut run);
    assert_eq!(play_first(&mut run, 2), (30.0f64 * (2.0 * 1.2)).floor());
    assert!(run.cards.iter().all(|c| c.enhancement == Enhancement::None));
    // Vampire first: nothing to strip, then Midas makes them Gold.
    let mut run = with_jokers(&[J::Vampire, J::MidasMask]);
    pair_of_kings(&mut run);
    assert_eq!(play_first(&mut run, 2), 60.0);
    let golds = run
        .cards
        .iter()
        .filter(|c| c.enhancement == Enhancement::Gold)
        .count();
    assert_eq!(golds, 2);
}

#[test]
fn mr_bones_saves_at_a_quarter() {
    let mut run = with_jokers(&[J::MrBones]);
    run.hands_left = 1;
    // Small Blind 300: a Pair of Kings with Joker-less 60 points is 20%: lost.
    pair_of_kings(&mut run);
    run.play(&[0, 1]).unwrap();
    assert!(run.is_over());

    let mut run = with_jokers(&[J::MrBones]);
    run.hands_left = 1;
    for s in [Spades, Hearts, Clubs] {
        run.add_card_to_hand(s, King);
    }
    // Three of a Kind: (30 + 30) x 3 = 180 >= 75: saved, Mr. Bones leaves.
    run.play(&[0, 1, 2]).unwrap();
    assert!(!run.is_over());
    assert!(run.jokers.is_empty());
    assert_eq!(run.cash_out.as_ref().unwrap().blind, 0);
}

/// Every play of 1 to 5 plain cards, at random hand levels, scores
/// (base chips + rank chips of the scoring cards) x base mult.
#[test]
fn every_plain_play_scores_by_the_formula() {
    let mut run = small();
    let deck: Vec<u32> = standard_card_order()
        .iter()
        .map(|&(s, r)| run.add_card_to_hand(s, r))
        .collect();
    run.hand.clear();
    // Fixed, varied levels so the table lookups are exercised.
    for (i, h) in bm_core::poker::HandType::ALL.into_iter().enumerate() {
        run.hand_levels.level_up(h, (i % 4) as i32);
    }
    let mut n_checked = 0u64;
    for k in 1..=5usize {
        let mut idx: Vec<usize> = (0..k).collect();
        loop {
            let ids: Vec<u32> = idx.iter().map(|&i| deck[i]).collect();
            let det = {
                let cards: Vec<&bm_core::card::Card> = ids.iter().map(|&id| run.card(id)).collect();
                detect(&cards, DetectRules::default())
            };
            let info = *run.hand_levels.get(det.hand);
            let ranks: f64 = det
                .scoring
                .iter()
                .map(|&p| run.card(ids[p]).rank.chips() as f64)
                .sum();
            let expect = ((info.chips + ranks) * info.mult).floor();
            run.play = ids;
            let before = run.round_chips;
            let s = evaluate_play(&mut run);
            assert_eq!(s.score, expect);
            assert_eq!(run.round_chips - before, expect);
            run.play.clear();
            n_checked += 1;
            if !next_combination(&mut idx, 52) {
                break;
            }
        }
    }
    assert_eq!(n_checked, 2_893_163);
}

/// Step `idx` (a sorted k-subset of 0..n) to the next one; false at the end.
fn next_combination(idx: &mut [usize], n: usize) -> bool {
    let k = idx.len();
    let mut i = k;
    while i > 0 {
        i -= 1;
        if idx[i] < n - k + i {
            idx[i] += 1;
            for j in i + 1..k {
                idx[j] = idx[j - 1] + 1;
            }
            return true;
        }
    }
    false
}

/// Random plays of modified cards against an independent per-card model of
/// the scoring order: chips (rank or Stone, Bonus, perma), mult (Mult, Lucky),
/// money (Gold seal, Lucky), X-mult (Glass), then the edition; Red seals
/// retrigger; held Steel cards X1.5 (Red seal: twice); Glass rolls last.
#[test]
fn modified_cards_follow_the_card_order() {
    let enh = [
        Enhancement::None,
        Enhancement::Bonus,
        Enhancement::Mult,
        Enhancement::Wild,
        Enhancement::Glass,
        Enhancement::Steel,
        Enhancement::Stone,
        Enhancement::Gold,
        Enhancement::Lucky,
    ];
    let eds = [
        Edition::None,
        Edition::Foil,
        Edition::Holographic,
        Edition::Polychrome,
        Edition::Negative,
    ];
    let seals = [Seal::None, Seal::Gold, Seal::Red, Seal::Blue, Seal::Purple];
    let all = standard_card_order();
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = |n: usize| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % n as u64) as usize
    };
    for case in 0..20_000 {
        let seed = format!("C{case:07}");
        let mut run = Run::scenario(&seed, BlindKind::Small);
        // A target that can't be reached keeps the round going.
        run.blind.as_mut().unwrap().chips = f64::INFINITY;
        let n_play = 1 + next(5);
        let n_held = next(4);
        let mut ids = Vec::new();
        for _ in 0..n_play + n_held {
            let (s, r) = all[next(52)];
            let id = run.add_special_card_to_hand(s, r, enh[next(9)], eds[next(5)], seals[next(5)]);
            ids.push(id);
        }
        // Independent model.
        let played = &ids[..n_play];
        let det = {
            let cards: Vec<&bm_core::card::Card> = played.iter().map(|&id| run.card(id)).collect();
            detect(&cards, DetectRules::default())
        };
        let info = *run.hand_levels.get(det.hand);
        let mut rng = Streams::new(&seed);
        let (mut chips, mut mult, mut money) = (info.chips, info.mult, 0i64);
        for &p in &det.scoring {
            let c = run.card(played[p]).clone();
            let triggers = if c.seal == Seal::Red { 2 } else { 1 };
            for _ in 0..triggers {
                chips += match c.enhancement {
                    Enhancement::Stone => 50.0,
                    Enhancement::Bonus => c.rank.chips() as f64 + 30.0,
                    _ => c.rank.chips() as f64,
                };
                if c.enhancement == Enhancement::Mult {
                    mult += 4.0;
                }
                if c.enhancement == Enhancement::Lucky && rng.random("lucky_mult") < 0.2 {
                    mult += 20.0;
                }
                if c.seal == Seal::Gold {
                    money += 3;
                }
                if c.enhancement == Enhancement::Lucky && rng.random("lucky_money") < 1.0 / 15.0 {
                    money += 20;
                }
                if c.enhancement == Enhancement::Glass {
                    mult *= 2.0;
                }
                match c.edition {
                    Edition::Foil => chips += 50.0,
                    Edition::Holographic => mult += 10.0,
                    Edition::Polychrome => mult *= 1.5,
                    _ => {}
                }
            }
        }
        for &id in &ids[n_play..] {
            let c = run.card(id);
            if c.enhancement == Enhancement::Steel {
                let passes = if c.seal == Seal::Red { 2 } else { 1 };
                for _ in 0..passes {
                    mult *= 1.5;
                }
            }
        }
        let expect = (chips * mult).floor();
        let shattered: Vec<u32> = det
            .scoring
            .iter()
            .map(|&p| played[p])
            .filter(|&id| run.card(id).enhancement == Enhancement::Glass)
            .filter(|_| rng.random("glass") < 0.25)
            .collect();
        let money_before = run.money;
        let pos: Vec<usize> = (0..n_play).collect();
        let s = run.play(&pos).unwrap();
        assert_eq!(s.score, expect, "case {case}");
        assert_eq!(s.destroyed, shattered, "case {case}");
        assert_eq!(run.money - money_before, money, "case {case}");
    }
}
