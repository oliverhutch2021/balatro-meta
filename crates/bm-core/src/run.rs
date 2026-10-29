//! The run's flow: starting a run, blind select, rounds, end of round,
//! cash-out and the shop (a stub until phase 2). Every public method is one
//! player move and checks that the game allows it at that moment.

use std::fmt;

use bm_rng::{pick_index, shuffle, Streams};

use crate::blinds::{
    blind_target, scaling_for_stake, BlindKind, BlindSlot, Boss, BossPool, SlotState,
};
use crate::card::{standard_card_order, Enhancement, Rank, Seal, SortMode, Suit};
use crate::hand_levels::HandLevels;
use crate::joker_data::JokerKind;
use crate::poker::HandType;
use crate::scoring::{calculate_joker, evaluate_play, joker_cash_out, Ctx, HandScore, PlayedHand};
use crate::state::{ActiveBlind, CardId, CashOut, HeldTag, Phase, Run, RunConfig};
use crate::tags::{draw_tag, TagKind, N_TAGS};

/// The game's walk order over its poker-hand table (LuaJIT 2.0.5, a fresh
/// run): the order of the hand lists behind Orbital Tag, To Do List and The
/// Ox's tie-break. Confirmed on the game's own runtime.
pub const HAND_WALK_ORDER: [HandType; 12] = [
    HandType::FlushHouse,
    HandType::FullHouse,
    HandType::Flush,
    HandType::Pair,
    HandType::HighCard,
    HandType::StraightFlush,
    HandType::Straight,
    HandType::TwoPair,
    HandType::FlushFive,
    HandType::FiveOfAKind,
    HandType::ThreeOfAKind,
    HandType::FourOfAKind,
];

/// A move the game refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused(pub String);

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refused {}

fn refuse<T>(why: &str) -> Result<T, Refused> {
    Err(Refused(why.to_string()))
}

/// A joker's stored sell value.
pub fn joker_sell_value(j: &crate::jokers::Joker) -> i64 {
    j.sell_cost
}

impl Run {
    /// Start a new run (`rules/run_setup.md` section 3).
    pub fn new(cfg: RunConfig) -> Run {
        let rng = Streams::new(&cfg.seed);
        let stake = cfg.stake.max(1);
        let discards = 3 + 1 - i32::from(stake >= 5); // Red Deck +1, Blue Stake -1
        let mut run = Run {
            next_sort_id: cfg.first_sort_id,
            rng,
            cards: Vec::with_capacity(64),
            playing_cards: Vec::with_capacity(64),
            deck: Vec::with_capacity(64),
            hand: Vec::with_capacity(16),
            play: Vec::with_capacity(5),
            discard: Vec::with_capacity(64),
            jokers: Vec::new(),
            joker_slots: 5,
            consumable_slots: 2,
            money: 4,
            bankrupt_at: 0,
            ante: 1,
            round: 0,
            win_ante: 8,
            won: false,
            phase: Phase::BlindSelect,
            hands_per_round: 4,
            discards_per_round: discards,
            hands_left: 4,
            discards_left: discards,
            hands_played_round: 0,
            discards_used_round: 0,
            hand_size_real: 8,
            temp_handsize: 0,
            hand_levels: HandLevels::default(),
            last_hand: None,
            most_played: HandType::HighCard,
            hands_played_run: 0,
            skips: 0,
            unused_discards: 0,
            probability: 1.0,
            slots: [SlotState::Select, SlotState::Upcoming, SlotState::Upcoming],
            boss: Boss::Hook,
            tags_offered: [TagKind::Uncommon; 2],
            bosses: BossPool::default(),
            boss_rerolled: false,
            orbital: Vec::new(),
            blind: None,
            last_blind_boss: false,
            round_chips: 0.0,
            tags: Vec::new(),
            tags_banned: [false; N_TAGS],
            idol: (Rank::Ace, Suit::Spades),
            mail_rank: Rank::Ace,
            ancient_suit: None,
            castle_suit: Suit::Spades,
            sort_mode: SortMode::Rank,
            cash_out: None,
            gros_michel_extinct: false,
            pending_money: 0,
            consumables: Vec::new(),
            vouchers: [false; crate::content::N_VOUCHERS],
            present: vec![false; crate::create::N_CENTERS],
            banned_jokers: Vec::new(),
            shop: None,
            pack: None,
            shop_voucher: None,
            joker_max: 2,
            shop_rates: Default::default(),
            discount_percent: 0,
            inflation: 0,
            edition_rate: 1.0,
            interest_cap: 25,
            interest_amount: 1,
            reroll_base: 5,
            reroll_temp: None,
            reroll_increase: 0,
            reroll_cost: 5,
            free_rerolls: 0,
            d6_this_shop: false,
            first_shop_buffoon: false,
            usage: [0; crate::content::N_CONSUMABLES],
            usage_total: Default::default(),
            last_tarot_planet: None,
            ecto_minus: 1,
            jokers_purchased_round: 0,
            boss_rerolls_allowed: false,
            joker_buffer: 0,
            cfg,
        };
        // The first boss, the first shop voucher and the two tags.
        run.boss = run.bosses.draw(&mut run.rng, 1, run.win_ante);
        run.shop_voucher = Some(run.next_voucher("Voucher1"));
        run.tags_offered = [run.draw_tag(), run.draw_tag()];
        // The 52 cards in the game's creation order, then one shuffle.
        for (s, r) in standard_card_order() {
            let id = run.new_card(s, r);
            run.playing_cards.push(id);
            run.deck.push(id);
        }
        run.shuffle_deck("shuffle");
        run.reset_round_targets(true);
        run.enter_blind_select();
        run
    }

