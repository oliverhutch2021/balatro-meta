//! Card creation: the "present" marks that keep duplicates out of pools,
//! pools with their placeholders, the general creator with its Soul checks,
//! sticker and edition rolls, and prices (`rules/shop.md` sections 2 and 5).

use bm_rng::pick_index;

use crate::card::{standard_card_order, Card, Edition, Enhancement};
use crate::content::{
    ConsumableKind, ConsumableSet, VoucherKind, BOOSTERS, ENHANCEMENT_POOL, N_CONSUMABLES,
    N_VOUCHERS,
};
use crate::joker_data::{JokerKind, PoolFlag, JOKERS, N_JOKERS};
use crate::jokers::Joker;
use crate::poker::HandType;
use crate::run::HAND_WALK_ORDER;
use crate::state::Run;

/// An item that pools hold and "present" marks track.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Center {
    Joker(JokerKind),
    Consumable(ConsumableKind),
    Voucher(VoucherKind),
}

pub const N_CENTERS: usize = N_JOKERS + N_CONSUMABLES + N_VOUCHERS;

impl Center {
    pub fn index(self) -> usize {
        match self {
            Center::Joker(j) => j.index(),
            Center::Consumable(c) => N_JOKERS + c.index(),
            Center::Voucher(v) => N_JOKERS + N_CONSUMABLES + v.index(),
        }
    }
}

/// A consumable card (in the slots, the shop or a pack).
#[derive(Clone, Debug, PartialEq)]
pub struct Consumable {
    pub kind: ConsumableKind,
    pub sort_id: u64,
    pub edition: Edition,
    /// Extra sell value (Gift Card).
    pub extra_value: i64,
    pub cost: i64,
    pub sell_cost: i64,
}

/// What the creator made.
#[derive(Clone, Debug, PartialEq)]
pub enum Created {
    Joker(Joker),
    Consumable(Consumable),
    /// A playing card, not yet in any area or in the deck list.
    Card(Card),
}

/// The kind asked of the creator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreateKind {
    Joker,
    Tarot,
    Planet,
    Spectral,
    /// A plain playing card.
    Base,
    /// An enhanced playing card.
    Enhanced,
}

/// Where the created card goes (decides the sticker rolls).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dest {
    ShopRow,
    Pack,
    Jokers,
    Consumables,
}

/// The creator's inputs (`create_card`).
#[derive(Clone, Copy, Debug)]
pub struct CreateArgs {
    pub kind: CreateKind,
    pub dest: Dest,
    pub legendary: bool,
    /// A fixed value standing in for the rarity roll.
    pub rarity: Option<f64>,
    pub soulable: bool,
    pub forced: Option<Center>,
    /// The source tag ("append"): `sho`, `buf`, `ar1`, ...
    pub append: &'static str,
}

impl CreateArgs {
    pub fn new(kind: CreateKind, dest: Dest, append: &'static str) -> CreateArgs {
        CreateArgs {
            kind,
            dest,
            legendary: false,
            rarity: None,
            soulable: false,
            forced: None,
            append,
        }
    }
}

fn edition_surcharge(e: Edition) -> i64 {
    match e {
        Edition::Foil => 2,
        Edition::Holographic => 3,
        Edition::Polychrome | Edition::Negative => 5,
        Edition::None => 0,
    }
}

impl Run {
    // ------------------------------------------------------------ present

    pub(crate) fn mark_present(&mut self, c: Center) {
        self.present[c.index()] = true;
    }

    /// A card of `c` was removed: its mark clears unless a card of it is
    /// still in the joker or consumable slots.
    pub(crate) fn card_removed(&mut self, c: Center) {
        let held = match c {
            Center::Joker(k) => self.jokers.iter().any(|j| j.kind == k),
            Center::Consumable(k) => self.consumables.iter().any(|x| x.kind == k),
            Center::Voucher(_) => false,
        };
        if !held {
            self.present[c.index()] = false;
        }
    }

    pub fn is_present(&self, c: Center) -> bool {
        self.present[c.index()]
    }

    fn showman(&self) -> bool {
        self.has_joker(JokerKind::Showman)
    }

    // -------------------------------------------------------------- prices

    /// A price from a base cost (`Card:set_cost`), before any free marking.
    pub fn price(&self, base: i64, edition: Edition) -> i64 {
        let extra = self.inflation + edition_surcharge(edition);
        let p = ((base + extra) as f64 + 0.5) * (100 - self.discount_percent) as f64 / 100.0;
        (p.floor() as i64).max(1)
    }

