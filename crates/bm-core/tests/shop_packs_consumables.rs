//! Checks for the shop, booster packs, vouchers and consumables. Expected
//! values come from `rules/shop.md`, `rules/consumables.md` and
//! `rules/vouchers_tags.md`; checks that depend on what a seed draws only
//! test the shape (counts, kinds, where things go) or that the same seed
//! gives the same result.

use bm_core::blinds::BlindKind;
use bm_core::card::{Edition, Enhancement, Rank, Seal, Suit};
use bm_core::content::{ConsumableKind as C, ConsumableSet, PackKind, VoucherKind, BOOSTERS};
use bm_core::joker_data::JokerKind as J;
use bm_core::packs::PackCard;
use bm_core::poker::HandType;
use bm_core::shop::RowItem;
use bm_core::state::HeldTag;
use bm_core::tags::TagKind;
use bm_core::{Phase, Run, RunConfig};

use Rank::*;
use Suit::*;

const SEED: &str = "SHOPSEED";

/// Win the first Small Blind with one card and go to the shop.
fn to_shop(seed: &str) -> Run {
    let mut run = Run::new(RunConfig::new(seed));
    run.select_blind().unwrap();
    run.blind.as_mut().unwrap().chips = 1.0;
    run.play(&[0]).unwrap();
    run.cash_out().unwrap();
    assert_eq!(run.phase, Phase::Shop);
    run
}

fn round() -> Run {
    Run::scenario(SEED, BlindKind::Small)
}

/// Use the consumable in slot 0 on the hand cards at `pos`.
fn use_on(run: &mut Run, kind: C, pos: &[usize]) {
    run.consumables.clear();
    run.give_consumable(kind);
    run.use_consumable(0, pos).unwrap();
}

// ---------------------------------------------------------------- shop

#[test]
fn first_shop_shape() {
    let run = to_shop(SEED);
    let shop = run.shop.as_ref().unwrap();
    assert_eq!(shop.row.len(), 2);
    assert_eq!(shop.vouchers.len(), 1);
    assert_eq!(Some(shop.vouchers[0].voucher), run.shop_voucher);
    assert_eq!(shop.vouchers[0].cost, 10);
    assert_eq!(shop.boosters.len(), 2);
    // The run's first shop pack is a normal Buffoon Pack.
    let first = &BOOSTERS[shop.boosters[0].as_ref().unwrap().booster as usize];
    assert_eq!(
        (first.kind, first.size, first.choose),
        (PackKind::Buffoon, 2, 1)
    );
    assert_eq!(shop.boosters[0].as_ref().unwrap().cost, 4);
    assert_eq!(run.reroll_cost, 5);
}

#[test]
fn the_same_seed_gives_the_same_shop() {
    let a = to_shop("SAMESEED");
    let b = to_shop("SAMESEED");
    assert_eq!(format!("{:?}", a.shop), format!("{:?}", b.shop));
    let c = to_shop("OTHRSEED");
    assert_ne!(format!("{:?}", a.shop), format!("{:?}", c.shop));
}

#[test]
fn rerolls_cost_five_then_one_more_each() {
    let mut run = to_shop(SEED);
    run.money = 100;
    run.reroll_shop().unwrap();
    assert_eq!((run.money, run.reroll_cost), (95, 6));
    run.reroll_shop().unwrap();
    assert_eq!((run.money, run.reroll_cost), (89, 7));
    assert_eq!(run.shop.as_ref().unwrap().row.len(), 2);
}

#[test]
fn chaos_the_clown_bought_mid_shop_gives_a_free_reroll() {
    let mut run = to_shop(SEED);
    run.money = 100;
    run.add_joker(J::ChaosTheClown);
    assert_eq!(run.reroll_cost, 0);
    run.reroll_shop().unwrap();
    assert_eq!((run.money, run.reroll_cost), (100, 5));
}

