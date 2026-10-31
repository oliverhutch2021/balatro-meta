//! The run's state: cards and their areas, jokers, money, the blind and the
//! counters the rules read (`rules/run_setup.md`, `rules/round.md`).

use bm_rng::{shuffle, Streams};

use crate::blinds::{BlindKind, BlindSlot, Boss, BossPool, SlotState};
use crate::card::{sort_hand, Card, Edition, Enhancement, Rank, SortMode, Suit};
use crate::consumables::UsageTotals;
use crate::content::{ConsumableKind, VoucherKind, N_CONSUMABLES, N_VOUCHERS};
use crate::create::Consumable;
use crate::hand_levels::HandLevels;
use crate::joker_data::JokerKind;
use crate::jokers::{joker_resource_change, Joker};
use crate::packs::OpenPack;
use crate::poker::{HandSet, HandType};
use crate::shop::Shop;
use crate::tags::{TagKind, TagProfile, N_TAGS};

/// The shop's kind weights (`rules/shop.md` 1.3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShopRates {
    pub joker: f64,
    pub tarot: f64,
    pub planet: f64,
    pub playing_card: f64,
    pub spectral: f64,
}

impl Default for ShopRates {
    fn default() -> Self {
        ShopRates {
            joker: 20.0,
            tarot: 4.0,
            planet: 4.0,
            playing_card: 0.0,
            spectral: 0.0,
        }
    }
}

/// Index of a playing card in [`Run::cards`].
pub type CardId = u32;

/// Where the run is, from the player's point of view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Phase {
    /// Choosing, skipping or rerolling a blind.
    BlindSelect,
    /// In a round, choosing cards to play or discard.
    Round,
    /// The cash-out screen after a won round.
    RoundEval,
    /// The shop.
    Shop,
    /// A booster pack is open (from the shop or a tag).
    Pack,
    /// The run is lost.
    GameOver,
}

/// The deck chosen for the run. Version 1 is Red Deck only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum DeckKind {
    #[default]
    Red,
}

/// What a run starts from.
#[derive(Clone, Debug, PartialEq)]
pub struct RunConfig {
    /// The seed (already normalised: 1-8 characters from 1-9 and A-Z).
    pub seed: String,
    pub deck: DeckKind,
    /// 1 White ... 8 Gold.
    pub stake: u8,
    pub tag_profile: TagProfile,
    /// The first `sort_id` to hand out. The game's counter never resets, so
    /// only the order matters; this exists for tests that mirror a session.
    pub first_sort_id: u64,
}

impl RunConfig {
    pub fn new(seed: &str) -> RunConfig {
        RunConfig {
            seed: seed.to_string(),
            deck: DeckKind::Red,
            stake: 1,
            tag_profile: TagProfile::default(),
            first_sort_id: 1,
        }
    }
}

/// The blind being played.
#[derive(Clone, Debug, PartialEq)]
pub struct ActiveBlind {
    pub kind: BlindKind,
    /// Chip target.
    pub chips: f64,
    /// Reward paid at cash-out if the target was reached.
    pub dollars: i64,
    pub disabled: bool,
    /// The boss's ability hit the current hand (read by Matador).
    pub triggered: bool,
    /// The Fish and Crimson Heart: armed for the next draw.
    pub prepped: bool,
    /// Discards removed by The Water.
    pub discards_sub: i32,
    /// Hands removed by The Needle.
    pub hands_sub: i32,
    /// The Eye: hand types already played this round.
    pub eye_hands: HandSet,
    /// The Mouth: the only hand type allowed this round.
    pub mouth_hand: Option<HandType>,
}

impl ActiveBlind {
    pub fn boss(&self) -> Option<Boss> {
        self.kind.boss()
    }

    /// The boss, if it is one and not disabled.
    pub fn active_boss(&self) -> Option<Boss> {
        if self.disabled {
            None
        } else {
            self.kind.boss()
        }
    }
}

/// A skip tag the player holds.
#[derive(Clone, Debug, PartialEq)]
pub struct HeldTag {
    pub kind: TagKind,
    /// Orbital Tag: the hand it levels.
    pub orbital_hand: Option<HandType>,
}