    fn draw_tag(&mut self) -> TagKind {
        draw_tag(
            &mut self.rng,
            self.ante,
            &self.cfg.tag_profile,
            &self.tags_banned,
        )
    }

    /// The Idol, Mail-In Rebate, Ancient Joker and Castle targets
    /// (`reset_idol_card` and friends), drawn with the current ante.
    fn reset_round_targets(&mut self, run_start: bool) {
        let a = self.ante;
        let mut valid: Vec<CardId> = self
            .playing_cards
            .iter()
            .copied()
            .filter(|&id| !self.card(id).is_stone())
            .collect();
        let cards = &self.cards;
        valid.sort_by_key(|&id| cards[id as usize].sort_id);
        self.idol = (Rank::Ace, Suit::Spades);
        if !valid.is_empty() {
            let mut g = self.rng.generator(&format!("idol{a}"));
            let c = self.card(valid[pick_index(valid.len(), &mut g)]);
            self.idol = (c.rank, c.suit);
        }
        self.mail_rank = Rank::Ace;
        if !valid.is_empty() {
            let mut g = self.rng.generator(&format!("mail{a}"));
            self.mail_rank = self.card(valid[pick_index(valid.len(), &mut g)]).rank;
        }
        if run_start {
            self.ancient_suit = None;
        }
        let suits: Vec<Suit> = [Suit::Spades, Suit::Hearts, Suit::Clubs, Suit::Diamonds]
            .into_iter()
            .filter(|&s| Some(s) != self.ancient_suit)
            .collect();
        let mut g = self.rng.generator(&format!("anc{a}"));
        self.ancient_suit = Some(suits[pick_index(suits.len(), &mut g)]);
        self.castle_suit = Suit::Spades;
        if !valid.is_empty() {
            let mut g = self.rng.generator(&format!("cas{a}"));
            self.castle_suit = self.card(valid[pick_index(valid.len(), &mut g)]).suit;
        }
    }

    // ---------------------------------------------------------- blind select

    /// The slot that can be selected or skipped now.
    pub fn on_deck(&self) -> BlindSlot {
        for s in BlindSlot::ALL {
            if !matches!(
                self.slots[s.index()],
                SlotState::Defeated | SlotState::Skipped
            ) {
                return s;
            }
        }
        BlindSlot::Boss
    }

    /// What the blind in a slot is this ante.
    pub fn blind_in_slot(&self, s: BlindSlot) -> BlindKind {
        match s {
            BlindSlot::Small => BlindKind::Small,
            BlindSlot::Big => BlindKind::Big,
            BlindSlot::Boss => BlindKind::Boss(self.boss),
        }
    }

    /// The Orbital Tag hand for a slot of the current ante, if drawn.
    pub fn orbital_hand(&self, s: BlindSlot) -> Option<HandType> {
        self.orbital
            .iter()
            .find(|(a, slot, _)| *a == self.ante && *slot == s)
            .map(|x| x.2)
    }

    fn enter_blind_select(&mut self) {
        self.phase = Phase::BlindSelect;
        let on = self.on_deck();
        self.slots[on.index()] = SlotState::Select;
        // Building the three blind panels fixes this ante's Orbital Tag hands.
        for s in BlindSlot::ALL {
            if self.orbital_hand(s).is_none() {
                let visible: Vec<HandType> = HAND_WALK_ORDER
                    .iter()
                    .copied()
                    .filter(|&h| self.hand_levels.get(h).visible)
                    .collect();
                let mut g = self.rng.generator("orbital");
                let h = visible[pick_index(visible.len(), &mut g)];
                self.orbital.push((self.ante, s, h));
            }
        }
        self.tags_immediate();
        self.tags_new_blind_choice();
    }