    /// Recompute a joker's price and sell value.
    pub(crate) fn set_joker_cost(&self, j: &mut Joker) {
        let mut cost = self.price(j.kind.info().cost, j.edition);
        if j.rental {
            cost = 1;
        }
        j.cost = cost;
        j.sell_cost = (cost / 2).max(1) + j.extra_value;
    }

    /// Recompute a consumable's price and sell value.
    pub(crate) fn set_consumable_cost(&self, c: &mut Consumable) {
        let mut cost = self.price(c.kind.base_cost(), c.edition);
        if c.kind.set() == ConsumableSet::Planet && self.has_joker(JokerKind::Astronomer) {
            cost = 0;
        }
        c.cost = cost;
        c.sell_cost = (cost / 2).max(1) + c.extra_value;
    }

    /// A booster's price.
    pub fn booster_price(&self, id: crate::content::BoosterId) -> i64 {
        let b = &BOOSTERS[id as usize];
        if b.kind == crate::content::PackKind::Celestial && self.has_joker(JokerKind::Astronomer) {
            return 0;
        }
        self.price(b.cost, Edition::None)
    }

    /// A shop playing card's price.
    pub fn card_price(&self, c: &Card) -> i64 {
        self.price(1, c.edition)
    }

    /// Every owned card's price and sell value again (after Clearance Sale,
    /// Liquidation, or an Astronomer arriving or leaving).
    pub(crate) fn reprice_owned(&mut self) {
        let mut jokers = std::mem::take(&mut self.jokers);
        for j in jokers.iter_mut() {
            self.set_joker_cost(j);
        }
        self.jokers = jokers;
        let mut cons = std::mem::take(&mut self.consumables);
        for c in cons.iter_mut() {
            self.set_consumable_cost(c);
        }
        self.consumables = cons;
        self.reprice_shop();
    }

    // --------------------------------------------------------------- pools

    /// Is this joker in the pool now (apart from its rarity)?
    fn joker_available(&self, k: JokerKind) -> bool {
        let info = k.info();
        if self.is_present(Center::Joker(k)) && !self.showman() {
            return false;
        }
        if let Some(e) = info.enhancement_gate {
            let any = self
                .playing_cards
                .iter()
                .any(|&id| self.card(id).enhancement == e);
            if !any {
                return false;
            }
        }
        match info.pool_flag {
            Some(PoolFlag::NotIf(_)) if self.gros_michel_extinct => return false,
            Some(PoolFlag::OnlyIf(_)) if !self.gros_michel_extinct => return false,
            _ => {}
        }
        !self.banned_jokers.contains(&k)
    }

    fn consumable_available(&self, k: ConsumableKind) -> bool {
        use ConsumableKind::*;
        if matches!(k, Soul | BlackHole) {
            return false;
        }
        if self.is_present(Center::Consumable(k)) && !self.showman() {
            return false;
        }
        if let Some(h) = k.planet_hand() {
            if matches!(
                h,
                HandType::FiveOfAKind | HandType::FlushHouse | HandType::FlushFive
            ) && self.hand_levels.get(h).played == 0
            {
                return false;
            }
        }
        true
    }

    pub(crate) fn voucher_available(&self, v: VoucherKind) -> bool {
        if self.vouchers[v.index()] {
            return false;
        }
        if v.requires().is_some_and(|r| !self.vouchers[r.index()]) {
            return false;
        }
        if self.is_present(Center::Voucher(v)) && !self.showman() {
            return false;
        }
        if let Some(shop) = self.shop.as_ref() {
            if shop.vouchers.iter().any(|s| s.voucher == v) {
                return false;
            }
        }
        true
    }

    /// Pick from a pool with placeholders (`None`), resampling on
    /// `<key>_resample<n>` while the pick lands on one. An all-placeholder
    /// pool becomes `[fallback]`.
    pub(crate) fn pick_with_resample<T: Copy>(
        &mut self,
        key: &str,
        pool: &[Option<T>],
        fallback: T,
    ) -> T {
        if pool.iter().all(|x| x.is_none()) {
            let mut g = self.rng.generator(key);
            let _ = pick_index(1, &mut g);
            return fallback;
        }
        let mut g = self.rng.generator(key);
        let mut i = pick_index(pool.len(), &mut g);
        let mut it = 1;
        while pool[i].is_none() {
            it += 1;
            let mut g = self.rng.generator(&format!("{key}_resample{it}"));
            i = pick_index(pool.len(), &mut g);
        }
        pool[i].expect("checked")
    }

