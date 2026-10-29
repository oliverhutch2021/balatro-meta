//! Joker reactions outside the scoring loops: blind start, the shop, packs,
//! sales, consumables, cards added or destroyed (`rules/jokers_1_76.md`,
//! `rules/jokers_77_150.md`). Each follows the game's routine for that
//! context; "not copyable" effects check `bp` (a Blueprint/Brainstorm copy).

use bm_rng::pick_index;

use crate::card::{standard_card_order, Edition, Enhancement, Seal};
use crate::content::{ConsumableKind, ConsumableSet};
use crate::create::{Center, CreateArgs, CreateKind, Dest};
use crate::joker_data::JokerKind as J;
use crate::scoring::{calculate_joker, Ctx, Effect, PlayedHand};
use crate::state::{CardId, HeldTag, Run};
use crate::tags::TagKind;

fn some() -> Option<Effect> {
    Some(Effect::default())
}

impl Run {
    /// Ask every joker, left to right, about a context with no cards.
    pub(crate) fn joker_event_all(&mut self, ctx: Ctx) {
        let h = PlayedHand::default();
        let mut i = 0;
        while i < self.jokers.len() {
            calculate_joker(self, i, ctx, &h, 0);
            i += 1;
        }
    }

    /// Playing cards joined the deck (Hologram).
    pub(crate) fn playing_card_added(&mut self, ids: &[CardId]) {
        let h = PlayedHand {
            full: ids.to_vec(),
            ..PlayedHand::default()
        };
        for i in 0..self.jokers.len() {
            calculate_joker(self, i, Ctx::PlayingCardAdded, &h, 0);
        }
    }

    /// Room for one more consumable.
    fn consumable_room_now(&self) -> bool {
        self.consumables.len() < self.consumable_slots as usize
    }
}

/// Blind-start effects (`setting_blind`) other than Chicot's.
pub(crate) fn setting_blind(
    run: &mut Run,
    idx: usize,
    _h: &PlayedHand,
    bp: bool,
) -> Option<Effect> {
    let kind = run.jokers[idx].kind;
    if run.jokers[idx].sliced {
        return None;
    }
    let ante = run.ante;
    match kind {
        J::CeremonialDagger if !bp => {
            let right = idx + 1;
            if right < run.jokers.len() && !run.jokers[right].eternal && !run.jokers[right].sliced {
                run.jokers[right].sliced = true;
                run.joker_buffer -= 1;
                let value = run.jokers[right].sell_cost;
                run.jokers[idx].mult += 2.0 * value as f64;
            }
            None
        }
        J::MarbleJoker => {
            let order = standard_card_order();
            let mut g = run.rng.generator("marb_fr");
            let (suit, rank) = order[pick_index(order.len(), &mut g)];
            let id = run.new_card(suit, rank);
            run.card_mut(id).enhancement = Enhancement::Stone;
            run.playing_cards.push(id);
            run.deck.insert(0, id);
            run.playing_card_added(&[id]);
            some()
        }
        J::Burglar => {
            run.discards_left = 0;
            run.hands_left += 3;
            some()
        }
        J::Madness if !bp => {
            let boss = run.blind.as_ref().is_some_and(|b| b.boss().is_some());
            if boss {
                return None;
            }
            run.jokers[idx].x_mult += 0.5;
            let mut cands: Vec<usize> = (0..run.jokers.len())
                .filter(|&i| i != idx && !run.jokers[i].eternal && !run.jokers[i].sliced)
                .collect();
            let jk = &run.jokers;
            cands.sort_by_key(|&i| jk[i].sort_id);
            if !cands.is_empty() {
                let mut g = run.rng.generator("madness");
                let v = cands[pick_index(cands.len(), &mut g)];
                run.jokers[v].sliced = true;
            }
            None
        }
        J::RiffRaff => {
            let used = run.jokers.len() as i32 + run.joker_buffer;
            if used < run.joker_slots {
                let n = (run.joker_slots - used).min(2);
                run.joker_buffer += n;
                for _ in 0..n {
                    let mut a = CreateArgs::new(CreateKind::Joker, Dest::Jokers, "rif");
                    a.rarity = Some(0.0);
                    run.create_joker_into_slots(a);
                }
                run.joker_buffer -= n;
                return some();
            }
            None
        }
        J::Cartomancer => {
            if run.consumable_room_now() {
                run.create_consumable_into_slots(CreateKind::Tarot, "car");
                return some();
            }
            None
        }
        _ => {
            let _ = ante;
            None
        }
    }
}