    /// Held tags that act at once (Economy, Skip, Handy, Garbage, Orbital).
    fn tags_immediate(&mut self) {
        let mut i = 0;
        while i < self.tags.len() {
            let t = self.tags[i].clone();
            let acted = match t.kind {
                TagKind::Skip => {
                    self.money += self.skips as i64 * 5;
                    true
                }
                TagKind::Garbage => {
                    self.money += self.unused_discards as i64;
                    true
                }
                TagKind::Handy => {
                    self.money += self.hands_played_run as i64;
                    true
                }
                TagKind::Economy => {
                    self.money += self.money.clamp(0, 40);
                    true
                }
                TagKind::Orbital => {
                    if let Some(h) = t.orbital_hand {
                        self.hand_levels.level_up(h, 3);
                    }
                    true
                }
                // Top-up Tag makes Common jokers: phase 2.
                _ => false,
            };
            if acted {
                self.tags.remove(i);
            } else {
                i += 1;
            }
        }
    }

    /// The first held tag that acts on a new blind choice (pack tags, Boss
    /// Tag). Pack tags are consumed as if their pack were opened and skipped
    /// until phase 2 adds packs.
    fn tags_new_blind_choice(&mut self) {
        for i in 0..self.tags.len() {
            match self.tags[i].kind {
                TagKind::Boss => {
                    self.tags.remove(i);
                    self.reroll_boss_inner();
                    return;
                }
                TagKind::Charm
                | TagKind::Meteor
                | TagKind::Ethereal
                | TagKind::Standard
                | TagKind::Buffoon => {
                    let t = self.tags.remove(i);
                    let key = match t.kind {
                        TagKind::Charm => "p_arcana_mega_1",
                        TagKind::Meteor => "p_celestial_mega_1",
                        TagKind::Ethereal => "p_spectral_normal_1",
                        TagKind::Standard => "p_standard_mega_1",
                        _ => "p_buffoon_mega_1",
                    };
                    let id = crate::content::booster_by_key(key).expect("known pack");
                    self.open_pack(id, Phase::BlindSelect);
                    return;
                }
                _ => {}
            }
        }
    }

    pub(crate) fn add_tag_pub(&mut self, tag: HeldTag) {
        self.add_tag(tag);
    }

    pub(crate) fn tags_new_blind_choice_pub(&mut self) {
        self.tags_new_blind_choice();
    }

    pub(crate) fn end_round_pub(&mut self) {
        self.end_round();
    }

    pub(crate) fn draw_to_hand_pub(&mut self) {
        self.draw_to_hand();
        self.check_out_of_cards();
    }

    /// Add a tag: every held Double Tag copies it (never a Double Tag).
    fn add_tag(&mut self, tag: HeldTag) {
        let mut copies = 0;
        if tag.kind != TagKind::Double {
            let mut i = 0;
            while i < self.tags.len() {
                if self.tags[i].kind == TagKind::Double {
                    self.tags.remove(i);
                    copies += 1;
                } else {
                    i += 1;
                }
            }
        }
        for _ in 0..=copies {
            self.tags.push(tag.clone());
        }
    }

    /// Select the blind on deck and start its round.
    pub fn select_blind(&mut self) -> Result<(), Refused> {
        if self.phase != Phase::BlindSelect {
            return refuse("not at blind select");
        }
        let slot = self.on_deck();
        let kind = self.blind_in_slot(slot);
        self.round += 1;
        self.new_round(slot, kind);
        Ok(())
    }

    /// Skip the Small or Big Blind for its tag.
    pub fn skip_blind(&mut self) -> Result<(), Refused> {
        if self.phase != Phase::BlindSelect {
            return refuse("not at blind select");
        }
        let slot = self.on_deck();
        if slot == BlindSlot::Boss {
            return refuse("the Boss Blind can't be skipped");
        }
        self.skips += 1;
        let kind = self.tags_offered[slot.index()];
        let orbital_hand = if kind == TagKind::Orbital {
            self.orbital_hand(slot)
        } else {
            None
        };
        self.add_tag(HeldTag { kind, orbital_hand });
        self.slots[slot.index()] = SlotState::Skipped;
        let next = self.on_deck();
        self.slots[next.index()] = SlotState::Select;
        self.tags_immediate();
        self.tags_new_blind_choice();
        Ok(())
    }

