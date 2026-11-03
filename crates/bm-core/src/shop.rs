//! The shop: building it, rerolls, buying, vouchers, selling consumables and
//! leaving (`rules/shop.md`, `rules/vouchers_tags.md`).

use crate::card::{Card, Edition};
use crate::content::{BoosterId, ConsumableKind, ConsumableSet, PackKind, VoucherKind, BOOSTERS};
use crate::create::{Center, Consumable, CreateArgs, CreateKind, Created, Dest};
use crate::joker_data::JokerKind;
use crate::jokers::Joker;
use crate::run::Refused;
use crate::state::{Phase, Run};
use crate::tags::TagKind;

/// What a joker-row slot holds.
#[derive(Clone, Debug, PartialEq)]
pub enum RowItem {
    Joker(Joker),
    Consumable(Consumable),
    Card(Card),
}

#[derive(Clone, Debug, PartialEq)]
pub struct RowSlot {
    pub item: RowItem,
    /// Price now (0 when marked free).
    pub cost: i64,
    /// Free: Coupon Tag, Uncommon/Rare Tag, an edition tag.
    pub couponed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VoucherSlot {
    pub voucher: VoucherKind,
    pub sort_id: u64,
    pub cost: i64,
    /// The ante's voucher (not a Voucher Tag's).
    pub ante_card: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BoosterSlot {
    pub booster: BoosterId,
    pub sort_id: u64,
    pub cost: i64,
    pub couponed: bool,
}

#[derive(Debug, Default, PartialEq)]
pub struct Shop {
    pub row: Vec<RowSlot>,
    pub vouchers: Vec<VoucherSlot>,
    /// The two booster slots; None once opened.
    pub boosters: Vec<Option<BoosterSlot>>,
}

crate::state::clone_by_field!(Shop {
    row,
    vouchers,
    boosters
});

fn refuse<T>(why: &str) -> Result<T, Refused> {
    Err(Refused(why.to_string()))
}

impl Run {
    /// Money that can be spent now (down to the debt floor).
    pub fn spendable(&self) -> i64 {
        self.money - self.bankrupt_at
    }

    pub(crate) fn can_afford(&self, price: i64) -> bool {
        price <= 0 || price <= self.spendable()
    }

    fn joker_room(&self, edition: Edition) -> bool {
        let need = if edition == Edition::Negative { 0 } else { 1 };
        self.jokers.len() as i32 + need <= self.joker_slots
    }

    fn consumable_room(&self, edition: Edition) -> bool {
        let need = if edition == Edition::Negative { 0 } else { 1 };
        self.consumables.len() as i32 + need <= self.consumable_slots
    }

    // -------------------------------------------------------------- reroll

    /// `calculate_reroll_cost`.
    pub(crate) fn calc_reroll_cost(&mut self, skip_increment: bool) {
        if self.free_rerolls < 0 {
            self.free_rerolls = 0;
        }
        if self.free_rerolls > 0 {
            self.reroll_cost = 0;
            return;
        }
        if !skip_increment {
            self.reroll_increase += 1;
        }
        self.reroll_cost = self.reroll_temp.unwrap_or(self.reroll_base) + self.reroll_increase;
    }

    // --------------------------------------------------------------- build

    /// Open a fresh shop (after cash-out).
    pub(crate) fn open_shop(&mut self) {
        self.shop = Some(Shop::default());
        self.phase = Phase::Shop;
        // 1. Shop start: the first D6 Tag.
        if let Some(i) = self.tags.iter().position(|t| t.kind == TagKind::D6) {
            if !self.d6_this_shop {
                self.d6_this_shop = true;
                self.tags.remove(i);
                self.reroll_temp = Some(0);
                self.calc_reroll_cost(true);
            }
        }
        // 2. The joker row.
        for _ in 0..self.joker_max {
            self.build_row_slot();
        }
        // 3. The ante's voucher.
        if let Some(v) = self.shop_voucher {
            self.shop_voucher_shown = true;
            let sort_id = self.take_sort_id();
            self.mark_present(Center::Voucher(v));
            let cost = self.price(v.base_cost(), Edition::None);
            if let Some(s) = self.shop.as_mut() {
                s.vouchers.push(VoucherSlot {
                    voucher: v,
                    sort_id,
                    cost,
                    ante_card: true,
                });
            }
        }
        // 4. Two booster slots.
        for _ in 0..2 {
            let b = self.pick_shop_pack();
            let sort_id = self.take_sort_id();
            let cost = self.booster_price(b);
            if let Some(s) = self.shop.as_mut() {
                s.boosters.push(Some(BoosterSlot {
                    booster: b,
                    sort_id,
                    cost,
                    couponed: false,
                }));
            }
        }
        // 5. Voucher Tags: one extra voucher each.
        let n = self
            .tags
            .iter()
            .filter(|t| t.kind == TagKind::Voucher)
            .count();
        self.tags.retain(|t| t.kind != TagKind::Voucher);
        for _ in 0..n {
            let v = self.next_voucher("Voucher_fromtag");
            let sort_id = self.take_sort_id();
            self.mark_present(Center::Voucher(v));
            let cost = self.price(v.base_cost(), Edition::None);
            if let Some(s) = self.shop.as_mut() {
                s.vouchers.push(VoucherSlot {
                    voucher: v,
                    sort_id,
                    cost,
                    ante_card: false,
                });
            }
        }
        // 6. The first Coupon Tag: the row and the boosters are free.
        if let Some(i) = self.tags.iter().position(|t| t.kind == TagKind::Coupon) {
            self.tags.remove(i);
            if let Some(s) = self.shop.as_mut() {
                for r in s.row.iter_mut() {
                    r.couponed = true;
                    r.cost = 0;
                }
                for b in s.boosters.iter_mut().flatten() {
                    b.couponed = true;
                    b.cost = 0;
                }
            }
        }
    }

    /// The pack for a booster slot (`get_pack`): the first shop pack of the
    /// run is a plain Buffoon Pack with no draw.
    fn pick_shop_pack(&mut self) -> BoosterId {
        if !self.first_shop_buffoon {
            self.first_shop_buffoon = true;
            return crate::content::booster_by_key("p_buffoon_normal_1").expect("known pack");
        }
        let total: f64 = BOOSTERS.iter().map(|b| b.weight).sum();
        let x = self.rng.random(&format!("shop_pack{}", self.ante)) * total;
        let mut sum = 0.0;
        for (i, b) in BOOSTERS.iter().enumerate() {
            sum += b.weight;
            if sum >= x && sum - b.weight <= x {
                return i as BoosterId;
            }
        }
        (BOOSTERS.len() - 1) as BoosterId
    }

    /// Build one joker-row slot (`rules/shop.md` 1.3).
    pub(crate) fn build_row_slot(&mut self) {
        let ante = self.ante;
        // A. Uncommon / Rare Tag.
        let mut tag_card: Option<Joker> = None;
        while let Some(i) = self
            .tags
            .iter()
            .position(|t| matches!(t.kind, TagKind::Uncommon | TagKind::Rare))
        {
            let t = self.tags.remove(i);
            if t.kind == TagKind::Uncommon {
                let mut a = CreateArgs::new(CreateKind::Joker, Dest::ShopRow, "uta");
                a.rarity = Some(0.9);
                if let Created::Joker(j) = self.create_card(a) {
                    tag_card = Some(j);
                }
                break;
            }
            // Rare Tag: needs fewer distinct rares held than the 20 in the pool.
            let mut rares: Vec<JokerKind> = self
                .jokers
                .iter()
                .filter(|j| j.rarity() == 3)
                .map(|j| j.kind)
                .collect();
            rares.sort();
            rares.dedup();
            if rares.len() < 20 {
                let mut a = CreateArgs::new(CreateKind::Joker, Dest::ShopRow, "rta");
                a.rarity = Some(1.0);
                if let Created::Joker(j) = self.create_card(a) {
                    tag_card = Some(j);
                }
                break;
            }
        }
        if let Some(mut j) = tag_card {
            self.apply_edition_tag(&mut j);
            self.set_joker_cost(&mut j);
            if let Some(s) = self.shop.as_mut() {
                s.row.push(RowSlot {
                    item: RowItem::Joker(j),
                    cost: 0,
                    couponed: true,
                });
            }
            return;
        }
        // B. The kind roll.
        let r = self.shop_rates;
        let total = r.joker + r.tarot + r.planet + r.playing_card + r.spectral;
        let x = self.rng.random(&format!("cdt{ante}")) * total;
        let mut playing = CreateKind::Base;
        if self.vouchers[VoucherKind::Illusion.index()] && self.rng.random("illusion") > 0.6 {
            playing = CreateKind::Enhanced;
        }
        let weights = [
            (r.joker, CreateKind::Joker),
            (r.tarot, CreateKind::Tarot),
            (r.planet, CreateKind::Planet),
            (r.playing_card, playing),
            (r.spectral, CreateKind::Spectral),
        ];
        let mut sum = 0.0;
        let mut kind = CreateKind::Joker;
        for (w, k) in weights {
            if x > sum && x <= sum + w {
                kind = k;
                break;
            }
            sum += w;
        }
        // C. Create.
        let created = self.create_card(CreateArgs::new(kind, Dest::ShopRow, "sho"));
        let slot = match created {
            Created::Joker(mut j) => {
                // E. Edition tags (queued step right after the slot).
                let couponed = self.apply_edition_tag(&mut j);
                self.set_joker_cost(&mut j);
                let cost = if couponed { 0 } else { j.cost };
                RowSlot {
                    item: RowItem::Joker(j),
                    cost,
                    couponed,
                }
            }
            Created::Consumable(c) => {
                let cost = c.cost;
                RowSlot {
                    item: RowItem::Consumable(c),
                    cost,
                    couponed: false,
                }
            }
            Created::Card(mut c) => {
                // D. Illusion's edition.
                if self.vouchers[VoucherKind::Illusion.index()] && self.rng.random("illusion") > 0.8
                {
                    let e = self.rng.random("illusion");
                    c.edition = if e > 0.85 {
                        Edition::Polychrome
                    } else if e > 0.5 {
                        Edition::Holographic
                    } else {
                        Edition::Foil
                    };
                }
                let cost = self.card_price(&c);
                RowSlot {
                    item: RowItem::Card(c),
                    cost,
                    couponed: false,
                }
            }
        };
        if let Some(s) = self.shop.as_mut() {
            s.row.push(slot);
        }
    }

    /// Offer a new shop joker to the held edition tags; the first one takes
    /// it. Returns whether the card became free.
    fn apply_edition_tag(&mut self, j: &mut Joker) -> bool {
        if j.edition != Edition::None {
            return false;
        }
        let Some(i) = self.tags.iter().position(|t| {
            matches!(
                t.kind,
                TagKind::Foil | TagKind::Holographic | TagKind::Polychrome | TagKind::Negative
            )
        }) else {
            return false;
        };
        let t = self.tags.remove(i);
        j.edition = match t.kind {
            TagKind::Foil => Edition::Foil,
            TagKind::Holographic => Edition::Holographic,
            TagKind::Polychrome => Edition::Polychrome,
            _ => Edition::Negative,
        };
        true
    }

    /// Prices of the shop's cards again (after a discount change).
    pub(crate) fn reprice_shop(&mut self) {
        let Some(mut shop) = self.shop.take() else {
            return;
        };
        for r in shop.row.iter_mut() {
            let cost = match &mut r.item {
                RowItem::Joker(j) => {
                    self.set_joker_cost(j);
                    j.cost
                }
                RowItem::Consumable(c) => {
                    self.set_consumable_cost(c);
                    c.cost
                }
                RowItem::Card(c) => self.card_price(c),
            };
            r.cost = if r.couponed { 0 } else { cost };
        }
        for v in shop.vouchers.iter_mut() {
            v.cost = self.price(v.voucher.base_cost(), Edition::None);
        }
        for b in shop.boosters.iter_mut().flatten() {
            b.cost = if b.couponed {
                0
            } else {
                self.booster_price(b.booster)
            };
        }
        self.shop = Some(shop);
    }

    // ------------------------------------------------------------- actions

    /// Reroll the joker row.
    pub fn reroll_shop(&mut self) -> Result<(), Refused> {
        if self.phase != Phase::Shop {
            return refuse("not in the shop");
        }
        let price = self.reroll_cost;
        if !(price == 0 || self.spendable() - price >= 0) {
            return refuse("can't afford the reroll");
        }
        self.money -= price;
        if self.free_rerolls > 0 {
            self.free_rerolls -= 1;
            self.calc_reroll_cost(true);
        } else {
            self.calc_reroll_cost(false);
        }
        let old = self
            .shop
            .as_mut()
            .map(|s| std::mem::take(&mut s.row))
            .unwrap_or_default();
        for r in old {
            self.row_item_removed(&r.item);
        }
        for _ in 0..self.joker_max {
            self.build_row_slot();
        }
        self.joker_event_all(crate::scoring::Ctx::RerollShop);
        Ok(())
    }

    pub(crate) fn row_item_removed(&mut self, item: &RowItem) {
        match item {
            RowItem::Joker(j) => self.card_removed(Center::Joker(j.kind)),
            RowItem::Consumable(c) => self.card_removed(Center::Consumable(c.kind)),
            RowItem::Card(_) => {}
        }
    }

    /// Buy the joker-row card at `i` into your slots (or the deck).
    pub fn buy(&mut self, i: usize) -> Result<(), Refused> {
        if self.phase != Phase::Shop {
            return refuse("not in the shop");
        }
        let Some(slot) = self.shop.as_ref().and_then(|s| s.row.get(i)).cloned() else {
            return refuse("no card there");
        };
        if !self.can_afford(slot.cost) {
            return refuse("can't afford it");
        }
        match &slot.item {
            RowItem::Joker(j) if !self.joker_room(j.edition) => {
                return refuse("no joker slot free")
            }
            RowItem::Consumable(c) if !self.consumable_room(c.edition) => {
                return refuse("no consumable slot free")
            }
            _ => {}
        }
        if let Some(s) = self.shop.as_mut() {
            s.row.remove(i);
        }
        match slot.item {
            RowItem::Card(c) => {
                self.cards.push(c);
                let id = (self.cards.len() - 1) as u32;
                self.deck.insert(0, id);
                self.playing_card_added(&[id]);
                self.playing_cards.push(id);
            }
            RowItem::Consumable(c) => {
                if c.edition == Edition::Negative {
                    self.consumable_slots += 1;
                }
                self.consumables.push(c);
            }
            RowItem::Joker(j) => self.push_joker(j),
        }
        if slot.cost != 0 {
            self.money -= slot.cost;
        }
        Ok(())
    }

    /// Buy a consumable from the joker row and use it at once (`targets`:
    /// selected hand positions; the hand is empty in the shop).
    pub fn buy_and_use(&mut self, i: usize) -> Result<(), Refused> {
        if self.phase != Phase::Shop {
            return refuse("not in the shop");
        }
        let Some(slot) = self.shop.as_ref().and_then(|s| s.row.get(i)).cloned() else {
            return refuse("no card there");
        };
        let RowItem::Consumable(c) = slot.item else {
            return refuse("only consumables can be bought and used");
        };
        if !self.can_afford(slot.cost) {
            return refuse("can't afford it");
        }
        if !self.can_use_consumable(&c, &[], crate::consumables::UseFrom::Shop) {
            return refuse("it can't be used now");
        }
        if let Some(s) = self.shop.as_mut() {
            s.row.remove(i);
        }
        if slot.cost != 0 {
            self.money -= slot.cost;
        }
        self.use_consumable_card(c, &[], crate::consumables::UseFrom::Shop);
        Ok(())
    }

    /// Redeem the voucher at `i`.
    pub fn redeem_voucher(&mut self, i: usize) -> Result<(), Refused> {
        if self.phase != Phase::Shop {
            return refuse("not in the shop");
        }
        let Some(v) = self.shop.as_ref().and_then(|s| s.vouchers.get(i)).cloned() else {
            return refuse("no voucher there");
        };
        if v.cost > self.spendable() {
            return refuse("can't afford it");
        }
        if let Some(s) = self.shop.as_mut() {
            s.vouchers.remove(i);
        }
        self.vouchers[v.voucher.index()] = true;
        self.money -= v.cost;
        self.shop_voucher = None;
        self.apply_voucher(v.voucher, true);
        self.card_removed(Center::Voucher(v.voucher));
        Ok(())
    }

    /// A voucher's effect (`Card:apply_to_run`). `bought` is false for a
    /// deck's starting vouchers (no shop to refill).
    pub(crate) fn apply_voucher(&mut self, v: VoucherKind, bought: bool) {
        use VoucherKind as V;
        match v {
            V::Overstock | V::OverstockPlus => {
                self.joker_max += 1;
                if bought && self.shop.is_some() {
                    let have = self.shop.as_ref().map_or(0, |s| s.row.len()) as i32;
                    for _ in have..self.joker_max {
                        self.build_row_slot();
                    }
                }
            }
            V::ClearanceSale => {
                self.discount_percent = 25;
                self.reprice_owned();
            }
            V::Liquidation => {
                self.discount_percent = 50;
                self.reprice_owned();
            }
            V::Hone => self.edition_rate = 2.0,
            V::GlowUp => self.edition_rate = 4.0,
            V::RerollSurplus | V::RerollGlut => {
                self.reroll_base -= 2;
                self.reroll_cost = (self.reroll_cost - 2).max(0);
            }
            V::CrystalBall => self.consumable_slots += 1,
            V::Grabber | V::NachoTong => {
                self.hands_per_round += 1;
                self.hands_left += 1;
            }
            V::Wasteful | V::Recyclomancy => {
                self.discards_per_round += 1;
                self.discards_left += 1;
            }
            V::TarotMerchant => self.shop_rates.tarot = 4.0 * (9.6 / 4.0),
            V::TarotTycoon => self.shop_rates.tarot = 4.0 * (32.0 / 4.0),
            V::PlanetMerchant => self.shop_rates.planet = 4.0 * (9.6 / 4.0),
            V::PlanetTycoon => self.shop_rates.planet = 4.0 * (32.0 / 4.0),
            V::SeedMoney => self.interest_cap = 50,
            V::MoneyTree => self.interest_cap = 100,
            V::Antimatter => self.joker_slots += 1,
            V::MagicTrick | V::Illusion => self.shop_rates.playing_card = 4.0,
            V::Hieroglyph => {
                self.ante -= 1;
                self.hands_per_round -= 1;
                self.hands_left -= 1;
            }
            V::Petroglyph => {
                self.ante -= 1;
                self.discards_per_round -= 1;
                if self.discards_left > 0 {
                    self.discards_left -= 1;
                }
            }
            V::PaintBrush | V::Palette => self.change_hand_size(1),
            V::OmenGlobe
            | V::Telescope
            | V::Observatory
            | V::Blank
            | V::DirectorsCut
            | V::Retcon => {}
        }
    }

    /// Sell the consumable at `i`.
    pub fn sell_consumable(&mut self, i: usize) -> Result<i64, Refused> {
        if self.phase == Phase::GameOver {
            return refuse("the run is over");
        }
        let Some(c) = self.consumables.get(i).cloned() else {
            return refuse("no consumable there");
        };
        self.consumables.remove(i);
        if c.edition == Edition::Negative {
            self.consumable_slots -= 1;
        }
        self.money += c.sell_cost;
        self.joker_event_all(crate::scoring::Ctx::SellingCard);
        self.card_removed(Center::Consumable(c.kind));
        Ok(c.sell_cost)
    }

    /// Open the booster in slot `i` (`packs.rs` takes it from there).
    pub fn open_booster(&mut self, i: usize) -> Result<(), Refused> {
        if self.phase != Phase::Shop {
            return refuse("not in the shop");
        }
        let Some(b) = self
            .shop
            .as_ref()
            .and_then(|s| s.boosters.get(i))
            .cloned()
            .flatten()
        else {
            return refuse("no pack there");
        };
        if !self.can_afford(b.cost) {
            return refuse("can't afford it");
        }
        if let Some(s) = self.shop.as_mut() {
            s.boosters[i] = None;
        }
        if b.cost != 0 {
            self.money -= b.cost;
        }
        self.open_pack(b.booster, Phase::Shop);
        Ok(())
    }

    /// Leave the shop for the next blind select.
    pub(crate) fn close_shop(&mut self) {
        self.joker_event_all(crate::scoring::Ctx::EndingShop);
        if let Some(shop) = self.shop.take() {
            for r in shop.row {
                self.row_item_removed(&r.item);
            }
            for v in shop.vouchers {
                self.card_removed(Center::Voucher(v.voucher));
            }
        }
        self.d6_this_shop = false;
    }

    /// The set of a consumable for creation helpers.
    pub(crate) fn consumable_set_kind(c: ConsumableKind) -> CreateKind {
        match c.set() {
            ConsumableSet::Tarot => CreateKind::Tarot,
            ConsumableSet::Planet => CreateKind::Planet,
            ConsumableSet::Spectral => CreateKind::Spectral,
        }
    }

    /// Is a booster of this kind?
    pub fn booster_kind(id: BoosterId) -> PackKind {
        BOOSTERS[id as usize].kind
    }
}