/// The other non-scoring contexts.
pub(crate) fn event(
    run: &mut Run,
    idx: usize,
    ctx: Ctx,
    h: &PlayedHand,
    bp: bool,
) -> Option<Effect> {
    let kind = run.jokers[idx].kind;
    let ante = run.ante;
    match ctx {
        Ctx::OpenBooster => {
            if kind == J::Hallucination
                && run.consumable_room_now()
                && run.rng.random(&format!("halu{ante}")) < run.probability / 2.0
            {
                return Some(Effect {
                    make: true,
                    ..Effect::default()
                });
            }
            None
        }
        Ctx::SkippingBooster => {
            if kind == J::RedCard && !bp {
                run.jokers[idx].mult += 3.0;
                return some();
            }
            None
        }
        Ctx::RerollShop => {
            if kind == J::FlashCard && !bp {
                run.jokers[idx].mult += 2.0;
                return some();
            }
            None
        }
        Ctx::SellingCard => {
            if kind == J::Campfire && !bp {
                run.jokers[idx].x_mult += 0.25;
                return some();
            }
            None
        }
        Ctx::SellingSelf => match kind {
            J::Luchador => {
                if run
                    .blind
                    .as_ref()
                    .is_some_and(|b| b.boss().is_some() && !b.disabled)
                {
                    run.disable_blind();
                }
                some()
            }
            J::DietCola => {
                run.add_tag_pub(HeldTag {
                    kind: TagKind::Double,
                    orbital_hand: None,
                });
                some()
            }
            J::InvisibleJoker if !bp && run.jokers[idx].counter >= 2 => {
                let mut cands: Vec<usize> = (0..run.jokers.len()).filter(|&i| i != idx).collect();
                if cands.is_empty() || run.jokers.len() as i32 > run.joker_slots {
                    return None;
                }
                let jk = &run.jokers;
                cands.sort_by_key(|&i| jk[i].sort_id);
                let mut g = run.rng.generator("invisible");
                let orig = run.jokers[cands[pick_index(cands.len(), &mut g)]].clone();
                if orig.kind == J::ToDoList {
                    let _ = run.pick_to_do_hand(None);
                }
                let mut copy = orig;
                copy.sort_id = run.take_sort_id();
                copy.added = false;
                if copy.edition == Edition::Negative {
                    copy.edition = Edition::None;
                }
                if copy.kind == J::InvisibleJoker {
                    copy.counter = 0;
                }
                run.set_joker_cost(&mut copy);
                run.mark_present(Center::Joker(copy.kind));
                run.push_joker(copy);
                some()
            }
            _ => None,
        },
        Ctx::EndingShop => {
            if kind == J::Perkeo && !run.consumables.is_empty() {
                let mut ids: Vec<usize> = (0..run.consumables.len()).collect();
                let cs = &run.consumables;
                ids.sort_by_key(|&i| cs[i].sort_id);
                let mut g = run.rng.generator("perkeo");
                let mut copy = run.consumables[ids[pick_index(ids.len(), &mut g)]].clone();
                copy.sort_id = run.take_sort_id();
                copy.edition = Edition::Negative;
                run.set_consumable_cost(&mut copy);
                run.mark_present(Center::Consumable(copy.kind));
                run.consumables.push(copy);
                run.consumable_slots += 1;
                return some();
            }
            None
        }
        Ctx::PlayingCardAdded => {
            if kind == J::Hologram && !bp && !run.jokers[idx].sliced && !h.full.is_empty() {
                run.jokers[idx].x_mult += 0.25 * h.full.len() as f64;
                return some();
            }
            None
        }
        Ctx::UsingConsumeable(ck, glass) => {
            if kind == J::Constellation && !bp && ck.set() == ConsumableSet::Planet {
                run.jokers[idx].x_mult += 0.1;
                return some();
            }
            if kind == J::GlassJoker && !bp && ck == ConsumableKind::HangedMan && glass > 0 {
                run.jokers[idx].x_mult += 0.75 * glass as f64;
                return some();
            }
            if kind == J::FortuneTeller && !bp && ck.set() == ConsumableSet::Tarot {
                return some();
            }
            None
        }
        Ctx::RemovePlayingCards => {
            if kind == J::Caino && !bp {
                let par = run.pareidolia();
                let faces = h
                    .full
                    .iter()
                    .filter(|&&id| run.card(id).is_face(par))
                    .count();
                if faces > 0 {
                    run.jokers[idx].x_mult += faces as f64;
                    return some();
                }
            }
            if kind == J::GlassJoker && !bp {
                let shattered = h.scoring.len();
                if shattered > 0 {
                    run.jokers[idx].x_mult += 0.75 * shattered as f64;
                    return some();
                }
            }
            None
        }
        Ctx::FirstHandDrawn => {
            if kind == J::Certificate {
                run.playing_card_added(&[]);
                let order = standard_card_order();
                let mut g = run.rng.generator("cert_fr");
                let (suit, rank) = order[pick_index(order.len(), &mut g)];
                let id = run.new_card(suit, rank);
                let t = run.rng.random("certsl");
                run.card_mut(id).seal = if t > 0.75 {
                    Seal::Red
                } else if t > 0.5 {
                    Seal::Blue
                } else if t > 0.25 {
                    Seal::Gold
                } else {
                    Seal::Purple
                };
                run.playing_cards.push(id);
                run.hand.push(id);
                let d = run.card_should_be_debuffed(id);
                run.card_mut(id).debuffed = d;
                run.sort_hand();
                return some();
            }
            None
        }
        _ => None,
    }
}