    /// The Reroll Boss button works (Director's Cut once per ante, Retcon
    /// without limit, $10 each).
    pub fn can_reroll_boss(&self) -> bool {
        use crate::content::VoucherKind as V;
        let dc = self.vouchers[V::DirectorsCut.index()];
        let retcon = self.vouchers[V::Retcon.index()];
        self.phase == Phase::BlindSelect
            && self.spendable() >= 10
            && (retcon || (dc && !self.boss_rerolled))
    }

    /// Reroll this ante's boss for $10.
    pub fn reroll_boss(&mut self) -> Result<(), Refused> {
        if !self.can_reroll_boss() {
            return refuse("the boss can't be rerolled now");
        }
        self.money -= 10;
        self.reroll_boss_inner();
        Ok(())
    }

    fn reroll_boss_inner(&mut self) {
        self.boss_rerolled = true;
        self.boss = self.bosses.draw(&mut self.rng, self.ante, self.win_ante);
        self.tags_new_blind_choice();
    }

    // ---------------------------------------------------------------- round

    fn new_round(&mut self, slot: BlindSlot, kind: BlindKind) {
        self.discards_left = self.discards_per_round.max(0);
        self.hands_left = self.hands_per_round.max(1);
        self.hands_played_round = 0;
        self.discards_used_round = 0;
        self.reroll_increase = 0;
        self.hand_levels.new_round();
        for i in 0..self.playing_cards.len() {
            let id = self.playing_cards[i];
            self.card_mut(id).face_down = false;
        }
        self.free_rerolls = self.count_jokers(crate::joker_data::JokerKind::ChaosTheClown) as i32;
        self.calc_reroll_cost(true);
        self.slots[slot.index()] = SlotState::Current;
        self.set_blind(kind);
        self.joker_buffer = 0;
        let mut i = 0;
        while i < self.jokers.len() {
            calculate_joker(self, i, Ctx::SettingBlind, &PlayedHand::default(), 0);
            i += 1;
        }
        self.joker_buffer = 0;
        for j in self.jokers.iter_mut() {
            if j.sliced {
                j.removing = true;
            }
        }
        self.purge_jokers();
        let key = format!("nr{}", self.ante);
        self.shuffle_deck(&key);
        self.draw_to_hand();
    }

    /// `Blind:set_blind` for a new blind.
    pub(crate) fn set_blind(&mut self, kind: BlindKind) {
        let scaling = scaling_for_stake(self.cfg.stake);
        let boss = kind.boss();
        self.blind = Some(ActiveBlind {
            kind,
            chips: blind_target(self.ante, scaling, kind, 1.0),
            dollars: kind.reward(self.cfg.stake),
            disabled: false,
            triggered: false,
            prepped: boss != Some(Boss::Fish),
            discards_sub: 0,
            hands_sub: 0,
            eye_hands: Default::default(),
            mouth_hand: None,
        });
        self.last_blind_boss = boss.is_some();
        match boss {
            Some(Boss::Water) => {
                let d = self.discards_left;
                if let Some(b) = self.blind.as_mut() {
                    b.discards_sub = d;
                }
                self.discards_left -= d;
            }
            Some(Boss::Needle) => {
                let h = self.hands_per_round - 1;
                if let Some(b) = self.blind.as_mut() {
                    b.hands_sub = h;
                }
                self.hands_left -= h;
            }
            Some(Boss::Manacle) => self.change_hand_size(-1),
            Some(Boss::AmberAcorn) if !self.jokers.is_empty() => {
                for j in self.jokers.iter_mut() {
                    j.face_down = true;
                }
                if self.jokers.len() > 1 {
                    for _ in 0..3 {
                        self.jokers.sort_by_key(|j| j.sort_id);
                        let mut g = self.rng.generator("aajk");
                        shuffle(&mut self.jokers, &mut g);
                    }
                }
            }
            _ => {}
        }
        self.refresh_card_debuffs();
        if boss != Some(Boss::CrimsonHeart) {
            for i in 0..self.jokers.len() {
                self.set_joker_debuff(i, false);
            }
        }
    }

