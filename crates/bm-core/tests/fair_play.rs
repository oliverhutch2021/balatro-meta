//! The fair-play wall: the bot's input depends only on what the screen
//! shows.

use bm_core::bm_rng::{SeedSampler, Streams};
use bm_core::joker_data::JokerKind;
use bm_core::observe::{encode, encode_value, fair_copy, screen, OBS_LEN, VALUE_LEN};
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
fn a_fair_copy_never_reads_the_real_draw_order() {
    // The same cards in another order give the same fair copies: lookahead learns nothing of the real
    // order, and positions holding the same cards are judged on the same deals.
    let mut run = Run::new(RunConfig::new("FAIRCOPY"));
    run.select_blind().unwrap();
    let mut other = run.clone();
    other.deck.reverse();
    for salt in 0..5 {
        assert_eq!(fair_copy(&run, salt).deck, fair_copy(&other, salt).deck);
    }
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

#[test]
fn the_value_input_counts_the_jokers_by_kind() {
    use bm_core::joker_data::N_JOKERS;
    let mut run = Run::new(RunConfig::new("VALUEIN1"));
    for k in [JokerKind::Joker, JokerKind::Blueprint, JokerKind::Joker] {
        run.add_joker(k);
    }
    let (mut a, mut v) = (Vec::new(), Vec::new());
    encode(&screen(&run), &mut a);
    encode_value(&screen(&run), &mut v);
    assert_eq!(v.len(), VALUE_LEN);
    assert_eq!(&v[..OBS_LEN], &a[..], "the screen's encoding comes first");
    let held = &v[OBS_LEN..OBS_LEN + N_JOKERS];
    assert_eq!(held[JokerKind::Joker.index()], 2.0);
    assert_eq!(held[JokerKind::Blueprint.index()], 1.0);
    assert_eq!(held.iter().sum::<f32>(), 3.0);
    assert!(
        v[OBS_LEN + N_JOKERS..].iter().all(|&x| x == 0.0),
        "no shop or pack yet"
    );
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
            // The value network's input too (it reads the shop's and packs' jokers by kind).
            encode_value(&screen(&run), &mut a);
            encode_value(&screen(&t), &mut b);
            assert_eq!(a.len(), VALUE_LEN);
            assert!(
                a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()),
                "value inputs differ: run {r} step {step} ({:?})",
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

/// A round under The Mark (face cards drawn face down): four cards in sight, one of each suit, and two face cards
/// from the deck, face down. And the same round with another face card of the first one's suit in its place: in
/// sight of the same cards, sorted by rank or by suit it sits where the first did, so nothing on the screen tells
/// the two rounds apart.
fn mark_round(jokers: &[JokerKind]) -> (Run, Run) {
    use bm_core::blinds::{BlindKind, Boss};
    use bm_core::card::{Rank, Suit};
    let mut run = Run::scenario("FACEDOWN", BlindKind::Boss(Boss::Mark));
    run.hand.clear();
    for &k in jokers {
        run.add_joker(k);
    }
    for (s, r) in [
        (Suit::Clubs, Rank::Two),
        (Suit::Hearts, Rank::Five),
        (Suit::Spades, Rank::Five),
        (Suit::Diamonds, Rank::Nine),
    ] {
        run.add_card_to_hand(s, r);
    }
    let faces: Vec<usize> = (0..run.deck.len())
        .filter(|&j| run.card(run.deck[j]).rank.is_face())
        .take(2)
        .collect();
    for &j in faces.iter().rev() {
        let id = run.deck.remove(j);
        run.hand.push(id);
        run.card_mut(id).face_down = true;
    }
    let mut other = run.clone();
    let suit = other.card(other.hand[4]).suit;
    let j = (0..other.deck.len())
        .find(|&j| {
            let c = other.card(other.deck[j]);
            c.rank.is_face() && c.suit == suit
        })
        .unwrap();
    let (a, b) = (other.hand[4], other.deck[j]);
    other.hand[4] = b;
    other.deck[j] = a;
    other.card_mut(b).face_down = true;
    other.card_mut(a).face_down = false;
    (run, other)
}

#[test]
fn face_down_cards_in_hand_are_dealt_again_in_a_fair_copy() {
    // The player sees card backs: a lookahead must not play the real cards, only cards that fit what the screen
    // tells (under The Mark a face card, and with a card of each suit in sight, sorting by suit tells its suit).
    let (run, other) = mark_round(&[]);
    let all = |r: &Run| {
        let mut v: Vec<u32> = r.deck.iter().chain(r.hand.iter()).copied().collect();
        v.sort();
        v
    };
    let mut dealt = std::collections::HashSet::new();
    for salt in 0..20 {
        let c = fair_copy(&run, salt);
        assert_eq!(all(&c), all(&run), "the same cards");
        assert_eq!(c.hand[..4], run.hand[..4], "the cards in sight stay");
        for i in 4..6 {
            let card = c.card(c.hand[i]);
            assert!(
                card.face_down && card.rank.is_face(),
                "a face card, face down"
            );
            assert_eq!(
                card.suit,
                run.card(run.hand[i]).suit,
                "sorting by suit shows its suit"
            );
            dealt.insert(c.hand[i]);
        }
        assert!(c.deck.iter().all(|&id| !c.card(id).face_down));
    }
    assert!(dealt.len() >= 3, "dealt again: {dealt:?}");
    // Which of those cards lie face down makes no difference to the copies.
    for salt in 0..5 {
        let (x, y) = (fair_copy(&run, salt), fair_copy(&other, salt));
        assert_eq!(x.hand, y.hand);
        assert_eq!(x.deck, y.deck);
    }
}

#[test]
fn the_teacher_plays_the_hand_it_sees_not_the_cards_face_down() {
    // Jokers that care what the face cards are (Photograph: the first face card scored; Baron: Kings held;
    // Scary Face): the teacher's move is the same whichever of them lie face down.
    let (run, other) = mark_round(&[
        JokerKind::Photograph,
        JokerKind::Baron,
        JokerKind::ScaryFace,
    ]);
    for salt in 0..8 {
        assert_eq!(
            bm_core::teacher::teacher_move(&run, salt),
            bm_core::teacher::teacher_move(&other, salt),
            "salt {salt}"
        );
    }
}

/// Amber Acorn at ante 8: the jokers flipped and shuffled, so the player knows which five it holds but not where
/// each one lies. Hearts Ace to Eight in hand (Straight Flushes to build), the joker row turned by `turn` places
/// (`order` gives the creation order: their sort ids, and so what Amber Acorn's own shuffle starts from, stay the
/// same). Every turn shows the same screen.
fn acorn_round(turn: usize, straight_flush_level: i32) -> Run {
    use bm_core::blinds::{BlindKind, Boss};
    use bm_core::card::{Rank, Suit};
    use bm_core::poker::HandType;
    let mut run = Run::scenario("ACORNLUC", BlindKind::Boss(Boss::AmberAcorn));
    run.ante = 8;
    run.blind.as_mut().unwrap().chips = 100_000.0;
    run.hand.clear();
    for r in [
        Rank::Ace,
        Rank::Two,
        Rank::Three,
        Rank::Four,
        Rank::Five,
        Rank::Six,
        Rank::Seven,
        Rank::Eight,
    ] {
        run.add_card_to_hand(Suit::Hearts, r);
    }
    for k in [
        JokerKind::Luchador,
        JokerKind::Joker,
        JokerKind::Cavendish,
        JokerKind::Blueprint,
        JokerKind::Brainstorm,
    ] {
        run.add_joker(k);
    }
    run.jokers.rotate_left(turn);
    for j in run.jokers.iter_mut() {
        j.face_down = true;
    }
    run.hand_levels
        .level_up(HandType::StraightFlush, straight_flush_level - 1);
    run
}

#[test]
fn the_teacher_cannot_tell_which_face_down_joker_is_which() {
    // Which face-down joker lies where is hidden: whatever the teacher does (sell a joker to disable the boss, play,
    // discard), it must do the same wherever the hidden jokers really are. A sale aimed at the real Luchador's slot
    // would read the hidden row.
    for level in 2..=8 {
        let runs: Vec<Run> = (0..5).map(|t| acorn_round(t, level)).collect();
        for r in &runs[1..] {
            assert_eq!(screen(r), screen(&runs[0]), "the same screen");
        }
        for salt in 0..6u64 {
            let moves: Vec<_> = runs
                .iter()
                .map(|r| {
                    bm_core::teacher::new_run();
                    bm_core::teacher::teacher_move(r, salt)
                })
                .collect();
            assert!(
                moves.iter().all(|m| *m == moves[0]),
                "level {level}, salt {salt}: {moves:?}"
            );
        }
    }
}

#[test]
fn a_fair_copy_places_face_down_jokers_from_what_the_player_knows() {
    // The copies of two runs that differ only in where the face-down jokers lie are the same copies: the
    // reshuffle starts from the jokers themselves (in creation order, as Amber Acorn's does), never from the row.
    let (a, b) = (acorn_round(0, 1), acorn_round(2, 1));
    let kinds = |r: &Run| r.jokers.iter().map(|j| j.kind).collect::<Vec<_>>();
    assert_ne!(kinds(&a), kinds(&b));
    let mut rows = std::collections::HashSet::new();
    for salt in 0..40 {
        let (x, y) = (fair_copy(&a, salt), fair_copy(&b, salt));
        assert_eq!(kinds(&x), kinds(&y), "salt {salt}");
        assert!(x.jokers.iter().all(|j| j.face_down));
        rows.insert(kinds(&x));
    }
    assert!(
        rows.len() >= 20,
        "the copies place them many ways: {}",
        rows.len()
    );
}

#[test]
fn a_fair_copy_does_not_know_the_ante_s_voucher_before_a_shop_shows_it() {
    // A run's first voucher is drawn at the start but shown only in its first shop: until then a fair copy draws
    // its own. Once a shop has shown it, it is known and stays.
    let mut differs = 0;
    for i in 0..30 {
        let run = Run::new(RunConfig::new(&format!("VOUCHER{i}")));
        let real = run.shop_voucher;
        assert!(real.is_some());
        let copies: Vec<_> = (0..4).map(|s| fair_copy(&run, s).shop_voucher).collect();
        assert!(copies.iter().all(|v| v.is_some()));
        if copies.iter().any(|&v| v != real) {
            differs += 1;
        }
        // Into the first shop: from there the voucher is on the screen, and every copy keeps it.
        let mut shown = run.clone();
        shown.select_blind().unwrap();
        shown.blind.as_mut().unwrap().chips = 1.0;
        shown.play(&[0]).unwrap();
        shown.cash_out().unwrap();
        assert_eq!(shown.phase, Phase::Shop);
        for s in 0..4 {
            assert_eq!(fair_copy(&shown, s).shop_voucher, shown.shop_voucher);
        }
    }
    assert!(
        differs >= 25,
        "only {differs} of 30 runs' copies drew another voucher"
    );
}

#[test]
fn a_fair_copy_does_not_carry_the_real_seed() {
    let run = Run::new(RunConfig::new("REALSEED"));
    for s in 0..5 {
        let c = fair_copy(&run, s);
        assert_ne!(c.cfg.seed, run.cfg.seed);
        assert_eq!(
            c.rng.seed(),
            c.cfg.seed,
            "the copy's seed is the one its streams come from"
        );
    }
}

#[test]
fn held_tarots_are_used_on_the_cards_in_sight_whichever_lie_face_down() {
    use bm_core::content::ConsumableKind;
    for kind in [
        ConsumableKind::Death,
        ConsumableKind::Strength,
        ConsumableKind::HangedMan,
        ConsumableKind::Magician,
    ] {
        let (mut run, mut other) = mark_round(&[JokerKind::Photograph]);
        run.give_consumable(kind);
        other.give_consumable(kind);
        for salt in 0..6 {
            // Each position as a run of its own (the teacher forgets what it remembered of the last one).
            let decide = |r: &Run| {
                bm_core::teacher::new_run();
                bm_core::teacher::teacher_move(r, salt)
            };
            assert_eq!(decide(&run), decide(&other), "{kind:?}, salt {salt}");
        }
    }
}