/// The cash-out screen's rows (`rules/round.md` section 9).
#[derive(Debug, Default, PartialEq)]
pub struct CashOut {
    pub blind: i64,
    pub hands: i64,
    pub discards: i64,
    /// (joker index, dollars) for jokers that pay at cash-out.
    pub jokers: Vec<(usize, i64)>,
    pub tags: i64,
    pub interest: i64,
    pub total: i64,
}

/// One run of Balatro.
#[derive(Debug)]
pub struct Run {
    pub cfg: RunConfig,
    pub rng: Streams,
    pub(crate) next_sort_id: u64,

    /// Every playing card ever made in this run; areas hold indices into it.
    pub cards: Vec<Card>,
    /// The run's playing cards (`G.playing_cards`), in the order they joined.
    pub playing_cards: Vec<CardId>,
    /// Draw pile: drawn from the end, cards put back at the front.
    pub deck: Vec<CardId>,
    pub hand: Vec<CardId>,
    pub play: Vec<CardId>,
    pub discard: Vec<CardId>,

    pub jokers: Vec<Joker>,
    pub joker_slots: i32,
    pub consumable_slots: i32,

    pub money: i64,
    /// Money can go down to this (Credit Card: -20).
    pub bankrupt_at: i64,
    pub ante: i32,
    pub round: i32,
    pub win_ante: i32,
    pub won: bool,
    pub phase: Phase,

    /// Hands and discards per round (`round_resets`).
    pub hands_per_round: i32,
    pub discards_per_round: i32,
    pub hands_left: i32,
    pub discards_left: i32,
    pub hands_played_round: i32,
    pub discards_used_round: i32,
    /// Hand size as set by effects (can go below 0); the hand holds
    /// `max(0, hand_size_real)` cards.
    pub hand_size_real: i32,
    /// Extra hand size from Juggle Tags for this round.
    pub temp_handsize: i32,
    pub hand_levels: HandLevels,
    pub last_hand: Option<HandType>,
    /// The Ox's target.
    pub most_played: HandType,
    pub hands_played_run: i32,
    pub skips: i32,
    pub unused_discards: i32,
    /// Chance numerator (1; doubled by each Oops! All 6s).
    pub probability: f64,

    pub slots: [SlotState; 3],
    pub boss: Boss,
    pub tags_offered: [TagKind; 2],
    pub bosses: BossPool,
    pub boss_rerolled: bool,
    /// Orbital Tag hands per (ante, slot), drawn when blind select is built.
    pub orbital: Vec<(i32, BlindSlot, HandType)>,
    pub blind: Option<ActiveBlind>,
    /// Whether the last blind played was a boss (Investment Tag, Rocket).
    pub last_blind_boss: bool,
    /// Chips scored this round.
    pub round_chips: f64,
    pub tags: Vec<HeldTag>,
    pub tags_banned: [bool; N_TAGS],

    /// The Idol's card, Mail-In Rebate's rank, Ancient Joker's and Castle's
    /// suits for this round.
    pub idol: (Rank, Suit),
    pub mail_rank: Rank,
    pub ancient_suit: Option<Suit>,
    pub castle_suit: Suit,

    pub sort_mode: SortMode,
    pub cash_out: Option<CashOut>,
    /// Gros Michel has gone extinct (Cavendish may appear).
    pub gros_michel_extinct: bool,
    /// Money produced during the current evaluation, not yet added
    /// (`G.GAME.dollar_buffer`).
    pub(crate) pending_money: i64,