    /// Fill the hand (`G.FUNCS.draw_from_deck_to_hand` and the state around
    /// it): round-start tags, the draw, first-hand effects, then the blind's
    /// after-draw effect.
    fn draw_to_hand(&mut self) {
        // Juggle Tag: +3 hand size for this round.
        let mut i = 0;
        while i < self.tags.len() {
            if self.tags[i].kind == TagKind::Juggle {
                self.tags.remove(i);
                self.change_hand_size(3);
                self.temp_handsize += 3;
            } else {
                i += 1;
            }
        }
        if self.hand_size() == 0 && self.hand.is_empty() {
            self.phase = Phase::GameOver;
            return;
        }
        let serpent = self
            .blind
            .as_ref()
            .is_some_and(|b| b.active_boss() == Some(Boss::Serpent))
            && (self.hands_played_round > 0 || self.discards_used_round > 0);
        let n = if serpent {
            self.deck.len().min(3)
        } else {
            let space = self.hand_size() as i64 - self.hand.len() as i64;
            (self.deck.len() as i64).min(space).max(0) as usize
        };
        let mut drew = false;
        for _ in 0..n {
            drew |= self.draw_one_to_hand();
        }
        if drew {
            self.sort_hand();
        }
        self.phase = Phase::Round;
        self.drawn_to_hand();
    }

    /// `Blind:drawn_to_hand`: Cerulean Bell forces a card, Crimson Heart
    /// moves its joker debuff.
    fn drawn_to_hand(&mut self) {
        let boss = self.blind.as_ref().and_then(|b| b.active_boss());
        let prepped = self.blind.as_ref().is_some_and(|b| b.prepped);
        match boss {
            Some(Boss::CeruleanBell) => {
                let any_forced = self.hand.iter().any(|&id| self.card(id).forced);
                if !any_forced && !self.hand.is_empty() {
                    let mut ids = self.hand.clone();
                    let cards = &self.cards;
                    ids.sort_by_key(|&id| cards[id as usize].sort_id);
                    let mut g = self.rng.generator("cerulean_bell");
                    let id = ids[pick_index(ids.len(), &mut g)];
                    self.card_mut(id).forced = true;
                }
            }
            Some(Boss::CrimsonHeart) if prepped && !self.jokers.is_empty() => {
                let n = self.jokers.len();
                let mut cands: Vec<usize> = (0..n)
                    .filter(|&i| !self.jokers[i].debuffed || n < 2)
                    .collect();
                for i in 0..n {
                    self.set_joker_debuff(i, false);
                }
                let jokers = &self.jokers;
                cands.sort_by_key(|&i| jokers[i].sort_id);
                if !cands.is_empty() {
                    let mut g = self.rng.generator("crimson_heart");
                    let pick = cands[pick_index(cands.len(), &mut g)];
                    self.set_joker_debuff(pick, true);
                }
            }
            _ => {}
        }
        if let Some(b) = self.blind.as_mut() {
            b.prepped = false;
        }
    }

    /// The card forced into the selection by Cerulean Bell, if any.
    pub fn forced_card(&self) -> Option<usize> {
        self.hand.iter().position(|&id| self.card(id).forced)
    }

    fn check_selection(&self, positions: &[usize]) -> Result<Vec<usize>, Refused> {
        if self.phase != Phase::Round {
            return refuse("not in a round");
        }
        let mut p = positions.to_vec();
        p.sort_unstable();
        p.dedup();
        if p.len() != positions.len() {
            return refuse("a card was selected twice");
        }
        if p.is_empty() || p.len() > 5 {
            return refuse("select 1 to 5 cards");
        }
        if p.iter().any(|&i| i >= self.hand.len()) {
            return refuse("no card at that position");
        }
        if let Some(f) = self.forced_card() {
            if !p.contains(&f) {
                return refuse("Cerulean Bell's card must be selected");
            }
        }
        Ok(p)
    }

    /// Play the cards at these hand positions; they score left to right in
    /// hand order.
    pub fn play(&mut self, positions: &[usize]) -> Result<HandScore, Refused> {
        let p = self.check_selection(positions)?;
        if let Some(b) = self.blind.as_mut() {
            b.triggered = false;
        }
        for i in 0..self.playing_cards.len() {
            let id = self.playing_cards[i];
            self.card_mut(id).forced = false;
        }
        let played: Vec<CardId> = p.iter().map(|&i| self.hand[i]).collect();
        self.hand.retain(|id| !played.contains(id));
        self.hands_left -= 1;
        for &id in &played {
            let c = self.card_mut(id);
            c.times_played += 1;
            c.played_this_ante = true;
            self.play.push(id);
        }
        self.press_play();
        let score = evaluate_play(self);
        self.money += self.pending_money;
        self.pending_money = 0;
        for &id in &score.destroyed {
            self.destroy_card(id);
        }
        let rest: Vec<CardId> = self.play.drain(..).collect();
        for id in rest {
            if !score.destroyed.contains(&id) {
                self.discard.push(id);
            }
        }
        self.hands_played_round += 1;
        self.hands_played_run += 1;
        self.purge_jokers();
        if self.blind_beaten() || self.hands_left < 1 {
            self.end_round();
        } else {
            self.draw_to_hand();
            self.check_out_of_cards();
        }
        Ok(score)
    }

