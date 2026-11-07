//! Checks for the jokers added in phase 2, the seals that make consumables,
//! and Observatory. Every expected number is worked out by hand from
//! `rules/jokers_1_76.md`, `rules/jokers_77_150.md` and `rules/scoring.md`,
//! not read back from the engine.

use bm_core::blinds::{BlindKind, Boss};
use bm_core::card::{Edition, Enhancement, Rank, Seal, Suit};
use bm_core::content::{ConsumableKind, ConsumableSet, VoucherKind};
use bm_core::joker_data::JokerKind as J;
use bm_core::poker::HandType;
use bm_core::state::HeldTag;
use bm_core::tags::TagKind;
use bm_core::{Phase, Run, RunConfig};

use Rank::*;
use Suit::*;

const SEED: &str = "TESTSEED";

fn small() -> Run {
    Run::scenario(SEED, BlindKind::Small)
}

fn with_jokers(jokers: &[J]) -> Run {
    let mut run = small();
    for &j in jokers {
        run.add_joker(j);
    }
    run
}

/// Play the hand cards at these positions and return the hand's score.
fn play(run: &mut Run, pos: &[usize]) -> f64 {
    run.play(pos).unwrap().score
}

fn pair_of_kings(run: &mut Run) {
    run.add_card_to_hand(Spades, King);
    run.add_card_to_hand(Hearts, King);
}

/// A run at blind select with these jokers.
fn at_blind_select(jokers: &[J]) -> Run {
    let mut run = Run::new(RunConfig::new(SEED));
    for &j in jokers {
        run.add_joker(j);
    }
    run
}

fn set_deck_enhancement(run: &mut Run, n: usize, e: Enhancement) {
    for i in 0..n {
        let id = run.deck[i];
        run.card_mut(id).enhancement = e;
    }
}

fn joker(run: &Run, kind: J) -> &bm_core::jokers::Joker {
    run.jokers.iter().find(|j| j.kind == kind).unwrap()
}

// ------------------------------------------------------- main context

#[test]
fn joker_stencil_counts_free_slots_and_stencils() {
    // Alone: 5 slots, 1 joker, 1 Stencil: X(4 + 1).
    let mut run = with_jokers(&[J::JokerStencil]);
    pair_of_kings(&mut run);
    assert_eq!(play(&mut run, &[0, 1]), 30.0 * 2.0 * 5.0);
    // With a Joker to its left: X(3 + 1) after +4.
    let mut run = with_jokers(&[J::Joker, J::JokerStencil]);
    pair_of_kings(&mut run);
    assert_eq!(play(&mut run, &[0, 1]), 30.0 * 6.0 * 4.0);
    // Five jokers, one Stencil: X1, nothing.
    let mut run = with_jokers(&[J::JokerStencil, J::Joker, J::Joker, J::Joker, J::Joker]);
    pair_of_kings(&mut run);
    assert_eq!(play(&mut run, &[0, 1]), 30.0 * (2.0 + 16.0));
}

#[test]
fn steel_joker_counts_steel_cards_in_the_full_deck() {
    let mut run = with_jokers(&[J::SteelJoker]);
    set_deck_enhancement(&mut run, 2, Enhancement::Steel);
    pair_of_kings(&mut run);
    let expected = (30.0f64 * (2.0 * (1.0 + 0.2 * 2.0))).floor();
    assert_eq!(play(&mut run, &[0, 1]), expected);
    // No Steel card: nothing.
    let mut run = with_jokers(&[J::SteelJoker]);
    pair_of_kings(&mut run);
    assert_eq!(play(&mut run, &[0, 1]), 60.0);
}

#[test]
fn stone_joker_and_drivers_license() {
    let mut run = with_jokers(&[J::StoneJoker]);
    set_deck_enhancement(&mut run, 2, Enhancement::Stone);
    pair_of_kings(&mut run);
    assert_eq!(play(&mut run, &[0, 1]), (30.0 + 50.0) * 2.0);

    let mut run = with_jokers(&[J::DriversLicense]);
    set_deck_enhancement(&mut run, 16, Enhancement::Bonus);
    pair_of_kings(&mut run);
    assert_eq!(play(&mut run, &[0, 1]), 30.0 * 2.0 * 3.0);
    let mut run = with_jokers(&[J::DriversLicense]);
    set_deck_enhancement(&mut run, 15, Enhancement::Bonus);
    pair_of_kings(&mut run);
    assert_eq!(play(&mut run, &[0, 1]), 60.0);
}