    // ---- phase 2: consumables, vouchers, the shop and packs
    pub consumables: Vec<Consumable>,
    /// Vouchers redeemed this run (index = `VoucherKind::index`).
    pub vouchers: [bool; N_VOUCHERS],
    /// "Present" marks that keep duplicates out of pools (`create::Center`).
    pub present: Vec<bool>,
    pub banned_jokers: Vec<crate::joker_data::JokerKind>,
    pub shop: Option<Shop>,
    pub pack: Option<OpenPack>,
    /// The ante's voucher, offered in each shop until a voucher is redeemed.
    pub shop_voucher: Option<VoucherKind>,
    /// Joker-row slots in the shop.
    pub joker_max: i32,
    pub shop_rates: ShopRates,
    pub discount_percent: i64,
    pub inflation: i64,
    pub edition_rate: f64,
    pub interest_cap: i64,
    pub interest_amount: i64,
    pub reroll_base: i64,
    /// D6 Tag's override of the base reroll cost (until the next round ends).
    pub reroll_temp: Option<i64>,
    pub reroll_increase: i64,
    pub reroll_cost: i64,
    pub free_rerolls: i32,
    pub(crate) d6_this_shop: bool,
    /// The run's first shop pack (a Buffoon Pack) has been placed.
    pub first_shop_buffoon: bool,
    pub usage: [u32; N_CONSUMABLES],
    pub usage_total: UsageTotals,
    pub last_tarot_planet: Option<ConsumableKind>,
    /// Ectoplasm's hand-size cost for its next use.
    pub ecto_minus: i32,
    /// Jokers bought this round (Flash Card and friends read it later).
    pub jokers_purchased_round: i32,
    /// Blind rerolls are possible (Director's Cut, Retcon).
    pub boss_rerolls_allowed: bool,
    /// `G.GAME.joker_buffer` during a blind start (Ceremonial Dagger,
    /// Riff-raff).
    pub(crate) joker_buffer: i32,
    /// `G.GAME.consumeable_buffer`: consumables decided but not yet made
    /// (Hallucination's Tarots wait for the pack's contents).
    pub(crate) consumable_buffer: i32,
    /// The Blueprint or Brainstorm that started the current copy chain
    /// (`context.blueprint_card`), while one is running.
    pub(crate) blueprint_card: Option<usize>,
    /// Blind start, steps the game queues behind the jokers' blind-start
    /// event: Riff-raff's jokers to make, Chicot's disables, and Amber
    /// Acorn's three joker shuffles.
    pub(crate) riffraff_pending: i32,
    pub(crate) chicot_disables: i32,
    pub(crate) acorn_shuffles_pending: bool,
    /// Cards a Vampire stripped during the current hand (a second Vampire
    /// skips them).
    pub(crate) vampired: Vec<CardId>,
}

/// Clone written out field by field, with a `clone_from` that reuses each field's memory: copies of a run are
/// made by the million (each hand a search scores is played on one), so a copy can be made into a run kept for
/// the purpose. The patterns name every field, so the compiler catches a field left out.
macro_rules! clone_by_field {
    ($t:ident { $($f:ident),* $(,)? }) => {
        impl Clone for $t {
            fn clone(&self) -> Self {
                let $t { $($f),* } = self;
                $t { $($f: $f.clone()),* }
            }

            fn clone_from(&mut self, source: &Self) {
                let $t { $($f),* } = self;
                $($f.clone_from(&source.$f);)*
            }
        }
    };
}
pub(crate) use clone_by_field;

clone_by_field!(CashOut {
    blind,
    hands,
    discards,
    jokers,
    tags,
    interest,
    total,
});

clone_by_field!(Run {
    cfg,
    rng,
    next_sort_id,
    cards,
    playing_cards,
    deck,
    hand,
    play,
    discard,
    jokers,
    joker_slots,
    consumable_slots,
    money,
    bankrupt_at,
    ante,
    round,
    win_ante,
    won,
    phase,
    hands_per_round,
    discards_per_round,
    hands_left,
    discards_left,
    hands_played_round,
    discards_used_round,
    hand_size_real,
    temp_handsize,
    hand_levels,
    last_hand,
    most_played,
    hands_played_run,
    skips,
    unused_discards,
    probability,
    slots,
    boss,
    tags_offered,
    bosses,
    boss_rerolled,
    orbital,
    blind,
    last_blind_boss,
    round_chips,
    tags,
    tags_banned,
    idol,
    mail_rank,
    ancient_suit,
    castle_suit,
    sort_mode,
    cash_out,
    gros_michel_extinct,
    pending_money,
    consumables,
    vouchers,
    present,
    banned_jokers,
    shop,
    pack,
    shop_voucher,
    joker_max,
    shop_rates,
    discount_percent,
    inflation,
    edition_rate,
    interest_cap,
    interest_amount,
    reroll_base,
    reroll_temp,
    reroll_increase,
    reroll_cost,
    free_rerolls,
    d6_this_shop,
    first_shop_buffoon,
    usage,
    usage_total,
    last_tarot_planet,
    ecto_minus,
    jokers_purchased_round,
    boss_rerolls_allowed,
    joker_buffer,
    consumable_buffer,
    blueprint_card,
    riffraff_pending,
    chicot_disables,
    acorn_shuffles_pending,
    vampired,
});