    /// `Blind:press_play`: The Hook, The Tooth, Crimson Heart, The Fish.
    fn press_play(&mut self) {
        let Some(boss) = self.blind.as_ref().and_then(|b| b.active_boss()) else {
            return;
        };
        match boss {
            Boss::Hook => {
                let mut pool = self.hand.clone();
                let mut picked = Vec::new();
                for i in 0..2 {
                    if self.hand.len() > i {
                        let cards = &self.cards;
                        let mut order: Vec<usize> = (0..pool.len()).collect();
                        order.sort_by_key(|&k| cards[pool[k] as usize].sort_id);
                        let mut g = self.rng.generator("hook");
                        let k = order[pick_index(order.len(), &mut g)];
                        picked.push(pool.remove(k));
                    }
                }
                if !picked.is_empty() {
                    let mut cards: Vec<CardId> = self
                        .hand
                        .iter()
                        .copied()
                        .filter(|id| picked.contains(id))
                        .collect();
                    cards.dedup();
                    self.discard_cards(cards, true);
                }
                if let Some(b) = self.blind.as_mut() {
                    b.triggered = true;
                }
            }
            Boss::Tooth => {
                self.money -= self.play.len() as i64;
                if let Some(b) = self.blind.as_mut() {
                    b.triggered = true;
                }
            }
            Boss::CrimsonHeart => {
                if !self.jokers.is_empty() {
                    if let Some(b) = self.blind.as_mut() {
                        b.triggered = true;
                        b.prepped = true;
                    }
                }
            }
            Boss::Fish => {
                if let Some(b) = self.blind.as_mut() {
                    b.prepped = true;
                }
            }
            _ => {}
        }
    }

    /// Discard the cards at these hand positions.
    pub fn discard(&mut self, positions: &[usize]) -> Result<(), Refused> {
        let p = self.check_selection(positions)?;
        if self.discards_left < 1 {
            return refuse("no discards left");
        }
        let cards: Vec<CardId> = p.iter().map(|&i| self.hand[i]).collect();
        self.discard_cards(cards, false);
        self.purge_jokers();
        self.draw_to_hand();
        self.check_out_of_cards();
        Ok(())
    }

    /// `G.FUNCS.discard_cards_from_highlighted`: `cards` are in hand order.
    fn discard_cards(&mut self, cards: Vec<CardId>, hook: bool) {
        for i in 0..self.playing_cards.len() {
            let id = self.playing_cards[i];
            self.card_mut(id).forced = false;
        }
        let none = PlayedHand {
            full: cards.clone(),
            ..PlayedHand::default()
        };
        for k in 0..self.jokers.len() {
            calculate_joker(self, k, Ctx::PreDiscard { hook }, &none, 0);
        }
        let n = cards.len();
        for (i, &id) in cards.iter().enumerate() {
            // A Purple Seal makes a Tarot here (phase 2).
            let _purple = self.card(id).seal == Seal::Purple && !self.card(id).debuffed;
            for k in 0..self.jokers.len() {
                calculate_joker(
                    self,
                    k,
                    Ctx::Discard {
                        card: id,
                        last: i + 1 == n,
                        hook,
                    },
                    &none,
                    0,
                );
            }
            self.hand.retain(|&h| h != id);
            self.discard.push(id);
        }
        if !hook {
            self.discards_used_round += 1;
            self.discards_left -= 1;
        }
    }

    /// The round also ends when hand and deck are both empty.
    fn check_out_of_cards(&mut self) {
        if self.phase == Phase::Round
            && self.hand.is_empty()
            && self.deck.is_empty()
            && self.play.is_empty()
        {
            self.end_round();
        }
    }

    /// Remove a playing card from the run (shattered, destroyed).
    fn destroy_card(&mut self, id: CardId) {
        self.playing_cards.retain(|&c| c != id);
        self.play.retain(|&c| c != id);
        self.hand.retain(|&c| c != id);
        self.deck.retain(|&c| c != id);
        self.discard.retain(|&c| c != id);
    }

    /// Jokers that destroyed themselves leave now.
    fn purge_jokers(&mut self) {
        let mut i = 0;
        while i < self.jokers.len() {
            if self.jokers[i].removing {
                self.remove_joker(i);
            } else {
                i += 1;
            }
        }
    }