#[test]
fn per_frame_values_come_from_before_the_hand() {
    // Vampire strips the played Steel card in 'before'; Steel Joker still
    // counts it this hand (X1.2), and Vampire then gives X1.1.
    let mut run = with_jokers(&[J::Vampire, J::SteelJoker]);
    run.add_special_card_to_hand(Spades, King, Enhancement::Steel, Edition::None, Seal::None);
    run.add_card_to_hand(Hearts, King);
    let mult: f64 = 2.0 * 1.1 * (1.0 + 0.2 * 1.0);
    assert_eq!(play(&mut run, &[0, 1]), (30.0 * mult).floor());
}

#[test]
fn blackboard_needs_every_held_card_black() {
    let mut run = with_jokers(&[J::Blackboard]);
    pair_of_kings(&mut run);
    run.add_card_to_hand(Spades, Two);
    run.add_card_to_hand(Clubs, Three);
    assert_eq!(play(&mut run, &[0, 1]), 30.0 * 2.0 * 3.0);

    let mut run = with_jokers(&[J::Blackboard]);
    pair_of_kings(&mut run);
    run.add_card_to_hand(Spades, Two);
    run.add_card_to_hand(Hearts, Four);
    assert_eq!(play(&mut run, &[0, 1]), 60.0);

    // A Wild card counts as black; no held cards at all also gives X3.
    let mut run = with_jokers(&[J::Blackboard]);
    pair_of_kings(&mut run);
    run.add_special_card_to_hand(Hearts, Two, Enhancement::Wild, Edition::None, Seal::None);
    assert_eq!(play(&mut run, &[0, 1]), 180.0);
    let mut run = with_jokers(&[J::Blackboard]);
    pair_of_kings(&mut run);
    assert_eq!(play(&mut run, &[0, 1]), 180.0);
}

#[test]
fn flower_pot_needs_all_four_suits() {
    let mut run = with_jokers(&[J::FlowerPot]);
    run.add_card_to_hand(Spades, King);
    run.add_card_to_hand(Hearts, King);
    run.add_card_to_hand(Diamonds, Queen);
    run.add_card_to_hand(Clubs, Queen);
    // Two Pair: (20 + 40) x 2, X3.
    assert_eq!(play(&mut run, &[0, 1, 2, 3]), 60.0 * 2.0 * 3.0);

    let mut run = with_jokers(&[J::FlowerPot]);
    run.add_card_to_hand(Spades, King);
    run.add_card_to_hand(Hearts, King);
    run.add_card_to_hand(Spades, Queen);
    run.add_card_to_hand(Hearts, Queen);
    assert_eq!(play(&mut run, &[0, 1, 2, 3]), 120.0);

    // Heart, Diamond, Spade and a Wild card: X3.
    let mut run = with_jokers(&[J::FlowerPot]);
    run.add_card_to_hand(Hearts, King);
    run.add_card_to_hand(Diamonds, King);
    run.add_card_to_hand(Spades, Queen);
    run.add_special_card_to_hand(Hearts, Queen, Enhancement::Wild, Edition::None, Seal::None);
    assert_eq!(play(&mut run, &[0, 1, 2, 3]), 360.0);
}

#[test]
fn seeing_double_needs_a_club_and_another_suit() {
    let mut run = with_jokers(&[J::SeeingDouble]);
    run.add_card_to_hand(Clubs, King);
    run.add_card_to_hand(Hearts, King);
    assert_eq!(play(&mut run, &[0, 1]), 30.0 * 2.0 * 2.0);

    let mut run = with_jokers(&[J::SeeingDouble]);
    pair_of_kings(&mut run);
    assert_eq!(play(&mut run, &[0, 1]), 60.0);

    // Two Wild cards: Clubs, then Diamonds.
    let mut run = with_jokers(&[J::SeeingDouble]);
    run.add_special_card_to_hand(Hearts, King, Enhancement::Wild, Edition::None, Seal::None);
    run.add_special_card_to_hand(Hearts, King, Enhancement::Wild, Edition::None, Seal::None);
    assert_eq!(play(&mut run, &[0, 1]), 120.0);
}

