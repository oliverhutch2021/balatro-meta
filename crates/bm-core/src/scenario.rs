//! Building positions by hand, for tests and for setting up checks against
//! the real game (BalatroBot can set up the same positions there).

use crate::blinds::BlindKind;
use crate::card::{Edition, Enhancement, Rank, Seal, Suit};
use crate::state::{CardId, Phase, Run, RunConfig};

impl Run {
    /// A run at ante 1 inside a round of `blind`, with an empty hand and the
    /// usual 52-card deck. Nothing has been played yet.
    pub fn scenario(seed: &str, blind: BlindKind) -> Run {
        let mut run = Run::new(RunConfig::new(seed));
        run.round = 1;
        run.set_blind(blind);
        run.phase = Phase::Round;
        run
    }

    /// Replace the current blind (its start-of-blind effects apply).
    pub fn force_blind(&mut self, blind: BlindKind) {
        self.set_blind(blind);
    }

    /// Make a new card, add it to the run and put it at the right end of the
    /// hand (not sorted).
    pub fn add_card_to_hand(&mut self, suit: Suit, rank: Rank) -> CardId {
        let id = self.new_card(suit, rank);
        self.playing_cards.push(id);
        self.hand.push(id);
        let d = self.card_should_be_debuffed(id);
        self.card_mut(id).debuffed = d;
        id
    }

    /// Add a card with modifiers to the hand.
    pub fn add_special_card_to_hand(
        &mut self,
        suit: Suit,
        rank: Rank,
        enhancement: Enhancement,
        edition: Edition,
        seal: Seal,
    ) -> CardId {
        let id = self.add_card_to_hand(suit, rank);
        let c = self.card_mut(id);
        c.enhancement = enhancement;
        c.edition = edition;
        c.seal = seal;
        let d = self.card_should_be_debuffed(id);
        self.card_mut(id).debuffed = d;
        id
    }

    /// Put a consumable of `kind` into the consumable slots (room or not).
    pub fn give_consumable(&mut self, kind: crate::content::ConsumableKind) -> usize {
        let c = self.make_consumable(kind);
        self.consumables.push(c);
        self.consumables.len() - 1
    }

    /// Redeem a voucher as if bought (its effect applies at once).
    pub fn redeem(&mut self, v: crate::content::VoucherKind) {
        self.vouchers[v.index()] = true;
        self.apply_voucher(v, true);
    }

    /// Move every card in hand back to the deck.
    pub fn clear_hand(&mut self) {
        let h: Vec<CardId> = self.hand.drain(..).collect();
        for id in h {
            self.deck.insert(0, id);
        }
    }

    /// Push a position toward rare situations, for tests that must reach every path of a bot (the fair-play
    /// fuzz: `tests/fair_play.rs`, `examples/fair_fuzz.rs`): up to three jokers of any kind (some with an
    /// edition), up to two consumables, a voucher, enhancements, seals and editions on cards, and in a round
    /// any boss, cards in hand turned face down and the jokers face down and moved (as under Amber Acorn).
    /// One boss at a time, as in the game, and never a face-down card held up by Cerulean Bell. `salt` picks
    /// everything. Says what it did.
    pub fn shake(&mut self, salt: u64) -> String {
        use crate::blinds::Boss;
        use crate::content::{ConsumableKind, VoucherKind};
        use crate::joker_data::JokerKind;
        let mut z = salt ^ 0x5DEE_CE66_D1CE_4E5B;
        let mut next = move |n: usize| {
            z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut x = z;
            x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            ((x ^ (x >> 31)) % n as u64) as usize
        };
        let mut what = Vec::new();
        for _ in 0..next(4) {
            let k = JokerKind::ALL[next(JokerKind::ALL.len())];
            if next(10) < 3 {
                let e = [
                    Edition::Foil,
                    Edition::Holographic,
                    Edition::Polychrome,
                    Edition::Negative,
                ][next(4)];
                self.add_joker_with_edition(k, e);
            } else {
                self.add_joker(k);
            }
            what.push(format!("+{}", k.name()));
        }
        for _ in 0..next(3) {
            let c = ConsumableKind::ALL[next(ConsumableKind::ALL.len())];
            self.give_consumable(c);
            what.push(format!("+{c:?}"));
        }
        if next(5) == 0 {
            let v = VoucherKind::from_index(next(crate::content::N_VOUCHERS));
            if !self.vouchers[v.index()] {
                self.redeem(v);
                what.push(format!("+{v:?}"));
            }
        }
        let ids = self.playing_cards.clone();
        for _ in 0..next(7) {
            let id = ids[next(ids.len())];
            let c = self.card_mut(id);
            match next(3) {
                0 => {
                    c.enhancement = [
                        Enhancement::Bonus,
                        Enhancement::Mult,
                        Enhancement::Wild,
                        Enhancement::Glass,
                        Enhancement::Steel,
                        Enhancement::Stone,
                        Enhancement::Gold,
                        Enhancement::Lucky,
                    ][next(8)]
                }
                1 => c.seal = [Seal::Gold, Seal::Red, Seal::Blue, Seal::Purple][next(4)],
                _ => {
                    c.edition = [Edition::Foil, Edition::Holographic, Edition::Polychrome][next(3)]
                }
            }
        }
        if self.phase == Phase::Round {
            if next(2) == 0 {
                let b = Boss::ALL[next(Boss::ALL.len())];
                for id in self.hand.clone() {
                    self.card_mut(id).forced = false;
                }
                self.force_blind(BlindKind::Boss(b));
                what.push(format!("boss {}", b.name()));
            }
            if next(10) < 3 && self.hand.len() > 2 {
                for _ in 0..1 + next(3) {
                    let id = self.hand[next(self.hand.len())];
                    if !self.card(id).forced {
                        self.card_mut(id).face_down = true;
                    }
                }
                what.push("cards face down".into());
            }
            if next(5) == 0 && self.jokers.len() > 1 {
                for j in self.jokers.iter_mut() {
                    j.face_down = true;
                }
                let n = self.jokers.len();
                self.jokers.rotate_left(1 + next(n - 1));
                what.push("jokers face down".into());
            }
        }
        if next(10) < 3 {
            self.money += next(60) as i64;
        }
        what.join(" ")
    }
}