    /// Sort the hand by rank or suit; the mode stays for later draws.
    pub fn sort_hand_by(&mut self, mode: SortMode) -> Result<(), Refused> {
        if self.phase != Phase::Round {
            return refuse("not in a round");
        }
        self.sort_mode = mode;
        self.sort_hand();
        Ok(())
    }

    /// Drag a hand card to another position.
    pub fn move_hand_card(&mut self, from: usize, to: usize) -> Result<(), Refused> {
        if from >= self.hand.len() || to >= self.hand.len() {
            return refuse("no card at that position");
        }
        let c = self.hand.remove(from);
        self.hand.insert(to, c);
        Ok(())
    }

    /// Drag a joker to another position.
    pub fn move_joker(&mut self, from: usize, to: usize) -> Result<(), Refused> {
        if from >= self.jokers.len() || to >= self.jokers.len() {
            return refuse("no joker at that position");
        }
        let j = self.jokers.remove(from);
        self.jokers.insert(to, j);
        Ok(())
    }

    /// Sell a joker.
    pub fn sell_joker(&mut self, idx: usize) -> Result<i64, Refused> {
        if self.phase == Phase::GameOver {
            return refuse("the run is over");
        }
        let Some(j) = self.jokers.get(idx) else {
            return refuse("no joker at that position");
        };
        if j.eternal {
            return refuse("an Eternal joker can't be sold");
        }
        let value = joker_sell_value(j);
        let kind = j.kind;
        let debuffed = j.debuffed;
        let boss = self.blind.as_ref().and_then(|b| b.active_boss());
        self.remove_joker(idx);
        self.money += value;
        if self.phase == Phase::Round
            && (boss == Some(Boss::VerdantLeaf)
                || (kind == JokerKind::Luchador && !debuffed && boss.is_some()))
        {
            self.disable_blind();
            if self.blind_beaten() {
                self.end_round();
            }
        }
        Ok(value)
    }

    // --------------------------------------------------------- end of round

    /// The round total has reached the blind's target (`chips - target >= 0`,
    /// so a NaN target can never be reached).
    pub fn blind_beaten(&self) -> bool {
        self.blind
            .as_ref()
            .is_some_and(|b| self.round_chips - b.chips >= 0.0)
    }

    fn end_round(&mut self) {
        let mut game_over = !self.blind_beaten();
        let none = PlayedHand::default();
        for i in 0..self.jokers.len() {
            if let Some(e) = calculate_joker(self, i, Ctx::EndOfRound { game_over }, &none, 0) {
                if e.saved {
                    game_over = false;
                }
            }
            if self.jokers[i].rental {
                self.money -= 3;
            }
            if self.jokers[i].perishable && self.jokers[i].perish_tally > 0 {
                self.jokers[i].perish_tally -= 1;
                if self.jokers[i].perish_tally == 0 {
                    self.set_joker_debuff(i, true);
                }
            }
        }
        let boss_round = self.blind.as_ref().is_some_and(|b| b.boss().is_some());
        if boss_round && self.ante == self.win_ante {
            self.won = true;
        }
        if game_over {
            self.purge_jokers();
            self.phase = Phase::GameOver;
            return;
        }
        self.unused_discards += self.discards_left;
        if boss_round {
            self.recompute_most_played();
        }
        self.held_end_of_round();
        // Hand to the discard pile, then (after a boss) the ante goes up,
        // then the pile goes back under the undrawn cards.
        let held: Vec<CardId> = self.hand.drain(..).collect();
        self.discard.extend(held);
        if boss_round {
            self.ante += 1;
        }
        let mut deck: Vec<CardId> = self.discard.drain(..).collect();
        deck.append(&mut self.deck);
        self.deck = deck;
        self.purge_jokers();

        self.phase = Phase::RoundEval;
        let slot = self
            .blind
            .as_ref()
            .map_or(BlindSlot::Small, |b| b.kind.slot());
        self.slots[slot.index()] = SlotState::Defeated;
        if boss_round {
            let key = format!("Voucher{}", self.ante);
            self.shop_voucher = Some(self.next_voucher(&key));
            for i in 0..self.playing_cards.len() {
                let id = self.playing_cards[i];
                self.card_mut(id).played_this_ante = false;
            }
        }
        if self.temp_handsize != 0 {
            let t = self.temp_handsize;
            self.temp_handsize = 0;
            self.change_hand_size(-t);
        }
        if self.reroll_temp.is_some() {
            self.reroll_temp = None;
            self.calc_reroll_cost(true);
        }
        self.reset_round_targets(false);
        for i in 0..self.playing_cards.len() {
            let id = self.playing_cards[i];
            self.card_mut(id).forced = false;
        }
        self.cash_out = Some(self.evaluate_round());
    }