#[test]
fn loyalty_card_fires_on_every_sixth_hand_after_it_was_made() {
    let mut run = with_jokers(&[J::LoyaltyCard]);
    run.hands_played_run = 5;
    pair_of_kings(&mut run);
    assert_eq!(play(&mut run, &[0, 1]), 30.0 * 2.0 * 4.0);

    let mut run = with_jokers(&[J::LoyaltyCard]);
    run.hands_played_run = 4;
    pair_of_kings(&mut run);
    assert_eq!(play(&mut run, &[0, 1]), 60.0);

    let mut run = with_jokers(&[J::LoyaltyCard]);
    run.hands_played_run = 11;
    pair_of_kings(&mut run);
    assert_eq!(play(&mut run, &[0, 1]), 240.0);
}

#[test]
fn swashbuckler_throwback_fortune_teller_and_stored_values() {
    // Joker's sell value is $1.
    let mut run = with_jokers(&[J::Joker, J::Swashbuckler]);
    pair_of_kings(&mut run);
    assert_eq!(play(&mut run, &[0, 1]), 30.0 * (2.0 + 4.0 + 1.0));

    let mut run = with_jokers(&[J::Throwback]);
    run.skips = 2;
    pair_of_kings(&mut run);
    assert_eq!(play(&mut run, &[0, 1]), 30.0 * 2.0 * 1.5);

    let mut run = with_jokers(&[J::FortuneTeller]);
    run.usage_total.tarot = 3;
    pair_of_kings(&mut run);
    assert_eq!(play(&mut run, &[0, 1]), 30.0 * 5.0);

    let mut run = with_jokers(&[J::RedCard, J::Castle]);
    run.jokers[0].mult = 3.0;
    run.jokers[1].chips = 9.0;
    pair_of_kings(&mut run);
    assert_eq!(play(&mut run, &[0, 1]), 39.0 * 5.0);
}

#[test]
fn observatory_gives_x1_5_per_matching_planet() {
    let mut run = small();
    run.vouchers[VoucherKind::Observatory.index()] = true;
    run.give_consumable(ConsumableKind::Pluto);
    run.give_consumable(ConsumableKind::Pluto);
    run.add_card_to_hand(Spades, King);
    // High Card: (5 + 10) x 1 x 1.5 x 1.5.
    assert_eq!(play(&mut run, &[0]), (15.0 * 2.25f64).floor());
    // A Planet of another hand does nothing.
    let mut run = small();
    run.vouchers[VoucherKind::Observatory.index()] = true;
    run.give_consumable(ConsumableKind::Mercury);
    run.add_card_to_hand(Spades, King);
    assert_eq!(play(&mut run, &[0]), 15.0);
}

// ------------------------------------------------ creations in scoring

#[test]
fn eight_ball_makes_a_tarot_per_eight_while_there_is_room() {
    let mut run = with_jokers(&[J::EightBall]);
    run.probability = 4.0; // every roll hits
    run.add_card_to_hand(Spades, Eight);
    run.add_card_to_hand(Hearts, Eight);
    run.add_card_to_hand(Clubs, Eight);
    play(&mut run, &[0, 1, 2]);
    assert_eq!(run.consumables.len(), 2);
    assert!(run
        .consumables
        .iter()
        .all(|c| c.kind.set() == ConsumableSet::Tarot));
}