impl Run {
    // ------------------------------------------------------------ identity

    pub(crate) fn take_sort_id(&mut self) -> u64 {
        let id = self.next_sort_id;
        self.next_sort_id += 1;
        id
    }

    /// Make a playing card (not yet in any area or in the run's deck list).
    pub(crate) fn new_card(&mut self, suit: Suit, rank: Rank) -> CardId {
        let sort_id = self.take_sort_id();
        self.cards.push(Card::new(sort_id, suit, rank));
        (self.cards.len() - 1) as CardId
    }

    pub fn card(&self, id: CardId) -> &Card {
        &self.cards[id as usize]
    }

    pub fn card_mut(&mut self, id: CardId) -> &mut Card {
        &mut self.cards[id as usize]
    }

    /// Cards in hand, left to right.
    pub fn hand_cards(&self) -> Vec<&Card> {
        self.hand.iter().map(|&id| self.card(id)).collect()
    }

    // ------------------------------------------------------------- jokers

    /// An active (owned, not debuffed) joker of this kind is held.
    pub fn has_joker(&self, kind: JokerKind) -> bool {
        self.jokers.iter().any(|j| j.kind == kind && !j.debuffed)
    }

    pub fn count_jokers(&self, kind: JokerKind) -> usize {
        self.jokers
            .iter()
            .filter(|j| j.kind == kind && !j.debuffed)
            .count()
    }

    pub fn smeared(&self) -> bool {
        self.has_joker(JokerKind::SmearedJoker)
    }

    pub fn pareidolia(&self) -> bool {
        self.has_joker(JokerKind::Pareidolia)
    }

    /// The hand's card limit.
    pub fn hand_size(&self) -> i32 {
        self.hand_size_real.max(0)
    }

    /// Change hand size; a rise during a round draws the extra cards at once
    /// if the hand holds any (`rules/round.md` 4.8).
    pub(crate) fn change_hand_size(&mut self, delta: i32) {
        if delta == 0 {
            return;
        }
        self.hand_size_real += delta;
        if delta > 0
            && self.hand_size_real > 1
            && !self.hand.is_empty()
            && self.phase == Phase::Round
        {
            for _ in 0..delta {
                self.draw_one_to_hand();
            }
            self.sort_hand();
        }
    }

    /// A joker joins (or its debuff is lifted): apply its resource changes.
    pub(crate) fn add_joker_effects(&mut self, idx: usize) {
        if self.jokers[idx].added {
            return;
        }
        self.jokers[idx].added = true;
        let j = self.jokers[idx].clone();
        let (h, d, hands) = joker_resource_change(&j);
        if h != 0 {
            self.change_hand_size(h);
        }
        if d > 0 {
            self.discards_per_round += d;
            self.discards_left += d;
        }
        if hands != 0 {
            self.hands_per_round += hands;
        }
        match j.kind {
            JokerKind::CreditCard => self.bankrupt_at -= 20,
            JokerKind::OopsAllSixes => self.probability *= 2.0,
            JokerKind::Chicot
                if self
                    .blind
                    .as_ref()
                    .is_some_and(|b| b.boss().is_some() && !b.disabled) =>
            {
                self.disable_blind();
            }
            JokerKind::ChaosTheClown => {
                self.free_rerolls += 1;
                self.calc_reroll_cost(true);
            }
            JokerKind::ToTheMoon => self.interest_amount += 1,
            _ => {}
        }
    }