    /// The next voucher from `key` (the ante's voucher, or a Voucher Tag's).
    pub(crate) fn next_voucher(&mut self, key: &str) -> VoucherKind {
        let pool: Vec<Option<VoucherKind>> = VoucherKind::all()
            .map(|v| self.voucher_available(v).then_some(v))
            .collect();
        self.pick_with_resample(key, &pool, VoucherKind::Blank)
    }

    // ------------------------------------------------------------- editions

    /// `poll_edition`: a roll on `key` against the joker thresholds (`m` 1)
    /// or Standard packs' (`m` 2, no Negative).
    pub(crate) fn poll_edition(&mut self, key: &str, m: f64, no_negative: bool) -> Edition {
        let u = self.rng.random(key);
        let r = self.edition_rate;
        if u > 1.0 - 0.003 * m && !no_negative {
            Edition::Negative
        } else if u > 1.0 - 0.006 * r * m {
            Edition::Polychrome
        } else if u > 1.0 - 0.02 * r * m {
            Edition::Holographic
        } else if u > 1.0 - 0.04 * r * m {
            Edition::Foil
        } else {
            Edition::None
        }
    }

    /// A guaranteed edition (Aura, Wheel of Fortune): never Negative.
    #[allow(clippy::eq_op)] // 1 - 0.04 x 25 is the game's own arithmetic (it is exactly 0)
    pub(crate) fn poll_edition_guaranteed(&mut self, key: &str) -> Edition {
        let u = self.rng.random(key);
        if u > 1.0 - 0.006 * 25.0 {
            Edition::Polychrome
        } else if u > 1.0 - 0.02 * 25.0 {
            Edition::Holographic
        } else if u > 1.0 - 0.04 * 25.0 {
            Edition::Foil
        } else {
            Edition::None
        }
    }

    // -------------------------------------------------------------- creator

    /// Make a joker object of `kind` (its name becomes present; To Do List
    /// picks its hand from `to_do`).
    pub(crate) fn make_joker(&mut self, kind: JokerKind) -> Joker {
        let sort_id = self.take_sort_id();
        let mut j = Joker::new(kind, sort_id);
        if kind == JokerKind::ToDoList {
            j.hand = Some(self.pick_to_do_hand(None));
        }
        if kind == JokerKind::LoyaltyCard {
            j.counter = self.hands_played_run as i64;
        }
        self.mark_present(Center::Joker(kind));
        self.set_joker_cost(&mut j);
        j
    }

    /// To Do List's target: a visible hand other than `old`, in the table's
    /// walk order, from stream `to_do`.
    pub(crate) fn pick_to_do_hand(&mut self, old: Option<HandType>) -> HandType {
        let hands: Vec<HandType> = HAND_WALK_ORDER
            .iter()
            .copied()
            .filter(|&h| self.hand_levels.get(h).visible && Some(h) != old)
            .collect();
        let mut g = self.rng.generator("to_do");
        hands[pick_index(hands.len(), &mut g)]
    }

    pub(crate) fn make_consumable(&mut self, kind: ConsumableKind) -> Consumable {
        let sort_id = self.take_sort_id();
        let mut c = Consumable {
            kind,
            sort_id,
            edition: Edition::None,
            extra_value: 0,
            cost: 0,
            sell_cost: 0,
        };
        self.mark_present(Center::Consumable(kind));
        self.set_consumable_cost(&mut c);
        c
    }