#[test]
fn superposition_seance_and_vagabond() {
    let mut run = with_jokers(&[J::Superposition]);
    for (s, r) in [
        (Spades, Ace),
        (Hearts, Two),
        (Clubs, Three),
        (Diamonds, Four),
        (Spades, Five),
    ] {
        run.add_card_to_hand(s, r);
    }
    play(&mut run, &[0, 1, 2, 3, 4]);
    assert_eq!(run.consumables.len(), 1);
    assert_eq!(run.consumables[0].kind.set(), ConsumableSet::Tarot);

    let mut run = with_jokers(&[J::Seance]);
    for r in [Five, Six, Seven, Eight, Nine] {
        run.add_card_to_hand(Spades, r);
    }
    play(&mut run, &[0, 1, 2, 3, 4]);
    assert_eq!(run.consumables.len(), 1);
    let k = run.consumables[0].kind;
    assert_eq!(k.set(), ConsumableSet::Spectral);
    assert!(k != ConsumableKind::Soul && k != ConsumableKind::BlackHole);

    let mut run = with_jokers(&[J::Vagabond]);
    run.money = 4;
    pair_of_kings(&mut run);
    play(&mut run, &[0, 1]);
    assert_eq!(run.consumables.len(), 1);
    let mut run = with_jokers(&[J::Vagabond]);
    run.money = 5;
    pair_of_kings(&mut run);
    play(&mut run, &[0, 1]);
    assert!(run.consumables.is_empty());
}

#[test]
fn to_do_list_pays_for_its_hand() {
    let mut run = with_jokers(&[J::ToDoList]);
    run.jokers[0].hand = Some(HandType::Pair);
    pair_of_kings(&mut run);
    let before = run.money;
    play(&mut run, &[0, 1]);
    assert_eq!(run.money, before + 4);
}

#[test]
fn dna_copies_a_lone_first_card_into_the_hand() {
    let mut run = with_jokers(&[J::DNA, J::Hologram]);
    run.blind.as_mut().unwrap().chips = 1e9;
    run.add_special_card_to_hand(Spades, King, Enhancement::Mult, Edition::Foil, Seal::Red);
    let n = run.playing_cards.len();
    play(&mut run, &[0]);
    assert_eq!(run.playing_cards.len(), n + 1);
    let copy = *run.playing_cards.last().unwrap();
    let c = run.card(copy);
    assert_eq!((c.suit, c.rank), (Spades, King));
    assert_eq!(
        (c.enhancement, c.edition, c.seal),
        (Enhancement::Mult, Edition::Foil, Seal::Red)
    );
    assert!(run.hand.contains(&copy));
    assert_eq!(joker(&run, J::Hologram).x_mult, 1.25);
    // Not on the second hand.
    let n = run.playing_cards.len();
    play(&mut run, &[0]);
    assert_eq!(run.playing_cards.len(), n);
}

#[test]
fn sixth_sense_destroys_a_lone_six_and_makes_a_spectral() {
    let mut run = with_jokers(&[J::SixthSense]);
    let six = run.add_card_to_hand(Spades, Six);
    play(&mut run, &[0]);
    assert!(!run.playing_cards.contains(&six));
    assert_eq!(run.consumables.len(), 1);
    assert_eq!(run.consumables[0].kind.set(), ConsumableSet::Spectral);
    // A Blueprint copy is never asked: still one Spectral.
    let mut run = with_jokers(&[J::Blueprint, J::SixthSense]);
    run.add_card_to_hand(Spades, Six);
    play(&mut run, &[0]);
    assert_eq!(run.consumables.len(), 1);
}

#[test]
fn glass_joker_and_caino_gain_from_shattered_cards() {
    let mut run = with_jokers(&[J::GlassJoker, J::Caino]);
    run.probability = 4.0; // Glass always shatters
    let k =
        run.add_special_card_to_hand(Spades, King, Enhancement::Glass, Edition::None, Seal::None);
    run.add_card_to_hand(Hearts, King);
    play(&mut run, &[0, 1]);
    assert!(!run.playing_cards.contains(&k));
    assert_eq!(joker(&run, J::GlassJoker).x_mult, 1.75);
    assert_eq!(joker(&run, J::Caino).x_mult, 2.0);
}

// ------------------------------------------------------------ discards