    /// A joker leaves (or is debuffed): undo its resource changes.
    pub(crate) fn remove_joker_effects(&mut self, idx: usize) {
        if !self.jokers[idx].added {
            return;
        }
        self.jokers[idx].added = false;
        let j = self.jokers[idx].clone();
        let (h, d, hands) = joker_resource_change(&j);
        if h != 0 {
            self.change_hand_size(-h);
        }
        if d > 0 {
            self.discards_per_round -= d;
            // Discards left can't go below 0 (`ease_discard`).
            self.discards_left = (self.discards_left - d).max(0);
        }
        if hands != 0 {
            self.hands_per_round -= hands;
        }
        match j.kind {
            JokerKind::CreditCard => self.bankrupt_at += 20,
            JokerKind::OopsAllSixes => self.probability /= 2.0,
            JokerKind::ChaosTheClown => {
                self.free_rerolls -= 1;
                self.calc_reroll_cost(true);
            }
            JokerKind::ToTheMoon => self.interest_amount -= 1,
            _ => {}
        }
    }

    /// The queued follow-ups of a joker joining, leaving, or changing its
    /// debuff: every playing card's boss debuff is tested again, and
    /// Astronomer re-prices every card.
    pub(crate) fn joker_area_changed(&mut self, kind: JokerKind) {
        self.refresh_card_debuffs();
        if kind == JokerKind::Astronomer {
            self.reprice_owned();
        }
    }

    /// Give the player a joker at the right end of the joker row, made as
    /// the game makes one (a To Do List draws its hand, a Loyalty Card
    /// starts its cycle).
    pub fn add_joker(&mut self, kind: JokerKind) -> usize {
        let j = self.make_joker(kind);
        self.push_joker(j);
        self.jokers.len() - 1
    }

    /// Give the player a joker with an edition.
    pub fn add_joker_with_edition(&mut self, kind: JokerKind, edition: Edition) -> usize {
        let mut j = self.make_joker(kind);
        j.edition = edition;
        self.set_joker_cost(&mut j);
        self.push_joker(j);
        self.jokers.len() - 1
    }

    /// Put a joker into the joker area: its add effects, a Negative's extra
    /// slot, then the area's follow-ups.
    pub(crate) fn push_joker(&mut self, j: Joker) {
        let negative = j.edition == Edition::Negative;
        let kind = j.kind;
        self.jokers.push(j);
        let idx = self.jokers.len() - 1;
        self.add_joker_effects(idx);
        if negative {
            self.joker_slots += 1;
        }
        self.joker_area_changed(kind);
    }

    /// Remove the joker at `idx` from the run (`Card:remove`): its remove
    /// effects, a Negative's slot, and its pool mark unless another copy
    /// is held.
    pub(crate) fn remove_joker(&mut self, idx: usize) {
        self.remove_joker_effects(idx);
        let j = self.jokers.remove(idx);
        if j.edition == Edition::Negative {
            self.joker_slots -= 1;
        }
        self.card_removed(crate::create::Center::Joker(j.kind));
        self.joker_area_changed(j.kind);
    }

    /// Debuff or un-debuff a joker, with its add/remove effects.
    pub(crate) fn set_joker_debuff(&mut self, idx: usize, debuff: bool) {
        let j = &self.jokers[idx];
        let kind = j.kind;
        if j.perishable && j.perish_tally <= 0 {
            if !j.debuffed {
                self.remove_joker_effects(idx);
                self.jokers[idx].debuffed = true;
                self.joker_area_changed(kind);
            }
            return;
        }
        if debuff != j.debuffed {
            if debuff {
                self.remove_joker_effects(idx);
            } else {
                self.add_joker_effects(idx);
            }
            self.jokers[idx].debuffed = debuff;
            self.joker_area_changed(kind);
        }
    }