    /// The Ox's "most played hand": the last hand in walk order with the
    /// highest play count.
    fn recompute_most_played(&mut self) {
        let mut best = HandType::HighCard;
        let mut played: i64 = -1;
        for h in HAND_WALK_ORDER {
            let p = self.hand_levels.get(h).played as i64;
            if p >= played {
                played = p;
                best = h;
            }
        }
        self.most_played = best;
    }

    /// End-of-round effects of held cards: Gold Cards pay $3 per trigger
    /// (Blue Seals make Planets in phase 2).
    fn held_end_of_round(&mut self) {
        let held = self.hand.clone();
        let none = PlayedHand::default();
        for &c in &held {
            let card = self.card(c).clone();
            let gold = card.enhancement == Enhancement::Gold && !card.debuffed;
            let blue = card.seal == Seal::Blue && !card.debuffed;
            let did = gold || blue;
            let mut reps = 1;
            if card.seal == Seal::Red && !card.debuffed && did {
                reps += 1;
            }
            for k in 0..self.jokers.len() {
                if let Some(e) =
                    calculate_joker(self, k, Ctx::EndOfRoundRepetition(c, did), &none, 0)
                {
                    reps += e.repetitions;
                }
            }
            if gold {
                self.money += 3 * reps as i64;
            }
        }
    }

    /// The cash-out rows (`rules/round.md` section 9).
    fn evaluate_round(&mut self) -> CashOut {
        let mut out = CashOut::default();
        let (reward, boss) = self
            .blind
            .as_ref()
            .map_or((0, None), |b| (b.dollars, b.boss()));
        if self.blind_beaten() {
            out.blind = reward;
        }
        // The blind is defeated: jokers turn face up, The Manacle gives its
        // hand size back.
        for j in self.jokers.iter_mut() {
            j.face_down = false;
        }
        if boss == Some(Boss::Manacle) && !self.blind.as_ref().is_some_and(|b| b.disabled) {
            self.change_hand_size(1);
        }
        if self.hands_left > 0 {
            out.hands = self.hands_left as i64;
        }
        for i in 0..self.jokers.len() {
            if let Some(d) = joker_cash_out(self, i) {
                out.jokers.push((i, d));
            }
        }
        if self.last_blind_boss {
            let n = self
                .tags
                .iter()
                .filter(|t| t.kind == TagKind::Investment)
                .count();
            self.tags.retain(|t| t.kind != TagKind::Investment);
            out.tags += 25 * n as i64;
        }
        if self.money >= 5 {
            out.interest = self.interest_amount * (self.money / 5).min(self.interest_cap / 5);
        }
        out.total = out.blind
            + out.hands
            + out.discards
            + out.jokers.iter().map(|x| x.1).sum::<i64>()
            + out.tags
            + out.interest;
        // The blind is cleared: its debuffs end.
        self.blind = None;
        self.refresh_card_debuffs();
        for i in 0..self.jokers.len() {
            self.set_joker_debuff(i, false);
        }
        out
    }

    /// Collect the cash-out total and go to the shop.
    pub fn cash_out(&mut self) -> Result<i64, Refused> {
        if self.phase != Phase::RoundEval {
            return refuse("not on the cash-out screen");
        }
        let key = format!("cashout{}", self.ante);
        self.shuffle_deck(&key);
        let total = self.cash_out.take().map_or(0, |c| c.total);
        self.hands_left = self.hands_per_round;
        self.discards_left = self.discards_per_round;
        self.money += total;
        self.round_chips = 0.0;
        self.jokers_purchased_round = 0;
        self.d6_this_shop = false;
        if self.slots[BlindSlot::Boss.index()] == SlotState::Defeated {
            self.tags_offered = [self.draw_tag(), self.draw_tag()];
            self.slots = [SlotState::Select, SlotState::Upcoming, SlotState::Upcoming];
            self.boss = self.bosses.draw(&mut self.rng, self.ante, self.win_ante);
            self.boss_rerolled = false;
        }
        self.open_shop();
        Ok(total)
    }

    /// Leave the shop for the next blind select.
    pub fn leave_shop(&mut self) -> Result<(), Refused> {
        if self.phase != Phase::Shop {
            return refuse("not in the shop");
        }
        self.close_shop();
        self.enter_blind_select();
        Ok(())
    }

    /// The run is over (lost). A won run can carry on into endless mode.
    pub fn is_over(&self) -> bool {
        self.phase == Phase::GameOver
    }
}