#[test]
fn faceless_mail_in_and_trading_card_pay() {
    let mut run = with_jokers(&[J::FacelessJoker]);
    run.add_card_to_hand(Spades, King);
    run.add_card_to_hand(Hearts, Queen);
    run.add_card_to_hand(Clubs, Jack);
    let before = run.money;
    run.discard(&[0, 1, 2]).unwrap();
    assert_eq!(run.money, before + 5);

    let mut run = with_jokers(&[J::MailInRebate]);
    let r = run.mail_rank;
    run.add_card_to_hand(Spades, r);
    run.add_card_to_hand(Hearts, r);
    let before = run.money;
    run.discard(&[0, 1]).unwrap();
    assert_eq!(run.money, before + 10);

    let mut run = with_jokers(&[J::TradingCard]);
    let c = run.add_card_to_hand(Spades, Two);
    let before = run.money;
    run.discard(&[0]).unwrap();
    assert_eq!(run.money, before + 3);
    assert!(!run.playing_cards.contains(&c));
    // Only the round's first discard.
    let d = run.add_card_to_hand(Spades, Three);
    let pos = run.hand.iter().position(|&x| x == d).unwrap();
    run.discard(&[pos]).unwrap();
    assert!(run.playing_cards.contains(&d));
}

#[test]
fn ramen_castle_hit_the_road_and_yorick_count_discarded_cards() {
    let mut run = with_jokers(&[J::Ramen, J::Castle, J::HitTheRoad, J::Yorick]);
    run.jokers[3].counter = 2;
    let suit = run.castle_suit;
    run.add_card_to_hand(suit, Jack);
    run.add_card_to_hand(suit, Two);
    run.discard(&[0, 1]).unwrap();
    let mut ramen = 2.0f64;
    ramen -= 0.01;
    ramen -= 0.01;
    assert_eq!(joker(&run, J::Ramen).x_mult, ramen);
    assert_eq!(joker(&run, J::Castle).chips, 6.0);
    assert_eq!(joker(&run, J::HitTheRoad).x_mult, 1.5);
    let y = joker(&run, J::Yorick);
    assert_eq!((y.x_mult, y.counter), (2.0, 23));
}

#[test]
fn burnt_joker_levels_the_first_discarded_hand() {
    let mut run = with_jokers(&[J::BurntJoker]);
    pair_of_kings(&mut run);
    run.discard(&[0, 1]).unwrap();
    assert_eq!(run.hand_levels.get(HandType::Pair).level, 2);
    // Not on the second discard.
    pair_of_kings(&mut run);
    let n = run.hand.len();
    run.discard(&[n - 2, n - 1]).unwrap();
    assert_eq!(run.hand_levels.get(HandType::Pair).level, 2);
}

#[test]
fn purple_seal_makes_a_tarot_when_discarded() {
    let mut run = small();
    run.add_special_card_to_hand(Spades, Two, Enhancement::None, Edition::None, Seal::Purple);
    run.discard(&[0]).unwrap();
    assert_eq!(run.consumables.len(), 1);
    assert_eq!(run.consumables[0].kind.set(), ConsumableSet::Tarot);
}

// --------------------------------------------------------- blind start

#[test]
fn ceremonial_dagger_eats_its_right_neighbour() {
    let mut run = at_blind_select(&[J::CeremonialDagger, J::Joker]);
    run.select_blind().unwrap();
    assert_eq!(run.jokers.len(), 1);
    // The Joker's sell value was $1: +2 mult.
    assert_eq!(run.jokers[0].mult, 2.0);
}

#[test]
fn burglar_marble_cartomancer_riff_raff_and_madness() {
    let mut run = at_blind_select(&[J::Burglar]);
    run.select_blind().unwrap();
    assert_eq!((run.hands_left, run.discards_left), (7, 0));

    let mut run = at_blind_select(&[J::MarbleJoker]);
    run.select_blind().unwrap();
    assert_eq!(run.playing_cards.len(), 53);
    let stone = *run.playing_cards.last().unwrap();
    assert_eq!(run.card(stone).enhancement, Enhancement::Stone);

    let mut run = at_blind_select(&[J::Cartomancer]);
    run.select_blind().unwrap();
    assert_eq!(run.consumables.len(), 1);

    let mut run = at_blind_select(&[J::RiffRaff]);
    run.select_blind().unwrap();
    assert_eq!(run.jokers.len(), 3);
    assert!(run.jokers[1..].iter().all(|j| j.rarity() == 1));

    let mut run = at_blind_select(&[J::Madness, J::Joker]);
    run.select_blind().unwrap();
    assert_eq!(run.jokers.len(), 1);
    assert_eq!(run.jokers[0].x_mult, 1.5);
}