#[test]
fn prices_follow_the_table() {
    let run = round();
    // (base, 0%, 25%, 50%) from `rules/shop.md` 5.1.
    let table = [
        (1, 1, 1, 1),
        (2, 2, 1, 1),
        (3, 3, 2, 1),
        (4, 4, 3, 2),
        (5, 5, 4, 2),
        (6, 6, 4, 3),
        (7, 7, 5, 3),
        (8, 8, 6, 4),
        (9, 9, 7, 4),
        (10, 10, 7, 5),
        (11, 11, 8, 5),
        (12, 12, 9, 6),
        (13, 13, 10, 6),
        (15, 15, 11, 7),
        (20, 20, 15, 10),
        (25, 25, 19, 12),
    ];
    for (b, p0, p25, p50) in table {
        let mut r = run.clone();
        assert_eq!(r.price(b, Edition::None), p0, "base {b}");
        r.discount_percent = 25;
        assert_eq!(r.price(b, Edition::None), p25, "base {b} at 25%");
        r.discount_percent = 50;
        assert_eq!(r.price(b, Edition::None), p50, "base {b} at 50%");
    }
    // Edition surcharges: foil +2, holographic +3, polychrome and negative +5.
    assert_eq!(run.price(6, Edition::Foil), 8);
    assert_eq!(run.price(6, Edition::Holographic), 9);
    assert_eq!(run.price(6, Edition::Polychrome), 11);
    assert_eq!(run.price(6, Edition::Negative), 11);
}

#[test]
fn buying_and_selling_a_joker() {
    let mut run = to_shop(SEED);
    run.money = 200;
    // Reroll until the row offers a joker.
    let mut tries = 0;
    let i = loop {
        let row = &run.shop.as_ref().unwrap().row;
        if let Some(i) = row.iter().position(|r| matches!(r.item, RowItem::Joker(_))) {
            break i;
        }
        run.reroll_shop().unwrap();
        tries += 1;
        assert!(tries < 20);
    };
    let slot = run.shop.as_ref().unwrap().row[i].clone();
    let before = run.money;
    run.buy(i).unwrap();
    assert_eq!(run.money, before - slot.cost);
    assert_eq!(run.jokers.len(), 1);
    let sell = (run.jokers[0].cost / 2).max(1);
    assert_eq!(run.jokers[0].sell_cost, sell);
    let before = run.money;
    run.sell_joker(0).unwrap();
    assert_eq!(run.money, before + sell);
}

#[test]
fn a_full_joker_row_refuses_to_buy_without_changing_anything() {
    let mut run = to_shop(SEED);
    run.money = 200;
    for _ in 0..5 {
        run.add_joker(J::Joker);
    }
    let i = loop {
        let row = &run.shop.as_ref().unwrap().row;
        if let Some(i) = row
            .iter()
            .position(|r| matches!(&r.item, RowItem::Joker(j) if j.edition != Edition::Negative))
        {
            break i;
        }
        run.reroll_shop().unwrap();
    };
    let money = run.money;
    assert!(run.buy(i).is_err());
    assert_eq!((run.money, run.jokers.len()), (money, 5));
}

// ------------------------------------------------------------ vouchers

#[test]
fn redeeming_the_shop_voucher() {
    let mut run = to_shop(SEED);
    run.money = 50;
    let v = run.shop.as_ref().unwrap().vouchers[0].voucher;
    run.redeem_voucher(0).unwrap();
    assert!(run.vouchers[v.index()]);
    assert_eq!(run.money, 40);
    assert!(run.shop.as_ref().unwrap().vouchers.is_empty());
    assert_eq!(run.shop_voucher, None);
}

#[test]
fn voucher_effects() {
    use VoucherKind as V;
    let mut run = to_shop(SEED);
    run.redeem(V::Overstock);
    assert_eq!(run.joker_max, 3);
    assert_eq!(run.shop.as_ref().unwrap().row.len(), 3);

    let mut run = round();
    run.redeem(V::CrystalBall);
    run.redeem(V::Grabber);
    run.redeem(V::Wasteful);
    run.redeem(V::PaintBrush);
    run.redeem(V::SeedMoney);
    assert_eq!(run.consumable_slots, 3);
    assert_eq!((run.hands_per_round, run.hands_left), (5, 5));
    assert_eq!((run.discards_per_round, run.discards_left), (5, 5));
    assert_eq!(run.hand_size(), 9);
    assert_eq!(run.interest_cap, 50);

    let mut run = round();
    run.redeem(V::RerollSurplus);
    assert_eq!((run.reroll_base, run.reroll_cost), (3, 3));
    run.redeem(V::Hieroglyph);
    assert_eq!((run.ante, run.hands_per_round), (0, 3));

    // Clearance Sale re-prices owned cards: a $6 joker sells for $3, then $2.
    let mut run = round();
    run.add_joker(J::Blackboard);
    assert_eq!(run.jokers[0].sell_cost, 3);
    run.redeem(V::ClearanceSale);
    assert_eq!((run.jokers[0].cost, run.jokers[0].sell_cost), (4, 2));
}

