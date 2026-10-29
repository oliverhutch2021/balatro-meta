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

    /// Move every card in hand back to the deck.
    pub fn clear_hand(&mut self) {
        let h: Vec<CardId> = self.hand.drain(..).collect();
        for id in h {
            self.deck.insert(0, id);
        }
    }
}