#[test]
fn certificate_adds_a_sealed_card_on_the_first_draw() {
    let mut run = at_blind_select(&[J::Certificate, J::Hologram]);
    run.select_blind().unwrap();
    assert_eq!(run.hand.len(), 9);
    assert_eq!(run.playing_cards.len(), 53);
    let new = *run.playing_cards.last().unwrap();
    assert_ne!(run.card(new).seal, Seal::None);
    assert_eq!(joker(&run, J::Hologram).x_mult, 1.25);
}

// ------------------------------------------------- end of round, cash-out

/// Win the current round with one hand.
fn win_round(run: &mut Run) {
    run.blind.as_mut().unwrap().chips = 1.0;
    pair_of_kings(run);
    let n = run.hand.len();
    play(run, &[n - 2, n - 1]);
    assert_eq!(run.phase, Phase::RoundEval);
}

#[test]
fn end_of_round_jokers() {
    let mut run = with_jokers(&[J::Egg, J::GiftCard, J::InvisibleJoker, J::ToDoList]);
    let todo = run.jokers[3].hand;
    win_round(&mut run);
    // Egg: $2 + $3 + $1 (Gift Card), Gift Card: $3 + $1.
    assert_eq!(run.jokers[0].sell_cost, 2 + 3 + 1);
    assert_eq!(run.jokers[1].sell_cost, 3 + 1);
    assert_eq!(run.jokers[2].counter, 1);
    assert_ne!(run.jokers[3].hand, todo);
}

#[test]
fn cash_out_rows() {
    let mut run = with_jokers(&[J::Cloud9, J::Rocket, J::Satellite, J::DelayedGratification]);
    run.usage[ConsumableKind::Pluto.index()] = 2;
    run.usage[ConsumableKind::Mars.index()] = 1;
    win_round(&mut run);
    let rows: Vec<i64> = run
        .cash_out
        .as_ref()
        .unwrap()
        .jokers
        .iter()
        .map(|r| r.1)
        .collect();
    // Four 9s; Rocket $1; two different Planets; 4 discards unused (the
    // Red Deck's +1).
    assert_eq!(rows, vec![4, 1, 2, 8]);
}

#[test]
fn rocket_grows_after_a_boss_and_campfire_resets() {
    let mut run = Run::scenario(SEED, BlindKind::Boss(Boss::Arm));
    run.add_joker(J::Rocket);
    run.add_joker(J::Campfire);
    run.jokers[1].x_mult = 2.0;
    win_round(&mut run);
    assert_eq!(run.jokers[0].counter, 3);
    assert_eq!(run.cash_out.as_ref().unwrap().jokers[0].1, 3);
    assert_eq!(run.jokers[1].x_mult, 1.0);
}

#[test]
fn blue_seal_makes_the_planet_of_the_last_hand() {
    let mut run = small();
    run.add_special_card_to_hand(Clubs, Two, Enhancement::None, Edition::None, Seal::Blue);
    win_round(&mut run);
    assert_eq!(run.consumables.len(), 1);
    assert_eq!(run.consumables[0].kind, ConsumableKind::Mercury);
}

#[test]
fn to_the_moon_raises_interest() {
    let mut run = with_jokers(&[J::ToTheMoon]);
    run.money = 25;
    win_round(&mut run);
    assert_eq!(run.cash_out.as_ref().unwrap().interest, 10);
}

// --------------------------------------------------------- sales, shop