#[test]
fn seed_money_raises_interest() {
    let mut run = round();
    run.redeem(VoucherKind::SeedMoney);
    run.money = 50;
    run.blind.as_mut().unwrap().chips = 1.0;
    run.add_card_to_hand(Spades, King);
    run.play(&[0]).unwrap();
    assert_eq!(run.cash_out.as_ref().unwrap().interest, 10);
}

// --------------------------------------------------------------- packs

/// Open a tag's pack at blind select (skipping the Small Blind for any tag).
fn tag_pack(tag: TagKind) -> Run {
    let mut run = Run::new(RunConfig::new(SEED));
    run.tags.push(HeldTag {
        kind: tag,
        orbital_hand: None,
    });
    run.skip_blind().unwrap();
    assert_eq!(run.phase, Phase::Pack);
    run
}

#[test]
fn pack_tags_open_their_mega_packs() {
    let run = tag_pack(TagKind::Charm);
    let p = run.pack.as_ref().unwrap();
    assert_eq!(
        (p.kind(), p.cards.len(), p.choices_left),
        (PackKind::Arcana, 5, 2)
    );
    // Arcana packs deal a hand.
    assert_eq!(run.hand.len(), 8);
    assert!(p.cards.iter().all(|c| matches!(c, PackCard::Consumable(k)
        if k.kind.set() == ConsumableSet::Tarot || k.kind == C::Soul)));

    let run = tag_pack(TagKind::Meteor);
    let p = run.pack.as_ref().unwrap();
    assert_eq!(
        (p.kind(), p.cards.len(), p.choices_left),
        (PackKind::Celestial, 5, 2)
    );
    assert!(run.hand.is_empty());

    let run = tag_pack(TagKind::Buffoon);
    let p = run.pack.as_ref().unwrap();
    assert_eq!(
        (p.kind(), p.cards.len(), p.choices_left),
        (PackKind::Buffoon, 4, 2)
    );

    let run = tag_pack(TagKind::Ethereal);
    let p = run.pack.as_ref().unwrap();
    assert_eq!(
        (p.kind(), p.cards.len(), p.choices_left),
        (PackKind::Spectral, 2, 1)
    );
    assert_eq!(run.hand.len(), 8);
}

#[test]
fn standard_pack_cards_join_the_deck() {
    let mut run = tag_pack(TagKind::Standard);
    let n = run.playing_cards.len();
    run.pack_choose(0, &[]).unwrap();
    run.pack_choose(0, &[]).unwrap();
    assert_eq!(run.playing_cards.len(), n + 2);
    // Two picks used up: the pack closes and blind select comes back.
    assert_eq!(run.phase, Phase::BlindSelect);
    assert!(run.pack.is_none());
}

#[test]
fn skipping_a_pack_returns_its_hand_and_feeds_red_card() {
    let mut run = Run::new(RunConfig::new(SEED));
    run.add_joker(J::RedCard);
    run.tags.push(HeldTag {
        kind: TagKind::Charm,
        orbital_hand: None,
    });
    run.skip_blind().unwrap();
    assert_eq!(run.hand.len(), 8);
    run.pack_skip().unwrap();
    assert_eq!(run.phase, Phase::BlindSelect);
    assert!(run.hand.is_empty());
    assert_eq!(run.deck.len(), 52);
    assert_eq!(run.jokers[0].mult, 3.0);
}

#[test]
fn celestial_pack_planets_level_hands() {
    let mut run = tag_pack(TagKind::Meteor);
    let PackCard::Consumable(c) = run.pack.as_ref().unwrap().cards[0].clone() else {
        panic!("a planet");
    };
    let h = c.kind.planet_hand();
    run.pack_choose(0, &[]).unwrap();
    if let Some(h) = h {
        assert_eq!(run.hand_levels.get(h).level, 2);
    }
}

