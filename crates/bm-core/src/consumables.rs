//! Using consumables: when each can be used and what it does, in the
//! game's order (`rules/consumables.md`).

use bm_rng::{pick_index, shuffle};

use crate::card::{Edition, Enhancement, Rank, Suit};
use crate::content::{ConsumableKind, ConsumableSet, ENHANCEMENT_POOL};
use crate::create::{Center, Consumable, CreateArgs, CreateKind, Created, Dest};
use crate::poker::HandType;
use crate::run::Refused;
use crate::scoring::{Ctx, PlayedHand};
use crate::state::{CardId, Phase, Run};

/// Where a used consumable comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UseFrom {
    /// The consumable slots (the card was at this index).
    Slots,
    /// An open booster pack.
    Pack,
    /// Buy and Use in the shop.
    Shop,
}

/// Run-wide usage records.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UsageTotals {
    pub tarot: u32,
    pub planet: u32,
    pub spectral: u32,
    pub tarot_planet: u32,
    pub all: u32,
}

fn refuse<T>(why: &str) -> Result<T, Refused> {
    Err(Refused(why.to_string()))
}

/// Suits in the order the Spectral picks use them.
const SHDC: [Suit; 4] = [Suit::Spades, Suit::Hearts, Suit::Diamonds, Suit::Clubs];

fn rank_up(r: Rank) -> Rank {
    match r {
        Rank::Ace => Rank::Two,
        r => Rank::from_id(r.id() + 1).expect("2..K go up by one"),
    }
}

impl Run {
    /// Hand positions to card ids, checked (hand order).
    pub(crate) fn selection_ids(&self, positions: &[usize]) -> Result<Vec<CardId>, Refused> {
        let mut p = positions.to_vec();
        p.sort_unstable();
        p.dedup();
        if p.len() != positions.len() {
            return refuse("a card was selected twice");
        }
        if p.iter().any(|&i| i >= self.hand.len()) {
            return refuse("no card at that position");
        }
        if let Some(f) = self.forced_card() {
            if !p.is_empty() && !p.contains(&f) {
                return refuse("Cerulean Bell's card must be selected");
            }
        }
        Ok(p.iter().map(|&i| self.hand[i]).collect())
    }

    fn jokers_without_edition(&self) -> Vec<usize> {
        let mut v: Vec<usize> = (0..self.jokers.len())
            .filter(|&i| self.jokers[i].edition == Edition::None)
            .collect();
        let jk = &self.jokers;
        v.sort_by_key(|&i| jk[i].sort_id);
        v
    }

    /// Can this consumable be used now with these selected cards?
    pub fn can_use_consumable(&self, c: &Consumable, sel: &[CardId], from: UseFrom) -> bool {
        use ConsumableKind::*;
        if matches!(self.phase, Phase::GameOver) {
            return false;
        }
        let hand_phase = self.phase == Phase::Round
            || (self.phase == Phase::Pack
                && self.pack.as_ref().is_some_and(|p| {
                    matches!(
                        p.kind(),
                        crate::content::PackKind::Arcana | crate::content::PackKind::Spectral
                    )
                }));
        let room =
            self.consumables.len() < self.consumable_slots as usize || from == UseFrom::Slots;
        let joker_room = (self.jokers.len() as i32) < self.joker_slots;
        if let Some((lo, hi)) = c.kind.selection_limits() {
            return hand_phase && sel.len() >= lo && sel.len() <= hi.min(5);
        }
        match c.kind {
            Hermit | Temperance | BlackHole => true,
            k if k.set() == ConsumableSet::Planet => true,
            WheelOfFortune | Ectoplasm | Hex => !self.jokers_without_edition().is_empty(),
            Ankh => !self.jokers.is_empty() && self.joker_slots > 1 && joker_room,
            Aura => hand_phase && sel.len() == 1 && self.card(sel[0]).edition == Edition::None,
            Emperor | HighPriestess => room,
            Fool => room && self.last_tarot_planet.is_some_and(|l| l != Fool),
            Judgement | Soul | Wraith => joker_room,
            Familiar | Grim | Incantation | Immolate | Sigil | Ouija => {
                hand_phase && self.hand.len() >= 2
            }
            _ => false,
        }
    }