#[test]
fn selling_luchador_disables_the_boss() {
    let mut run = Run::scenario(SEED, BlindKind::Boss(Boss::Wall));
    run.add_joker(J::Luchador);
    let target = run.blind.as_ref().unwrap().chips;
    run.sell_joker(0).unwrap();
    let b = run.blind.as_ref().unwrap();
    assert!(b.disabled);
    assert_eq!(b.chips, target / 2.0);
}

#[test]
fn diet_cola_invisible_joker_and_campfire_sales() {
    let mut run = with_jokers(&[J::DietCola]);
    run.sell_joker(0).unwrap();
    assert!(run.tags.iter().any(|t| t.kind == TagKind::Double));

    let mut run = with_jokers(&[J::Joker, J::InvisibleJoker, J::Campfire]);
    run.jokers[1].counter = 2;
    run.sell_joker(1).unwrap();
    // A copy of Joker or Campfire joined; Campfire heard of the sale.
    assert_eq!(run.jokers.len(), 3);
    let campfires: Vec<f64> = run
        .jokers
        .iter()
        .filter(|j| j.kind == J::Campfire)
        .map(|j| j.x_mult)
        .collect();
    assert!(campfires.iter().all(|&x| x == 1.25));
}

#[test]
fn chaos_the_clown_gives_a_free_reroll_while_held() {
    let mut run = small();
    run.add_joker(J::ChaosTheClown);
    assert_eq!((run.free_rerolls, run.reroll_cost), (1, 0));
    run.sell_joker(0).unwrap();
    assert_eq!(run.free_rerolls, 0);
    assert!(run.reroll_cost > 0);
}

#[test]
fn astronomer_makes_planets_free() {
    let mut run = small();
    run.give_consumable(ConsumableKind::Pluto);
    assert_eq!(run.consumables[0].sell_cost, 1);
    run.add_joker(J::Astronomer);
    assert_eq!(run.consumables[0].cost, 0);
    run.sell_joker(0).unwrap();
    assert_eq!(run.consumables[0].cost, 3);
}

#[test]
fn top_up_tag_fills_two_joker_slots_with_commons() {
    let mut run = Run::new(RunConfig::new(SEED));
    run.tags.push(HeldTag {
        kind: TagKind::TopUp,
        orbital_hand: None,
    });
    run.skip_blind().unwrap();
    assert_eq!(run.jokers.len(), 2);
    assert!(run.jokers.iter().all(|j| j.rarity() == 1));
    assert!(run.tags.iter().all(|t| t.kind != TagKind::TopUp));
}

#[test]
fn every_joker_is_implemented() {
    for j in bm_core::joker_data::JOKERS.iter() {
        assert!(bm_core::jokers::is_implemented(j.kind), "{:?}", j.kind);
    }
}

// ------------------------------------------- blind-start order (review)

/// A run at blind select with the boss on deck.
fn at_boss_select(boss: Boss, jokers: &[J]) -> Run {
    use bm_core::blinds::SlotState;
    let mut run = at_blind_select(jokers);
    run.boss = boss;
    run.slots = [SlotState::Defeated, SlotState::Defeated, SlotState::Select];
    run
}

#[test]
fn a_chicot_eaten_by_a_dagger_does_not_disable_the_boss() {
    let mut run = at_boss_select(Boss::Wall, &[J::CeremonialDagger, J::Chicot]);
    run.select_blind().unwrap();
    assert_eq!(run.jokers.len(), 1);
    assert!(!run.blind.as_ref().unwrap().disabled);
    // Chicot alone does.
    let mut run = at_boss_select(Boss::Wall, &[J::Chicot]);
    run.select_blind().unwrap();
    assert!(run.blind.as_ref().unwrap().disabled);
}

#[test]
fn chicot_disables_after_burglar_so_the_water_gives_discards_back() {
    let mut run = at_boss_select(Boss::Water, &[J::Chicot, J::Burglar]);
    run.select_blind().unwrap();
    // The Water takes 4, Burglar sets 0 and adds 3 hands, then Chicot's
    // queued disable gives the 4 back.
    assert_eq!((run.hands_left, run.discards_left), (7, 4));
}