// --------------------------------------------------------- consumables

#[test]
fn enhancing_tarots() {
    let cases = [
        (C::Magician, 2, Enhancement::Lucky),
        (C::Empress, 2, Enhancement::Mult),
        (C::Hierophant, 2, Enhancement::Bonus),
        (C::Lovers, 1, Enhancement::Wild),
        (C::Chariot, 1, Enhancement::Steel),
        (C::Justice, 1, Enhancement::Glass),
        (C::Devil, 1, Enhancement::Gold),
        (C::Tower, 1, Enhancement::Stone),
    ];
    for (kind, n, e) in cases {
        let mut run = round();
        let a = run.add_card_to_hand(Spades, King);
        let b = run.add_card_to_hand(Hearts, Two);
        let pos: Vec<usize> = (0..n).collect();
        use_on(&mut run, kind, &pos);
        assert_eq!(run.card(a).enhancement, e, "{kind:?}");
        let want_b = if n == 2 { e } else { Enhancement::None };
        assert_eq!(run.card(b).enhancement, want_b, "{kind:?}");
    }
    // Too many cards: refused.
    let mut run = round();
    run.add_card_to_hand(Spades, King);
    run.add_card_to_hand(Hearts, Two);
    run.give_consumable(C::Lovers);
    assert!(run.use_consumable(0, &[0, 1]).is_err());
    assert_eq!(run.consumables.len(), 1);
}

#[test]
fn rank_and_suit_tarots() {
    let mut run = round();
    let k = run.add_card_to_hand(Spades, King);
    let a = run.add_card_to_hand(Hearts, Ace);
    use_on(&mut run, C::Strength, &[0, 1]);
    assert_eq!(run.card(k).rank, Ace);
    assert_eq!(run.card(a).rank, Two);
    assert_eq!(run.card(a).suit, Hearts);

    for (kind, suit) in [
        (C::Star, Diamonds),
        (C::Moon, Clubs),
        (C::Sun, Hearts),
        (C::World, Spades),
    ] {
        let mut run = round();
        let ids: Vec<_> = [
            (Spades, Two),
            (Hearts, Three),
            (Clubs, Four),
            (Diamonds, Five),
        ]
        .into_iter()
        .map(|(s, r)| run.add_card_to_hand(s, r))
        .collect();
        use_on(&mut run, kind, &[0, 1, 2]);
        for &id in &ids[..3] {
            assert_eq!(run.card(id).suit, suit, "{kind:?}");
        }
        assert_eq!(run.card(ids[3]).suit, Diamonds);
    }
}

#[test]
fn hermit_temperance_and_hanged_man() {
    let mut run = round();
    run.money = 10;
    use_on(&mut run, C::Hermit, &[]);
    assert_eq!(run.money, 20);
    run.money = 30;
    use_on(&mut run, C::Hermit, &[]);
    assert_eq!(run.money, 50);

    // Joker ($2) sells for $1, Blackboard ($6) for $3.
    let mut run = round();
    run.add_joker(J::Joker);
    run.add_joker(J::Blackboard);
    run.money = 0;
    use_on(&mut run, C::Temperance, &[]);
    assert_eq!(run.money, 1 + 3);

    let mut run = round();
    let a = run.add_card_to_hand(Spades, King);
    let b = run.add_card_to_hand(Hearts, Two);
    run.add_card_to_hand(Clubs, Three);
    use_on(&mut run, C::HangedMan, &[0, 1]);
    assert!(!run.playing_cards.contains(&a) && !run.playing_cards.contains(&b));
    assert_eq!(run.hand.len(), 1);
}

#[test]
fn death_turns_the_left_card_into_the_right_one() {
    let mut run = round();
    let left = run.add_card_to_hand(Spades, Two);
    let right =
        run.add_special_card_to_hand(Hearts, King, Enhancement::Gold, Edition::Foil, Seal::Red);
    let sort_id = run.card(left).sort_id;
    use_on(&mut run, C::Death, &[0, 1]);
    let c = run.card(left);
    assert_eq!((c.suit, c.rank), (Hearts, King));
    assert_eq!(
        (c.enhancement, c.edition, c.seal),
        (Enhancement::Gold, Edition::Foil, Seal::Red)
    );
    assert_eq!(c.sort_id, sort_id);
    assert_eq!(run.card(right).rank, King);
}