    /// The general card creator (`create_card`).
    pub fn create_card(&mut self, a: CreateArgs) -> Created {
        let ante = self.ante;
        let mut forced = a.forced;
        let mut kind = a.kind;
        // 1. Soul checks.
        if forced.is_none() && a.soulable {
            let name = match kind {
                CreateKind::Tarot => "Tarot",
                CreateKind::Planet => "Planet",
                CreateKind::Spectral => "Spectral",
                _ => "",
            };
            let soul_ok =
                !self.is_present(Center::Consumable(ConsumableKind::Soul)) || self.showman();
            if matches!(kind, CreateKind::Tarot | CreateKind::Spectral)
                && soul_ok
                && self.rng.random(&format!("soul_{name}{ante}")) > 0.997
            {
                forced = Some(Center::Consumable(ConsumableKind::Soul));
            }
            let hole_ok =
                !self.is_present(Center::Consumable(ConsumableKind::BlackHole)) || self.showman();
            if matches!(kind, CreateKind::Planet | CreateKind::Spectral)
                && hole_ok
                && self.rng.random(&format!("soul_{name}{ante}")) > 0.997
            {
                forced = Some(Center::Consumable(ConsumableKind::BlackHole));
            }
        }
        if let Some(f) = forced {
            kind = match f {
                Center::Joker(_) => CreateKind::Joker,
                Center::Consumable(c) => match c.set() {
                    ConsumableSet::Tarot => CreateKind::Tarot,
                    ConsumableSet::Planet => CreateKind::Planet,
                    ConsumableSet::Spectral => CreateKind::Spectral,
                },
                Center::Voucher(_) => unreachable!("vouchers are not made by the creator"),
            };
        }
        match kind {
            CreateKind::Joker => {
                let jk = match forced {
                    Some(Center::Joker(k)) => k,
                    _ => {
                        let rarity = if a.legendary {
                            if a.rarity.is_none() {
                                let _ = self.rng.random(&format!("rarity{ante}{}", a.append));
                            }
                            4
                        } else {
                            let r = match a.rarity {
                                Some(r) => r,
                                None => self.rng.random(&format!("rarity{ante}{}", a.append)),
                            };
                            if r > 0.95 {
                                3
                            } else if r > 0.7 {
                                2
                            } else {
                                1
                            }
                        };
                        let pool: Vec<Option<JokerKind>> = JOKERS
                            .iter()
                            .filter(|j| j.rarity == rarity)
                            .map(|j| self.joker_available(j.kind).then_some(j.kind))
                            .collect();
                        let key = if rarity == 4 {
                            "Joker4".to_string()
                        } else {
                            format!("Joker{rarity}{}{ante}", a.append)
                        };
                        self.pick_with_resample(&key, &pool, JokerKind::Joker)
                    }
                };
                let mut j = self.make_joker(jk);
                if matches!(a.dest, Dest::ShopRow | Dest::Pack) {
                    let (k1, k2) = if a.dest == Dest::Pack {
                        ("packetper", "packssjr")
                    } else {
                        ("etperpoll", "ssjr")
                    };
                    let poll = self.rng.random(&format!("{k1}{ante}"));
                    let stake = self.cfg.stake;
                    if stake >= 4 && poll > 0.7 {
                        if jk.info().eternal_compat {
                            j.eternal = true;
                        }
                    } else if stake >= 7 && poll > 0.4 && poll <= 0.7 && jk.info().perishable_compat
                    {
                        j.perishable = true;
                        j.perish_tally = 5;
                    }
                    if stake >= 8 && self.rng.random(&format!("{k2}{ante}")) > 0.7 {
                        j.rental = true;
                    }
                }
                j.edition = self.poll_edition(&format!("edi{}{ante}", a.append), 1.0, false);
                self.set_joker_cost(&mut j);
                Created::Joker(j)
            }
            CreateKind::Tarot | CreateKind::Planet | CreateKind::Spectral => {
                let ck = match forced {
                    Some(Center::Consumable(c)) => c,
                    _ => {
                        let (set, name, fallback) = match kind {
                            CreateKind::Tarot => {
                                (ConsumableSet::Tarot, "Tarot", ConsumableKind::Strength)
                            }
                            CreateKind::Planet => {
                                (ConsumableSet::Planet, "Planet", ConsumableKind::Pluto)
                            }
                            _ => (
                                ConsumableSet::Spectral,
                                "Spectral",
                                ConsumableKind::Incantation,
                            ),
                        };
                        let pool: Vec<Option<ConsumableKind>> = ConsumableKind::of_set(set)
                            .iter()
                            .map(|&c| self.consumable_available(c).then_some(c))
                            .collect();
                        self.pick_with_resample(
                            &format!("{name}{}{ante}", a.append),
                            &pool,
                            fallback,
                        )
                    }
                };
                Created::Consumable(self.make_consumable(ck))
            }
            CreateKind::Base | CreateKind::Enhanced => {
                let enhancement = if kind == CreateKind::Enhanced {
                    let pool: Vec<Option<Enhancement>> =
                        ENHANCEMENT_POOL.iter().map(|&e| Some(e)).collect();
                    self.pick_with_resample(
                        &format!("Enhanced{}{ante}", a.append),
                        &pool,
                        Enhancement::Bonus,
                    )
                } else {
                    Enhancement::None
                };
                let order = standard_card_order();
                let mut g = self.rng.generator(&format!("front{}{ante}", a.append));
                let (suit, rank) = order[pick_index(order.len(), &mut g)];
                let sort_id = self.take_sort_id();
                let mut c = Card::new(sort_id, suit, rank);
                c.enhancement = enhancement;
                Created::Card(c)
            }
        }
    }
}