    /// Use the consumable in slot `i` on the hand cards at `positions`.
    pub fn use_consumable(&mut self, i: usize, positions: &[usize]) -> Result<(), Refused> {
        let Some(c) = self.consumables.get(i).cloned() else {
            return refuse("no consumable there");
        };
        let sel = self.selection_ids(positions)?;
        if !self.can_use_consumable(&c, &sel, UseFrom::Slots) {
            return refuse("it can't be used now");
        }
        self.consumables.remove(i);
        let negative = c.edition == Edition::Negative;
        self.use_consumable_card(c, &sel, UseFrom::Slots);
        if negative {
            self.consumable_slots -= 1;
        }
        // A Chicot gained mid-round may have disabled the boss.
        self.check_disabled_win();
        Ok(())
    }

    /// Make a consumable of `kind` into the slots if there's room.
    pub(crate) fn create_consumable_into_slots(&mut self, kind: CreateKind, append: &'static str) {
        if self.consumable_room_now() {
            if let Created::Consumable(c) =
                self.create_card(CreateArgs::new(kind, Dest::Consumables, append))
            {
                self.consumables.push(c);
            }
        }
    }

    /// Make a joker into the joker slots and run its add effects.
    pub(crate) fn create_joker_into_slots(&mut self, a: CreateArgs) {
        if let Created::Joker(j) = self.create_card(a) {
            self.push_joker(j);
        }
    }

    /// Destroy playing cards (Glass ones shatter): they leave every area and
    /// the deck list.
    pub(crate) fn destroy_cards(&mut self, ids: &[CardId]) {
        for &id in ids {
            self.playing_cards.retain(|&c| c != id);
            self.hand.retain(|&c| c != id);
            self.deck.retain(|&c| c != id);
            self.discard.retain(|&c| c != id);
            self.play.retain(|&c| c != id);
        }
    }

    /// A full copy of a playing card with a new identity (Cryptid, DNA).
    pub(crate) fn copy_playing_card(&mut self, from: CardId) -> CardId {
        let mut c = self.card(from).clone();
        c.sort_id = self.take_sort_id();
        c.original_suit = c.suit;
        self.cards.push(c);
        (self.cards.len() - 1) as CardId
    }