#[test]
fn creating_tarots() {
    let mut run = round();
    use_on(&mut run, C::HighPriestess, &[]);
    assert_eq!(run.consumables.len(), 2);
    assert!(run
        .consumables
        .iter()
        .all(|c| c.kind.set() == ConsumableSet::Planet));

    let mut run = round();
    use_on(&mut run, C::Emperor, &[]);
    assert_eq!(run.consumables.len(), 2);
    assert!(run
        .consumables
        .iter()
        .all(|c| c.kind.set() == ConsumableSet::Tarot && c.kind != C::Emperor));

    // The Fool makes the last Tarot or Planet used.
    let mut run = round();
    use_on(&mut run, C::Mercury, &[]);
    use_on(&mut run, C::Fool, &[]);
    assert_eq!(run.consumables.len(), 1);
    assert_eq!(run.consumables[0].kind, C::Mercury);
    // A Fool after a Fool can't be used.
    run.consumables.clear();
    run.give_consumable(C::Fool);
    assert!(run.use_consumable(0, &[]).is_err());

    let mut run = round();
    use_on(&mut run, C::Judgement, &[]);
    assert_eq!(run.jokers.len(), 1);
}

#[test]
fn planets_and_black_hole_level_hands() {
    let mut run = round();
    use_on(&mut run, C::Pluto, &[]);
    let hc = run.hand_levels.get(HandType::HighCard);
    assert_eq!((hc.level, hc.chips, hc.mult), (2, 15.0, 2.0));
    use_on(&mut run, C::Mercury, &[]);
    let pair = run.hand_levels.get(HandType::Pair);
    assert_eq!((pair.level, pair.chips, pair.mult), (2, 25.0, 3.0));
    assert_eq!(run.usage_total.planet, 2);

    let mut run = round();
    use_on(&mut run, C::BlackHole, &[]);
    for h in HandType::ALL {
        assert_eq!(run.hand_levels.get(h).level, 2, "{h:?}");
    }
    assert_eq!(run.usage_total.planet, 0);
}

#[test]
fn seal_spectrals_and_aura() {
    for (kind, seal) in [
        (C::Talisman, Seal::Gold),
        (C::DejaVu, Seal::Red),
        (C::Trance, Seal::Blue),
        (C::Medium, Seal::Purple),
    ] {
        let mut run = round();
        let a = run.add_card_to_hand(Spades, King);
        use_on(&mut run, kind, &[0]);
        assert_eq!(run.card(a).seal, seal, "{kind:?}");
    }
    let mut run = round();
    let a = run.add_card_to_hand(Spades, King);
    use_on(&mut run, C::Aura, &[0]);
    assert!(matches!(
        run.card(a).edition,
        Edition::Foil | Edition::Holographic | Edition::Polychrome
    ));
}

#[test]
fn spectrals_that_destroy_and_make_cards() {
    let fill = |run: &mut Run| {
        for (s, r) in [
            (Spades, Two),
            (Hearts, Three),
            (Clubs, Four),
            (Diamonds, Five),
        ] {
            run.add_card_to_hand(s, r);
        }
    };
    // Familiar: one destroyed, three face cards made (no Stone).
    let mut run = round();
    fill(&mut run);
    let n = run.playing_cards.len();
    use_on(&mut run, C::Familiar, &[]);
    assert_eq!(run.playing_cards.len(), n - 1 + 3);
    let made = &run.playing_cards[run.playing_cards.len() - 3..];
    for &id in made {
        let c = run.card(id);
        assert!(matches!(c.rank, Jack | Queen | King));
        assert!(c.enhancement != Enhancement::None && c.enhancement != Enhancement::Stone);
    }
    // Grim: two Aces. Incantation: four number cards.
    let mut run = round();
    fill(&mut run);
    let n = run.playing_cards.len();
    use_on(&mut run, C::Grim, &[]);
    assert_eq!(run.playing_cards.len(), n + 1);
    let mut run = round();
    fill(&mut run);
    let n = run.playing_cards.len();
    use_on(&mut run, C::Incantation, &[]);
    assert_eq!(run.playing_cards.len(), n + 3);
    let made = &run.playing_cards[run.playing_cards.len() - 4..];
    assert!(made.iter().all(|&id| run.card(id).rank.id() <= 10));
    // Immolate: up to 5 destroyed, +$20.
    let mut run = round();
    fill(&mut run);
    run.money = 0;
    let n = run.playing_cards.len();
    use_on(&mut run, C::Immolate, &[]);
    assert_eq!(run.playing_cards.len(), n - 4);
    assert_eq!(run.money, 20);
    // Cryptid: two copies into the hand.
    let mut run = round();
    fill(&mut run);
    let n = run.playing_cards.len();
    use_on(&mut run, C::Cryptid, &[0]);
    assert_eq!(run.playing_cards.len(), n + 2);
    assert_eq!(run.hand.len(), 6);
}