    /// Room for one more consumable, counting ones already decided
    /// (`#consumeables + consumeable_buffer < limit`).
    pub(crate) fn consumable_room_now(&self) -> bool {
        (self.consumables.len() as i32) + self.consumable_buffer < self.consumable_slots
    }

    // ------------------------------------------------- per-frame values

    /// A joker's values that the game recomputes every frame
    /// (`Card:update`), from the current state: Joker Stencil's and
    /// Throwback's X-mult, Swashbuckler's mult, and the deck tallies of
    /// Steel Joker, Stone Joker, Driver's License and Cloud 9 (kept in
    /// `counter`). `own` is the joker's index in the joker area, if it is
    /// there.
    pub fn frame_refresh(&self, j: &mut Joker, own: Option<usize>) {
        let tally = |f: &dyn Fn(&crate::card::Card) -> bool| {
            self.playing_cards
                .iter()
                .filter(|&&id| f(self.card(id)))
                .count() as i64
        };
        match j.kind {
            JokerKind::JokerStencil => {
                let stencils = self
                    .jokers
                    .iter()
                    .filter(|o| o.kind == JokerKind::JokerStencil)
                    .count() as i32;
                j.x_mult = (self.joker_slots - self.jokers.len() as i32 + stencils) as f64;
            }
            JokerKind::Swashbuckler => {
                let sum: i64 = self
                    .jokers
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| Some(*i) != own)
                    .map(|(_, o)| o.sell_cost)
                    .sum();
                j.mult = sum as f64;
            }
            JokerKind::Throwback => j.x_mult = 1.0 + self.skips as f64 * 0.25,
            JokerKind::SteelJoker => {
                j.counter = tally(&|c| c.enhancement == Enhancement::Steel);
            }
            JokerKind::StoneJoker => {
                j.counter = tally(&|c| c.enhancement == Enhancement::Stone);
            }
            JokerKind::DriversLicense => {
                j.counter = tally(&|c| c.enhancement != Enhancement::None);
            }
            JokerKind::Cloud9 => j.counter = tally(&|c| c.rank_id() == Some(9)),
            _ => {}
        }
    }

    /// Refresh every owned joker's per-frame values. The evaluation of a
    /// hand runs inside one frame, so it sees the values from before it.
    pub(crate) fn refresh_frame_values(&mut self) {
        for i in 0..self.jokers.len() {
            let mut j = self.jokers[i].clone();
            self.frame_refresh(&mut j, Some(i));
            self.jokers[i] = j;
        }
    }

    // -------------------------------------------------------------- cards

    /// The boss's debuff test for one playing card (`Blind:debuff_card`).
    pub(crate) fn card_should_be_debuffed(&self, id: CardId) -> bool {
        let Some(b) = self.blind.as_ref() else {
            return false;
        };
        let Some(boss) = b.active_boss() else {
            return false;
        };
        let c = self.card(id);
        let smeared = self.smeared();
        let par = self.pareidolia();
        match boss {
            Boss::Club => c.has_suit_for_boss(Suit::Clubs, smeared),
            Boss::Goad => c.has_suit_for_boss(Suit::Spades, smeared),
            Boss::Head => c.has_suit_for_boss(Suit::Hearts, smeared),
            Boss::Window => c.has_suit_for_boss(Suit::Diamonds, smeared),
            Boss::Plant => c.is_face_for_boss(par),
            Boss::Pillar => c.played_this_ante,
            Boss::VerdantLeaf => true,
            _ => false,
        }
    }

    /// Re-run the boss's debuff test on every playing card (the game does
    /// this when a blind is set or disabled, and whenever a joker joins or
    /// leaves).
    pub(crate) fn refresh_card_debuffs(&mut self) {
        for i in 0..self.playing_cards.len() {
            let id = self.playing_cards[i];
            let d = self.card_should_be_debuffed(id);
            self.cards[id as usize].debuffed = d;
        }
    }

    /// Change a playing card's enhancement (`set_ability`): it loses its
    /// "played this ante" mark and is re-tested for the boss's debuff.
    pub(crate) fn set_enhancement(&mut self, id: CardId, e: Enhancement) {
        let c = self.card_mut(id);
        c.enhancement = e;
        c.played_this_ante = false;
        let d = self.card_should_be_debuffed(id);
        self.card_mut(id).debuffed = d;
    }

    /// Sort the hand by the current mode (descending; equal keys keep
    /// creation order).
    pub(crate) fn sort_hand(&mut self) {
        let mode = self.sort_mode;
        let cards = &self.cards;
        sort_hand(&mut self.hand, |id| {
            let c = &cards[id as usize];
            (c.sort_key(mode), c.sort_id)
        });
    }

    /// Shuffle the deck on a named stream (sorted by creation first).
    pub(crate) fn shuffle_deck(&mut self, key: &str) {
        let cards = &self.cards;
        self.deck.sort_by_key(|&id| cards[id as usize].sort_id);
        let mut g = self.rng.generator(key);
        shuffle(&mut self.deck, &mut g);
    }

    /// Take the top card of the deck into the hand (face down if the boss
    /// says so). Does not sort.
    pub(crate) fn draw_one_to_hand(&mut self) -> bool {
        let Some(id) = self.deck.pop() else {
            return false;
        };
        let flipped = self.stays_flipped(id);
        let c = self.card_mut(id);
        c.face_down = flipped;
        self.hand.push(id);
        true
    }

    /// `Blind:stay_flipped` for a card arriving in the hand.
    fn stays_flipped(&mut self, id: CardId) -> bool {
        let Some(boss) = self.blind.as_ref().and_then(|b| b.active_boss()) else {
            return false;
        };
        match boss {
            Boss::Wheel => self.rng.random("wheel") < self.probability / 7.0,
            Boss::House => self.hands_played_round == 0 && self.discards_used_round == 0,
            Boss::Mark => self.card(id).is_face_for_boss(self.pareidolia()),
            Boss::Fish => self.blind.as_ref().is_some_and(|b| b.prepped),
            _ => false,
        }
    }

    // --------------------------------------------------------------- blind

    /// `Blind:disable` (Chicot, Luchador, Verdant Leaf's sale).
    pub(crate) fn disable_blind(&mut self) {
        let Some(b) = self.blind.as_mut() else {
            return;
        };
        b.disabled = true;
        let boss = b.boss();
        for j in self.jokers.iter_mut() {
            j.face_down = false;
        }
        match boss {
            Some(Boss::Water) => {
                let d = self.blind.as_ref().map_or(0, |b| b.discards_sub);
                self.discards_left += d;
            }
            Some(Boss::Wheel) | Some(Boss::House) | Some(Boss::Mark) | Some(Boss::Fish) => {
                for i in 0..self.hand.len() {
                    let id = self.hand[i];
                    self.card_mut(id).face_down = false;
                }
            }
            Some(Boss::Needle) => {
                let h = self.blind.as_ref().map_or(0, |b| b.hands_sub);
                self.hands_left += h;
            }
            Some(Boss::Wall) => {
                if let Some(b) = self.blind.as_mut() {
                    b.chips /= 2.0;
                }
            }
            Some(Boss::CeruleanBell) => {
                for i in 0..self.playing_cards.len() {
                    let id = self.playing_cards[i];
                    self.card_mut(id).forced = false;
                }
            }
            Some(Boss::Manacle) => {
                // The size change draws one card itself when the hand holds
                // cards, and the disable draws one more: mid-round that is two
                // cards (a quirk of the game), at blind start just the one.
                self.change_hand_size(1);
                if self.phase == Phase::Round && self.draw_one_to_hand() {
                    self.sort_hand();
                }
            }
            Some(Boss::VioletVessel) => {
                if let Some(b) = self.blind.as_mut() {
                    b.chips /= 3.0;
                }
            }
            _ => {}
        }
        self.refresh_card_debuffs();
        for i in 0..self.jokers.len() {
            self.set_joker_debuff(i, false);
        }
    }

    // -------------------------------------------------------------- money

    /// Money as Bull and Bootstraps read it during scoring.
    pub(crate) fn money_with_pending(&self) -> i64 {
        self.money + self.pending_money
    }
}
