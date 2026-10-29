//! Booster packs: opening (from the shop or a tag), their contents card by
//! card, the hand Arcana and Spectral packs deal, choosing, skipping and
//! closing (`rules/shop.md` section 3, `rules/vouchers_tags.md` 2.5).

use crate::card::{Card, Edition, Seal};
use crate::content::{BoosterId, ConsumableKind, PackKind, VoucherKind, BOOSTERS};
use crate::create::{Center, Consumable, CreateArgs, CreateKind, Created, Dest};
use crate::jokers::Joker;
use crate::poker::HandType;
use crate::run::Refused;
use crate::scoring::Ctx;
use crate::state::{Phase, Run};

/// A card shown in an open pack.
#[derive(Clone, Debug, PartialEq)]
pub enum PackCard {
    Joker(Joker),
    Consumable(Consumable),
    Card(Card),
}

#[derive(Clone, Debug, PartialEq)]
pub struct OpenPack {
    pub booster: BoosterId,
    pub cards: Vec<PackCard>,
    pub choices_left: usize,
    /// Where the game goes back to when the pack closes.
    pub return_to: Phase,
}

impl OpenPack {
    pub fn kind(&self) -> PackKind {
        BOOSTERS[self.booster as usize].kind
    }
}

fn refuse<T>(why: &str) -> Result<T, Refused> {
    Err(Refused(why.to_string()))
}

impl Run {
    /// Open a pack (already paid for): jokers react, the contents are made,
    /// and Arcana and Spectral packs deal a hand.
    pub(crate) fn open_pack(&mut self, booster: BoosterId, return_to: Phase) {
        let info = BOOSTERS[booster as usize];
        // Jokers react at the click (Hallucination rolls now, makes its Tarot
        // after the contents).
        let halu = self.open_booster_jokers();
        let ante = self.ante;
        let mut cards = Vec::with_capacity(info.size);
        for i in 0..info.size {
            let card = match info.kind {
                PackKind::Arcana => {
                    let spectral = self.vouchers[VoucherKind::OmenGlobe.index()]
                        && self.rng.random("omen_globe") > 0.8;
                    let mut a = if spectral {
                        CreateArgs::new(CreateKind::Spectral, Dest::Pack, "ar2")
                    } else {
                        CreateArgs::new(CreateKind::Tarot, Dest::Pack, "ar1")
                    };
                    a.soulable = true;
                    self.create_card(a)
                }
                PackKind::Celestial => {
                    let forced = if self.vouchers[VoucherKind::Telescope.index()] && i == 0 {
                        self.telescope_planet()
                    } else {
                        None
                    };
                    let mut a = CreateArgs::new(CreateKind::Planet, Dest::Pack, "pl1");
                    a.soulable = true;
                    a.forced = forced.map(Center::Consumable);
                    self.create_card(a)
                }
                PackKind::Spectral => {
                    let mut a = CreateArgs::new(CreateKind::Spectral, Dest::Pack, "spe");
                    a.soulable = true;
                    self.create_card(a)
                }
                PackKind::Standard => {
                    let kind = if self.rng.random(&format!("stdset{ante}")) > 0.6 {
                        CreateKind::Enhanced
                    } else {
                        CreateKind::Base
                    };
                    let mut a = CreateArgs::new(kind, Dest::Pack, "sta");
                    a.soulable = true;
                    let mut c = match self.create_card(a) {
                        Created::Card(c) => c,
                        _ => unreachable!("a playing card was asked for"),
                    };
                    c.edition = self.poll_edition(&format!("standard_edition{ante}"), 2.0, true);
                    if self.rng.random(&format!("stdseal{ante}")) > 1.0 - 0.02 * 10.0 {
                        let t = self.rng.random(&format!("stdsealtype{ante}"));
                        c.seal = if t > 0.75 {
                            Seal::Red
                        } else if t > 0.5 {
                            Seal::Blue
                        } else if t > 0.25 {
                            Seal::Gold
                        } else {
                            Seal::Purple
                        };
                    }
                    Created::Card(c)
                }
                PackKind::Buffoon => {
                    let mut a = CreateArgs::new(CreateKind::Joker, Dest::Pack, "buf");
                    a.soulable = true;
                    self.create_card(a)
                }
            };
            cards.push(match card {
                Created::Joker(j) => PackCard::Joker(j),
                Created::Consumable(c) => PackCard::Consumable(c),
                Created::Card(c) => PackCard::Card(c),
            });
        }
        // Hallucination's Tarots, decided at the opening, are made now.
        self.consumable_buffer = 0;
        for _ in 0..halu {
            self.create_consumable_into_slots(CreateKind::Tarot, "hal");
        }
        self.pack = Some(OpenPack {
            booster,
            cards,
            choices_left: info.choose,
            return_to,
        });
        self.phase = Phase::Pack;
        if matches!(info.kind, PackKind::Arcana | PackKind::Spectral) {
            self.draw_pack_hand();
        }
    }