#[test]
fn riff_raff_jokers_arrive_after_the_other_blind_start_effects() {
    let mut run = at_blind_select(&[J::RiffRaff, J::CeremonialDagger]);
    run.select_blind().unwrap();
    // The Dagger had no right neighbour when it acted: nothing eaten.
    assert_eq!(run.jokers.len(), 4);
    assert_eq!(joker(&run, J::CeremonialDagger).mult, 0.0);
}

#[test]
fn amber_acorn_shuffles_after_the_dagger_chose_its_victim() {
    let mut run = at_boss_select(
        Boss::AmberAcorn,
        &[J::CeremonialDagger, J::Joker, J::GreedyJoker, J::LustyJoker],
    );
    run.select_blind().unwrap();
    // The victim is the Joker that was right of the Dagger before the
    // shuffles, whatever order the shuffles leave.
    assert_eq!(run.jokers.len(), 3);
    assert!(run.jokers.iter().all(|j| j.kind != J::Joker));
    assert_eq!(joker(&run, J::CeremonialDagger).mult, 2.0);
}

#[test]
fn discards_never_go_below_zero_when_a_discard_joker_leaves() {
    let mut run = with_jokers(&[J::Drunkard]);
    assert_eq!(run.discards_left, 5);
    run.discards_left = 0;
    run.sell_joker(0).unwrap();
    assert_eq!(run.discards_left, 0);
    assert_eq!(run.discards_per_round, 4);
}

#[test]
fn a_second_vampire_skips_cards_the_first_one_stripped() {
    // [Vampire, Midas Mask, Vampire]: the first strips both Steel Kings,
    // Midas Mask makes them Gold, the second Vampire leaves them Gold.
    let mut run = with_jokers(&[J::Vampire, J::MidasMask, J::Vampire]);
    let mut kings = Vec::new();
    for s in [Spades, Hearts] {
        kings.push(run.add_special_card_to_hand(
            s,
            King,
            Enhancement::Steel,
            Edition::None,
            Seal::None,
        ));
    }
    play(&mut run, &[0, 1]);
    for k in kings {
        assert_eq!(run.card(k).enhancement, Enhancement::Gold);
    }
    assert_eq!(run.jokers[0].x_mult, 1.2);
    assert_eq!(run.jokers[2].x_mult, 1.0);
}

#[test]
fn losing_the_ante_8_boss_is_not_a_win() {
    let mut run = Run::scenario(SEED, BlindKind::Boss(Boss::Arm));
    run.ante = 8;
    run.hands_left = 1;
    pair_of_kings(&mut run);
    play(&mut run, &[0, 1]);
    assert_eq!(run.phase, Phase::GameOver);
    assert!(!run.won);

    let mut run = Run::scenario(SEED, BlindKind::Boss(Boss::Arm));
    run.ante = 8;
    win_round(&mut run);
    assert!(run.won);
}

#[test]
fn the_pillar_mark_is_set_once_the_hand_is_scored() {
    // The real game's checks (7 November): a card Midas Mask turns Gold while it is played keeps the "played this
    // ante" mark, and a DNA copy made while it is played is born without it.
    let mut run = with_jokers(&[J::MidasMask]);
    pair_of_kings(&mut run);
    let kings: Vec<_> = run.hand[run.hand.len() - 2..].to_vec();
    let pos: Vec<usize> = (0..run.hand.len())
        .filter(|&i| kings.contains(&run.hand[i]))
        .collect();
    play(&mut run, &pos);
    for k in kings {
        assert_eq!(run.card(k).enhancement, Enhancement::Gold);
        assert!(
            run.card(k).played_this_ante,
            "turned Gold while played, and still marked"
        );
    }
    let mut run = with_jokers(&[J::DNA]);
    run.add_card_to_hand(Clubs, King);
    let king = *run.hand.last().unwrap();
    let before = run.playing_cards.len();
    let pos = run.hand.len() - 1;
    play(&mut run, &[pos]);
    assert_eq!(run.playing_cards.len(), before + 1, "DNA copied the card");
    let copy = *run.playing_cards.last().unwrap();
    assert!(run.card(king).played_this_ante);
    assert!(
        !run.card(copy).played_this_ante,
        "the copy was made before the mark"
    );
}