    /// The effect of a used consumable (it has left its area already).
    pub(crate) fn use_consumable_card(&mut self, c: Consumable, sel: &[CardId], from: UseFrom) {
        use ConsumableKind::*;
        let kind = c.kind;
        // Records.
        self.usage[kind.index()] += 1;
        self.usage_total.all += 1;
        match kind.set() {
            ConsumableSet::Tarot => {
                self.usage_total.tarot += 1;
                self.usage_total.tarot_planet += 1;
            }
            ConsumableSet::Planet => {
                self.usage_total.planet += 1;
                self.usage_total.tarot_planet += 1;
            }
            ConsumableSet::Spectral => self.usage_total.spectral += 1,
        }
        let empty = PlayedHand::default();
        let glass_selected = sel
            .iter()
            .filter(|&&id| self.card(id).enhancement == Enhancement::Glass)
            .count();
        let ante = self.ante;
        // Cards destroyed at the click are announced first (Caino), then the
        // use itself (Constellation, Glass Joker for The Hanged Man).
        let mut victims: Vec<CardId> = Vec::new();
        match kind {
            HangedMan => victims = sel.to_vec(),
            Familiar | Grim | Incantation => {
                let mut ids = self.hand.clone();
                let cards = &self.cards;
                ids.sort_by_key(|&id| cards[id as usize].sort_id);
                let mut g = self.rng.generator("random_destroy");
                victims.push(ids[pick_index(ids.len(), &mut g)]);
            }
            Immolate => {
                let mut ids = self.hand.clone();
                let cards = &self.cards;
                ids.sort_by_key(|&id| cards[id as usize].sort_id);
                let mut g = self.rng.generator("immolate");
                shuffle(&mut ids, &mut g);
                victims = ids.into_iter().take(5).collect();
            }
            _ => {}
        }
        if !victims.is_empty() {
            let h = PlayedHand {
                full: victims.clone(),
                ..PlayedHand::default()
            };
            for k in 0..self.jokers.len() {
                crate::scoring::calculate_joker(self, k, Ctx::RemovePlayingCards, &h, 0);
            }
        }
        let wheel_hit =
            kind == WheelOfFortune && self.rng.random("wheel_of_fortune") < self.probability / 4.0;
        let sigil_suit = (kind == Sigil).then(|| {
            let mut g = self.rng.generator("sigil");
            SHDC[pick_index(4, &mut g)]
        });
        let ouija_rank = (kind == Ouija).then(|| {
            let ranks = [
                Rank::Two,
                Rank::Three,
                Rank::Four,
                Rank::Five,
                Rank::Six,
                Rank::Seven,
                Rank::Eight,
                Rank::Nine,
                Rank::Ten,
                Rank::Jack,
                Rank::Queen,
                Rank::King,
                Rank::Ace,
            ];
            let mut g = self.rng.generator("ouija");
            ranks[pick_index(13, &mut g)]
        });
        let ankh_original = (kind == Ankh).then(|| {
            let mut v: Vec<usize> = (0..self.jokers.len()).collect();
            let jk = &self.jokers;
            v.sort_by_key(|&i| jk[i].sort_id);
            let mut g = self.rng.generator("ankh_choice");
            self.jokers[v[pick_index(v.len(), &mut g)]].sort_id
        });
        if let Some(h) = kind.planet_hand() {
            self.hand_levels.level_up(h, 1);
        }
        if kind == BlackHole {
            for h in HandType::ALL {
                self.hand_levels.level_up(h, 1);
            }
        }
        for k in 0..self.jokers.len() {
            crate::scoring::calculate_joker(
                self,
                k,
                Ctx::UsingConsumeable(kind, glass_selected as u8),
                &empty,
                0,
            );
        }

        // The queued effect.
        match kind {
            Fool => {
                if let Some(last) = self.last_tarot_planet {
                    if self.consumables.len() < self.consumable_slots as usize {
                        let mut a = CreateArgs::new(
                            Self::consumable_set_kind(last),
                            Dest::Consumables,
                            "fool",
                        );
                        a.forced = Some(Center::Consumable(last));
                        if let Created::Consumable(x) = self.create_card(a) {
                            self.consumables.push(x);
                        }
                    }
                }
            }
            HighPriestess | Emperor => {
                let free = (self.consumable_slots as usize).saturating_sub(self.consumables.len());
                let n = free.min(2);
                let (k, append) = if kind == Emperor {
                    (CreateKind::Tarot, "emp")
                } else {
                    (CreateKind::Planet, "pri")
                };
                for _ in 0..n {
                    self.create_consumable_into_slots(k, append);
                }
            }
            Hermit => self.money += self.money.clamp(0, 20),
            Temperance => {
                let total: i64 = self.jokers.iter().map(|j| j.sell_cost).sum();
                self.money += total.min(50);
            }
            WheelOfFortune => {
                if wheel_hit {
                    let cands = self.jokers_without_edition();
                    if !cands.is_empty() {
                        let mut g = self.rng.generator("wheel_of_fortune");
                        let i = cands[pick_index(cands.len(), &mut g)];
                        let e = self.poll_edition_guaranteed("wheel_of_fortune");
                        self.set_joker_edition(i, e);
                    }
                }
            }
            Strength => {
                for &id in sel {
                    let r = rank_up(self.card(id).rank);
                    self.card_mut(id).rank = r;
                    let d = self.card_should_be_debuffed(id);
                    self.card_mut(id).debuffed = d;
                }
            }
            HangedMan | Familiar | Grim | Incantation | Immolate => {
                self.destroy_cards(&victims);
                let (n, name) = match kind {
                    Familiar => (3, "familiar_create"),
                    Grim => (2, "grim_create"),
                    Incantation => (4, "incantation_create"),
                    _ => (0, ""),
                };
                let mut made = Vec::new();
                for _ in 0..n {
                    let rank = match kind {
                        Familiar => {
                            let mut g = self.rng.generator(name);
                            [Rank::Jack, Rank::Queen, Rank::King][pick_index(3, &mut g)]
                        }
                        Grim => Rank::Ace,
                        _ => {
                            let ranks = [
                                Rank::Two,
                                Rank::Three,
                                Rank::Four,
                                Rank::Five,
                                Rank::Six,
                                Rank::Seven,
                                Rank::Eight,
                                Rank::Nine,
                                Rank::Ten,
                            ];
                            let mut g = self.rng.generator(name);
                            ranks[pick_index(9, &mut g)]
                        }
                    };
                    let mut g = self.rng.generator(name);
                    let suit = SHDC[pick_index(4, &mut g)];
                    let pool: Vec<Enhancement> = ENHANCEMENT_POOL
                        .iter()
                        .copied()
                        .filter(|&e| e != Enhancement::Stone)
                        .collect();
                    let mut g = self.rng.generator("spe_card");
                    let enhancement = pool[pick_index(pool.len(), &mut g)];
                    let id = self.new_card(suit, rank);
                    self.card_mut(id).enhancement = enhancement;
                    self.playing_cards.push(id);
                    self.hand.push(id);
                    made.push(id);
                }
                if !made.is_empty() {
                    self.playing_card_added(&made);
                }
                if kind == Immolate {
                    self.money += 20;
                }
            }
            Death => {
                if sel.len() == 2 {
                    let pos = |id: CardId| self.hand.iter().position(|&h| h == id).unwrap_or(0);
                    let (a, b) = (sel[0], sel[1]);
                    let (left, right) = if pos(a) > pos(b) { (b, a) } else { (a, b) };
                    let mut copy = self.card(right).clone();
                    let keep = self.card(left).clone();
                    copy.sort_id = keep.sort_id;
                    copy.original_suit = keep.original_suit;
                    *self.card_mut(left) = copy;
                }
            }
            Talisman | DejaVu | Trance | Medium => {
                if let Some(&id) = sel.first() {
                    self.card_mut(id).seal = kind.spectral_seal().expect("seal spectral");
                }
            }
            Aura => {
                if let Some(&id) = sel.first() {
                    let e = self.poll_edition_guaranteed("aura");
                    self.card_mut(id).edition = e;
                }
            }
            Judgement => self.create_joker_into_slots(CreateArgs::new(
                CreateKind::Joker,
                Dest::Jokers,
                "jud",
            )),
            Wraith => {
                let mut a = CreateArgs::new(CreateKind::Joker, Dest::Jokers, "wra");
                a.rarity = Some(0.99);
                self.create_joker_into_slots(a);
                if self.money != 0 {
                    self.money = 0;
                }
            }
            Soul => {
                let mut a = CreateArgs::new(CreateKind::Joker, Dest::Jokers, "sou");
                a.legendary = true;
                self.create_joker_into_slots(a);
            }
            Sigil | Ouija => {
                for &id in &self.hand.clone() {
                    if let Some(s) = sigil_suit {
                        self.card_mut(id).suit = s;
                    }
                    if let Some(r) = ouija_rank {
                        self.card_mut(id).rank = r;
                    }
                    let d = self.card_should_be_debuffed(id);
                    self.card_mut(id).debuffed = d;
                }
                if kind == Ouija {
                    self.change_hand_size(-1);
                }
            }
            Ectoplasm => {
                let cands = self.jokers_without_edition();
                if !cands.is_empty() {
                    let mut g = self.rng.generator("ectoplasm");
                    let i = cands[pick_index(cands.len(), &mut g)];
                    self.set_joker_edition(i, Edition::Negative);
                    let k = self.ecto_minus;
                    self.change_hand_size(-k);
                    self.ecto_minus += 1;
                }
            }
            Ankh => {
                if let Some(orig_id) = ankh_original {
                    let mut i = 0;
                    while i < self.jokers.len() {
                        let j = &self.jokers[i];
                        if !j.eternal && j.sort_id != orig_id {
                            self.remove_joker(i);
                        } else {
                            i += 1;
                        }
                    }
                    if let Some(orig) = self.jokers.iter().find(|j| j.sort_id == orig_id).cloned() {
                        let mut copy = orig.clone();
                        copy.sort_id = self.take_sort_id();
                        if orig.kind == crate::joker_data::JokerKind::ToDoList {
                            let _ = self.pick_to_do_hand(None);
                        }
                        if copy.edition == Edition::Negative {
                            copy.edition = Edition::None;
                        }
                        copy.added = false;
                        self.set_joker_cost(&mut copy);
                        self.mark_present(Center::Joker(copy.kind));
                        self.push_joker(copy);
                    }
                }
            }
            Hex => {
                let cands = self.jokers_without_edition();
                if !cands.is_empty() {
                    let mut g = self.rng.generator("hex");
                    let i = cands[pick_index(cands.len(), &mut g)];
                    let keep = self.jokers[i].sort_id;
                    self.set_joker_edition(i, Edition::Polychrome);
                    let mut j = 0;
                    while j < self.jokers.len() {
                        let jk = &self.jokers[j];
                        if jk.sort_id != keep && !jk.eternal {
                            self.remove_joker(j);
                        } else {
                            j += 1;
                        }
                    }
                }
            }
            Cryptid => {
                if let Some(&id) = sel.first() {
                    let mut made = Vec::new();
                    for _ in 0..2 {
                        let n = self.copy_playing_card(id);
                        self.playing_cards.push(n);
                        self.hand.push(n);
                        made.push(n);
                    }
                    self.playing_card_added(&made);
                }
            }
            k => {
                if let Some(e) = k.tarot_enhancement() {
                    for &id in sel {
                        self.set_enhancement(id, e);
                    }
                } else if let Some(s) = k.tarot_suit() {
                    for &id in sel {
                        self.card_mut(id).suit = s;
                        let d = self.card_should_be_debuffed(id);
                        self.card_mut(id).debuffed = d;
                    }
                }
            }
        }
        let _ = ante;
        // After the effect: the last-used record, then the card is gone.
        if matches!(kind.set(), ConsumableSet::Tarot | ConsumableSet::Planet) {
            self.last_tarot_planet = Some(kind);
        }
        self.card_removed(Center::Consumable(kind));
        if from != UseFrom::Pack {
            self.refresh_card_debuffs();
        }
        // An emptied hand in a round refills (and may end the round).
        if self.phase == Phase::Round && self.hand.is_empty() {
            if self.deck.is_empty() {
                self.end_round_pub();
            } else {
                self.draw_to_hand_pub();
            }
        }
    }

    /// Give a joker an edition (a Negative one adds a slot unless debuffed).
    pub(crate) fn set_joker_edition(&mut self, i: usize, e: Edition) {
        let was_negative = self.jokers[i].edition == Edition::Negative;
        self.jokers[i].edition = e;
        if e == Edition::Negative && !was_negative && !self.jokers[i].debuffed {
            self.joker_slots += 1;
        }
        let mut j = self.jokers[i].clone();
        self.set_joker_cost(&mut j);
        self.jokers[i] = j;
    }
}