    /// Telescope's card: the Planet of the most played hand (strongest wins
    /// ties), or None if nothing has been played.
    fn telescope_planet(&self) -> Option<ConsumableKind> {
        let mut best: Option<HandType> = None;
        let mut played = 0;
        for h in HandType::ALL.iter().rev() {
            let i = self.hand_levels.get(*h);
            if i.visible && i.played > played {
                played = i.played;
                best = Some(*h);
            }
        }
        best.map(ConsumableKind::planet_for)
    }

    /// Deal the pack's hand: up to hand size, re-sorting after each card.
    fn draw_pack_hand(&mut self) {
        let space = self.hand_size() as i64 - self.hand.len() as i64;
        let n = (self.deck.len() as i64).min(space).max(0) as usize;
        let mut drew = false;
        for _ in 0..n {
            if let Some(id) = self.deck.pop() {
                self.card_mut(id).face_down = false;
                self.hand.push(id);
                drew = true;
            }
        }
        if drew {
            self.sort_hand();
        }
    }

    /// Hallucination and other "pack opened" reactions. Returns whether a
    /// Tarot is to be made after the contents.
    fn open_booster_jokers(&mut self) -> usize {
        let mut make = 0;
        for i in 0..self.jokers.len() {
            if let Some(e) =
                crate::scoring::calculate_joker(self, i, Ctx::OpenBooster, &Default::default(), 0)
            {
                if e.make {
                    make += 1;
                }
            }
        }
        make
    }

    /// Take the pack card at `i`. `positions` are selected hand cards for a
    /// Tarot or Spectral that needs them.
    pub fn pack_choose(&mut self, i: usize, positions: &[usize]) -> Result<(), Refused> {
        if self.phase != Phase::Pack {
            return refuse("no pack is open");
        }
        let Some(card) = self.pack.as_ref().and_then(|p| p.cards.get(i)).cloned() else {
            return refuse("no card there");
        };
        match &card {
            PackCard::Joker(j) => {
                let need = if j.edition == Edition::Negative { 0 } else { 1 };
                if self.jokers.len() as i32 + need > self.joker_slots {
                    return refuse("no joker slot free");
                }
            }
            PackCard::Consumable(c) => {
                let sel = self.selection_ids(positions)?;
                if !self.can_use_consumable(c, &sel, crate::consumables::UseFrom::Pack) {
                    return refuse("it can't be used now");
                }
            }
            PackCard::Card(_) => {}
        }
        if let Some(p) = self.pack.as_mut() {
            p.cards.remove(i);
        }
        match card {
            PackCard::Joker(mut j) => {
                self.set_joker_cost(&mut j);
                self.push_joker(j);
            }
            PackCard::Consumable(c) => {
                let sel = self.selection_ids(positions)?;
                self.use_consumable_card(c, &sel, crate::consumables::UseFrom::Pack);
            }
            PackCard::Card(c) => {
                self.cards.push(c);
                let id = (self.cards.len() - 1) as u32;
                self.deck.insert(0, id);
                self.playing_card_added(&[id]);
                self.playing_cards.push(id);
            }
        }
        let done = self.pack.as_mut().is_none_or(|p| {
            p.choices_left -= 1;
            p.choices_left == 0 || p.cards.is_empty()
        });
        if done {
            self.close_pack();
        }
        Ok(())
    }

    /// Skip the rest of the pack.
    pub fn pack_skip(&mut self) -> Result<(), Refused> {
        if self.phase != Phase::Pack {
            return refuse("no pack is open");
        }
        self.joker_event_all(Ctx::SkippingBooster);
        self.close_pack();
        Ok(())
    }

    /// Close the pack: unchosen cards go, the hand goes back to the deck,
    /// the game returns, and the next pack or Boss Tag may fire.
    fn close_pack(&mut self) {
        let Some(p) = self.pack.take() else { return };
        for c in p.cards {
            match c {
                PackCard::Joker(j) => self.card_removed(Center::Joker(j.kind)),
                PackCard::Consumable(c) => self.card_removed(Center::Consumable(c.kind)),
                PackCard::Card(_) => {}
            }
        }
        // Each hand card from the left goes to the bottom of the deck.
        let hand: Vec<u32> = self.hand.drain(..).collect();
        for id in hand {
            self.deck.insert(0, id);
        }
        self.phase = p.return_to;
        if self.phase == Phase::BlindSelect {
            self.tags_new_blind_choice_pub();
        }
    }
}