#[test]
fn sigil_and_ouija() {
    let mut run = round();
    for (s, r) in [(Spades, Two), (Hearts, Three), (Clubs, Four)] {
        run.add_card_to_hand(s, r);
    }
    use_on(&mut run, C::Sigil, &[]);
    let s0 = run.card(run.hand[0]).suit;
    assert!(run.hand.iter().all(|&id| run.card(id).suit == s0));

    let mut run = round();
    for (s, r) in [(Spades, Two), (Hearts, Three), (Clubs, Four)] {
        run.add_card_to_hand(s, r);
    }
    use_on(&mut run, C::Ouija, &[]);
    let r0 = run.card(run.hand[0]).rank;
    assert!(run.hand.iter().all(|&id| run.card(id).rank == r0));
    assert_eq!(run.hand_size(), 7);
}

#[test]
fn spectrals_that_change_jokers() {
    // Wraith: a Rare joker, money to $0.
    let mut run = round();
    run.money = 37;
    use_on(&mut run, C::Wraith, &[]);
    assert_eq!(run.jokers.len(), 1);
    assert_eq!(run.jokers[0].rarity(), 3);
    assert_eq!(run.money, 0);
    // The Soul: a legendary.
    let mut run = round();
    use_on(&mut run, C::Soul, &[]);
    assert_eq!(run.jokers[0].rarity(), 4);
    // Ectoplasm: a joker turns Negative (+1 slot), hand size -1.
    let mut run = round();
    run.add_joker(J::Joker);
    use_on(&mut run, C::Ectoplasm, &[]);
    assert_eq!(run.jokers[0].edition, Edition::Negative);
    assert_eq!((run.joker_slots, run.hand_size()), (6, 7));
    // Hex: one joker Polychrome, the others (not Eternal) destroyed.
    let mut run = round();
    run.add_joker(J::Joker);
    run.add_joker(J::Banner);
    run.add_joker(J::Hack);
    run.jokers[2].eternal = true;
    use_on(&mut run, C::Hex, &[]);
    assert!(run.jokers.iter().any(|j| j.edition == Edition::Polychrome));
    assert!(run.jokers.len() <= 2);
    assert!(run.jokers.iter().any(|j| j.kind == J::Hack));
    // Ankh: one joker and its copy remain (plus Eternal ones).
    let mut run = round();
    run.add_joker(J::Joker);
    run.add_joker(J::Banner);
    use_on(&mut run, C::Ankh, &[]);
    assert_eq!(run.jokers.len(), 2);
    assert_eq!(run.jokers[0].kind, run.jokers[1].kind);
}

#[test]
fn wheel_of_fortune_gives_an_edition_on_a_hit() {
    let mut run = round();
    run.add_joker(J::Joker);
    run.probability = 4.0; // every roll hits
    use_on(&mut run, C::WheelOfFortune, &[]);
    assert!(matches!(
        run.jokers[0].edition,
        Edition::Foil | Edition::Holographic | Edition::Polychrome
    ));
}

#[test]
fn selling_a_consumable_pays_its_sell_value() {
    let mut run = round();
    run.give_consumable(C::Pluto);
    run.give_consumable(C::Familiar);
    let before = run.money;
    run.sell_consumable(0).unwrap();
    run.sell_consumable(0).unwrap();
    // Planets cost $3 (sell $1), Spectrals $4 (sell $2).
    assert_eq!(run.money, before + 1 + 2);
}
