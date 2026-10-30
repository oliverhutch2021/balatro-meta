//! The teacher: a rule-based bot stronger than the greedy one. It is the network's warm start (the network
//! first learns to copy it, then improves on it with PPO) and a second number to beat. Like every bot here it
//! decides from the screen and from fair copies only (`observe::fair_copy`), never from the run's future.
//!
//! What it does that the greedy bot doesn't:
//! - values every joker (our own rough judgement: what it adds to a typical run on White Stake, more for
//!   jokers that grow), buys the best one it can, sells its weakest joker for a clearly better one, and
//!   rerolls with money above what it keeps for interest;
//! - keeps $25 (the interest cap) once its joker slots are full, unless something is worth breaking it for;
//!   buys good vouchers;
//! - opens Buffoon packs when it has joker room, Celestial packs for the hands it plays, Arcana packs;
//! - uses Tarots on sensible cards: enhancements on the cards its best hand uses, Steel and Gold on cards
//!   it holds, suit changes towards its main suit, Death to copy its best card over its worst, Hanged Man
//!   on its lowest cards; the money and card-making Tarots and Spectrals at once; sells the ones it can't
//!   use well;
//! - keeps its x-mult jokers at the right end (they multiply what the others add);
//! - plays like the greedy bot, but when no discard is left and even its best hand played with every hand
//!   left can't reach the target, it plays its low cards to dig for a bigger hand.
//!
//! It never makes moves BalatroBot has no call for (buy and use, boss reroll), so its runs stay checkable
//! against the real game.

use crate::bots::{candidate_masks, greedy_move, preview_score};
use crate::card::{Edition, Enhancement, Seal, Suit};
use crate::consumables::UseFrom;
use crate::content::{ConsumableKind, ConsumableSet, PackKind, VoucherKind, BOOSTERS};
use crate::create::Consumable;
use crate::env::MoveKind;
use crate::joker_data::JokerKind;
use crate::observe::fair_copy;
use crate::packs::PackCard;
use crate::poker::HandType;
use crate::shop::RowItem;
use crate::state::{Phase, Run};

/// Money kept for interest ($1 per $5 held, up to the cap) once its joker slots are full: keeping it
/// earlier starves the early build (more runs end at ante 2).
const KEEP_FOR_INTEREST_FROM_JOKERS: usize = 5;
/// A joker in the shop must be worth this much more than the weakest held one to replace it.
const UPGRADE_MARGIN: f64 = 1.5;

/// What a joker is worth to the teacher, 0 to 10.
pub fn joker_value(k: JokerKind) -> f64 {
    use JokerKind::*;
    match k {
        Blueprint | Brainstorm | Triboulet | Yorick | Caino => 9.0,
        Perkeo | Chicot | Cavendish | Constellation => 7.0,
        Baron | Vampire | Campfire | Obelisk | Madness | CardSharp | AncientJoker
        | JokerStencil | Bloodstone | TheDuo | TheTrio | TheTribe | BaseballCard | Hologram
        | Stuntman | RideTheBus | GreenJoker | Supernova | Throwback | SteelJoker | LoyaltyCard => {
            6.0
        }
        DriversLicense | Acrobat | SeeingDouble | FlowerPot | Ramen | Photograph | TheFamily
        | TheOrder | Blackboard | LuckyCat | SquareJoker | Runner | Castle | WeeJoker | Hiker
        | SpareTrousers | Swashbuckler | AbstractJoker | Bull | Bootstraps | BlueJoker
        | Fibonacci | RaisedFist | Dusk | Hack | Mime | SockAndBuskin | GrosMichel | TheIdol => 5.0,
        HitTheRoad | GlassJoker | Burglar | Erosion | FortuneTeller | RedCard | Scholar
        | WalkieTalkie | SmileyFace | ScaryFace | EvenSteven | OddTodd | Banner | MysticSummit
        | Misprint | ShootTheMoon | HalfJoker | IceCream | Popcorn | Seltzer | HangingChad
        | Arrowhead | OnyxAgate | FourFingers | Shortcut | Pareidolia | SmearedJoker
        | OopsAllSixes | CeremonialDagger | InvisibleJoker | DNA | BurntJoker | SpaceJoker
        | Rocket | Cloud9 | FlashCard | MrBones => 4.0,
        Joker | GreedyJoker | LustyJoker | WrathfulJoker | GluttonousJoker | JollyJoker
        | ZanyJoker | MadJoker | CrazyJoker | DrollJoker | SlyJoker | WilyJoker | CleverJoker
        | DeviousJoker | CraftyJoker | RoughGem | Splash | StoneJoker | MarbleJoker
        | GoldenTicket | MidasMask | Luchador | DietCola | Matador | ToDoList | Superposition
        | Seance | RiffRaff | Certificate | Astronomer | Cartomancer | Vagabond | Juggler
        | Drunkard | MerryAndy | TradingCard | ReservedParking | MailInRebate | FacelessJoker
        | ChaosTheClown | DelayedGratification | BusinessCard | GoldenJoker | ToTheMoon
        | GiftCard | Satellite | TurtleBean => 3.0,
        SixthSense | Showman | Hallucination | EightBall | Troubadour | Egg => 2.0,
        CreditCard => 1.0,
    }
}

/// Jokers whose effect multiplies (applied in the joker phase): they belong at the right end.
fn multiplies(k: JokerKind) -> bool {
    use JokerKind::*;
    matches!(
        k,
        Cavendish
            | Hologram
            | Constellation
            | Madness
            | Vampire
            | Campfire
            | Obelisk
            | LuckyCat
            | GlassJoker
            | Throwback
            | SteelJoker
            | CardSharp
            | JokerStencil
            | Acrobat
            | SeeingDouble
            | FlowerPot
            | LoyaltyCard
            | Ramen
            | HitTheRoad
            | DriversLicense
            | Blackboard
            | TheDuo
            | TheTrio
            | TheFamily
            | TheOrder
            | TheTribe
            | Caino
            | Yorick
    )
}

fn edition_bonus(e: Edition) -> f64 {
    match e {
        Edition::None => 0.0,
        Edition::Foil => 0.8,
        Edition::Holographic => 1.2,
        Edition::Polychrome => 2.5,
        Edition::Negative => 2.0, // a free slot
    }
}

/// A joker's worth with its edition.
fn worth(k: JokerKind, e: Edition) -> f64 {
    joker_value(k) + edition_bonus(e)
}

fn voucher_value(v: VoucherKind) -> f64 {
    use VoucherKind::*;
    match v {
        Antimatter => 9.0,
        Grabber | PaintBrush => 7.0,
        NachoTong | Overstock | Wasteful | Palette | Observatory => 6.0,
        ClearanceSale | SeedMoney | Recyclomancy | OverstockPlus => 5.0,
        Liquidation | MoneyTree | Telescope | Hone | GlowUp => 4.0,
        RerollSurplus | RerollGlut | CrystalBall => 3.0,
        _ => 1.0,
    }
}

/// Money the teacher keeps (interest) unless something is worth more.
fn reserve(run: &Run) -> i64 {
    if run.interest_amount > 0 && run.jokers.len() >= KEEP_FOR_INTEREST_FROM_JOKERS {
        run.interest_cap
    } else {
        0
    }
}

fn joker_room(run: &Run, e: Edition) -> bool {
    run.jokers.len() as i32 + i32::from(e != Edition::Negative) <= run.joker_slots
}

fn consumable_room(run: &Run, e: Edition) -> bool {
    run.consumables.len() as i32 + i32::from(e != Edition::Negative) <= run.consumable_slots
}

/// The held joker the teacher would part with first (not Eternal), with its worth.
fn weakest_joker(run: &Run) -> Option<(usize, f64)> {
    run.jokers
        .iter()
        .enumerate()
        .filter(|(_, j)| !j.eternal && !j.face_down)
        .map(|(i, j)| (i, worth(j.kind, j.edition)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

/// The hand type it plays most (by times played, then level); Pair before anything has been played.
fn main_hand(run: &Run) -> HandType {
    HandType::ALL
        .iter()
        .copied()
        .max_by_key(|&h| {
            let i = run.hand_levels.get(h);
            (i.played, i.level)
        })
        .filter(|&h| run.hand_levels.get(h).played > 0)
        .unwrap_or(HandType::Pair)
}

/// The suit most of the deck has.
fn main_suit(run: &Run) -> Suit {
    let mut n = [0u32; 4];
    for &id in &run.playing_cards {
        n[run.card(id).suit as usize] += 1;
    }
    Suit::ALL
        .iter()
        .copied()
        .max_by_key(|&s| (n[s as usize], s as usize))
        .unwrap_or(Suit::Spades)
}

/// The best hand to play now (as the greedy bot judges it): (score, mask).
fn best_play(run: &Run, salt: u64) -> (f64, u64) {
    let fair = fair_copy(run, salt);
    let mut best = (f64::MIN, 1u64);
    for mask in candidate_masks(run.hand.len(), run.forced_card()) {
        let s = preview_score(&fair, mask);
        if s > best.0 || (s == best.0 && mask.count_ones() < best.1.count_ones()) {
            best = (s, mask);
        }
    }
    best
}

fn rank_of(run: &Run, pos: usize) -> u8 {
    run.card(run.hand[pos]).rank.id()
}

/// Hand positions, lowest rank first (`inside`: only those in `mask`, else only those outside it).
fn positions(run: &Run, mask: u64, inside: bool) -> Vec<usize> {
    let mut v: Vec<usize> = (0..run.hand.len())
        .filter(|&i| (mask >> i & 1 == 1) == inside)
        .collect();
    v.sort_by_key(|&i| rank_of(run, i));
    v
}

/// Targets for a consumable that acts on hand cards, or None if it shouldn't be used now.
fn targets(run: &Run, c: &Consumable, best_mask: u64) -> Option<Vec<usize>> {
    use ConsumableKind::*;
    let plain = |i: &usize| {
        let card = run.card(run.hand[*i]);
        card.enhancement == Enhancement::None && !card.face_down
    };
    let (_, hi) = c.kind.selection_limits()?;
    let hi = hi.min(5);
    let mut best_high = positions(run, best_mask, true);
    best_high.reverse(); // highest first
    let out: Vec<usize> = match c.kind {
        // Enhancements that pay when the card scores: on the cards the best hand plays.
        Magician | Empress | Hierophant | Justice | Lovers => {
            best_high.into_iter().filter(plain).take(hi).collect()
        }
        // Steel and Gold pay while the card is held: on the cards it keeps.
        Chariot | Devil => {
            let mut held = positions(run, best_mask, false);
            held.reverse();
            held.into_iter().filter(plain).take(hi).collect()
        }
        // Suit changes towards the deck's main suit.
        Star | Moon | Sun | World => {
            let to = c.kind.tarot_suit()?;
            if to != main_suit(run) {
                return None;
            }
            (0..run.hand.len())
                .filter(|&i| {
                    let card = run.card(run.hand[i]);
                    card.suit != to && card.enhancement != Enhancement::Wild && !card.face_down
                })
                .take(hi)
                .collect()
        }
        // Death: the left card becomes a copy of the right one; worst on the left, best on the right.
        Death => {
            let worst = positions(run, best_mask, false).into_iter().next()?;
            let best = (0..run.hand.len())
                .filter(|&i| i > worst)
                .max_by_key(|&i| rank_of(run, i))?;
            if rank_of(run, best) <= rank_of(run, worst) {
                return None;
            }
            vec![worst, best]
        }
        // Hanged Man: thin the deck of its lowest cards while it is large.
        HangedMan => {
            if run.playing_cards.len() <= 36 {
                return None;
            }
            positions(run, best_mask, false)
                .into_iter()
                .filter(|&i| rank_of(run, i) <= 6)
                .take(hi)
                .collect()
        }
        // Seals and copies on the best card of the best hand.
        Talisman | DejaVu | Trance | Medium | Cryptid => {
            let pick = best_high
                .into_iter()
                .find(|&i| c.kind == Cryptid || run.card(run.hand[i]).seal == Seal::None)?;
            vec![pick]
        }
        _ => return None, // Strength, Tower: not worth it to this bot
    };
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Whether to use a consumable that needs no cards, now.
fn use_freely(run: &Run, c: &Consumable) -> bool {
    use ConsumableKind::*;
    match c.kind {
        k if k.set() == ConsumableSet::Planet => true,
        Hermit | Temperance | BlackHole | WheelOfFortune | Emperor | HighPriestess | Fool
        | Judgement | Soul => true,
        Familiar | Grim | Incantation | Immolate | Sigil => run.phase == Phase::Round,
        Hex => run.jokers.len() == 1,
        Wraith => run.money < 8,
        _ => false, // Ouija, Ectoplasm, Ankh: they cost the run too much
    }
}

/// A consumable to use now: (slot, hand mask), using `best` (lazily) for targets.
fn consumable_to_use(run: &Run, best: &mut dyn FnMut() -> u64) -> Option<(usize, u64)> {
    for (i, c) in run.consumables.iter().enumerate() {
        if c.kind.selection_limits().is_none() {
            if use_freely(run, c) && run.can_use_consumable(c, &[], UseFrom::Slots) {
                return Some((i, 0));
            }
            continue;
        }
        if run.phase != Phase::Round || run.hand.is_empty() {
            continue;
        }
        let Some(t) = targets(run, c, best()) else {
            continue;
        };
        if run.forced_card().is_some_and(|f| !t.contains(&f)) {
            continue; // Cerulean Bell: every selection must include its card
        }
        let ids: Vec<u32> = t.iter().map(|&p| run.hand[p]).collect();
        if run.can_use_consumable(c, &ids, UseFrom::Slots) {
            return Some((i, t.iter().fold(0u64, |m, &p| m | 1 << p)));
        }
    }
    None
}

/// A held consumable this bot will never use well (sold in the shop to free the slot).
fn useless(run: &Run, c: &Consumable) -> bool {
    use ConsumableKind::*;
    match c.kind {
        Strength | Tower | Ouija | Ectoplasm | Ankh => true,
        Star | Moon | Sun | World => c.kind.tarot_suit() != Some(main_suit(run)),
        _ => false,
    }
}

/// Jokers in the order the teacher wants: adders first, multipliers last (Blueprint at the front of the
/// multipliers, so it copies one). One move to get closer, if any.
fn reorder(run: &Run) -> Option<(MoveKind, u64)> {
    let class = |k: JokerKind| -> u8 {
        if k == JokerKind::Blueprint {
            2
        } else if multiplies(k) {
            3
        } else {
            1
        }
    };
    let cur: Vec<u8> = run
        .jokers
        .iter()
        .map(|j| if j.face_down { 0 } else { class(j.kind) })
        .collect();
    if cur.contains(&0) {
        return None; // face down (Amber Acorn): leave them
    }
    let mut want = cur.clone();
    want.sort();
    let at = (0..cur.len()).find(|&i| cur[i] != want[i])?;
    let from = (at + 1..cur.len()).find(|&j| cur[j] == want[at])?;
    Some((MoveKind::MoveJoker, (from | at << 8) as u64))
}

fn shop_move(run: &Run) -> (MoveKind, u64) {
    let Some(shop) = run.shop.as_ref() else {
        return (MoveKind::LeaveShop, 0);
    };
    if let Some(m) = reorder(run) {
        return m;
    }
    if let Some((i, mask)) = consumable_to_use(run, &mut || 0) {
        return (MoveKind::UseConsumable, i as u64 | mask << 8);
    }
    let keep = reserve(run);
    let spare = |cost: i64| run.spendable() - cost >= keep;
    let afford = |cost: i64| cost <= 0 || cost <= run.spendable();
    let weakest = weakest_joker(run);

    // Jokers: the best one worth having, bought now or after selling the weakest held one.
    let mut best_joker: Option<(usize, f64, i64, Edition)> = None;
    for (i, r) in shop.row.iter().enumerate() {
        if let RowItem::Joker(j) = &r.item {
            let w = worth(j.kind, j.edition);
            if best_joker.is_none_or(|b| w > b.1) {
                best_joker = Some((i, w, r.cost, j.edition));
            }
        }
    }
    if let Some((i, w, cost, e)) = best_joker {
        let strong = w >= 6.0 || run.jokers.len() < 2;
        if joker_room(run, e) && afford(cost) && (spare(cost) || strong) && w >= 3.0 {
            return (MoveKind::Buy, i as u64);
        }
        if !joker_room(run, e) {
            if let Some((wi, ww)) = weakest {
                let sell = run.jokers[wi].sell_cost.max(0);
                if w >= ww + UPGRADE_MARGIN
                    && cost <= run.spendable() + sell
                    && (run.spendable() + sell - cost >= keep || strong)
                {
                    return (MoveKind::SellJoker, wi as u64);
                }
            }
        }
    }
    // Planets for the hands it plays, and useful Tarots, with money to spare.
    for (i, r) in shop.row.iter().enumerate() {
        if let RowItem::Consumable(c) = &r.item {
            if !consumable_room(run, c.edition) || !afford(r.cost) || !spare(r.cost) {
                continue;
            }
            let want = match c.kind.set() {
                ConsumableSet::Planet => {
                    c.kind.planet_hand() == Some(main_hand(run))
                        || c.kind == ConsumableKind::BlackHole
                }
                ConsumableSet::Tarot => !useless(run, c),
                ConsumableSet::Spectral => false,
            };
            if want {
                return (MoveKind::Buy, i as u64);
            }
        }
    }
    // Vouchers worth having.
    for (i, v) in shop.vouchers.iter().enumerate() {
        let settled =
            run.jokers.len() >= KEEP_FOR_INTEREST_FROM_JOKERS || run.spendable() - v.cost >= 15;
        if voucher_value(v.voucher) >= 4.0 && afford(v.cost) && spare(v.cost) && settled {
            return (MoveKind::RedeemVoucher, i as u64);
        }
    }
    // Packs: jokers while there's room, planets, tarots.
    let mut pick: Option<(usize, f64)> = None;
    for (i, b) in shop.boosters.iter().enumerate() {
        let Some(b) = b else { continue };
        if !afford(b.cost) || !spare(b.cost) {
            continue;
        }
        let v = match BOOSTERS[b.booster as usize].kind {
            PackKind::Buffoon if joker_room(run, Edition::None) => 5.0,
            PackKind::Buffoon => 2.0,
            PackKind::Celestial => 4.0,
            PackKind::Arcana => 3.0,
            PackKind::Spectral => 2.0,
            PackKind::Standard => 1.0,
        };
        if pick.is_none_or(|p| v > p.1) {
            pick = Some((i, v));
        }
    }
    if let Some((i, v)) = pick {
        if v >= 3.0 {
            return (MoveKind::OpenBooster, i as u64);
        }
    }
    // Sell what it won't use, to make room.
    if let Some(i) = run.consumables.iter().position(|c| useless(run, c)) {
        return (MoveKind::SellConsumable, i as u64);
    }
    // Reroll for a better joker with money to spare.
    let wants_joker = joker_room(run, Edition::None) || weakest.is_some_and(|(_, w)| w < 5.0);
    let cushion = if keep > 0 { 5 } else { 15 };
    if wants_joker
        && run.reroll_cost <= 8
        && spare(run.reroll_cost + cushion)
        && afford(run.reroll_cost)
    {
        return (MoveKind::Reroll, 0);
    }
    (MoveKind::LeaveShop, 0)
}

fn pack_move(run: &Run, salt: u64) -> (MoveKind, u64) {
    let Some(p) = run.pack.as_ref() else {
        return (MoveKind::PackSkip, 0);
    };
    let mut best_mask_cache: Option<u64> = None;
    let mut best_mask = || {
        *best_mask_cache.get_or_insert_with(|| {
            if run.hand.is_empty() {
                0
            } else {
                best_play(run, salt).1
            }
        })
    };
    let mut choice: Option<(usize, u64, f64)> = None;
    for (i, c) in p.cards.iter().enumerate() {
        let (mask, value) = match c {
            PackCard::Joker(j) => {
                if !joker_room(run, j.edition) {
                    continue;
                }
                (0, worth(j.kind, j.edition))
            }
            PackCard::Consumable(c) => {
                if c.kind.set() == ConsumableSet::Planet {
                    let v = if c.kind.planet_hand() == Some(main_hand(run)) {
                        6.0
                    } else {
                        3.0
                    };
                    if !run.can_use_consumable(c, &[], UseFrom::Pack) {
                        continue;
                    }
                    (0, v)
                } else if c.kind.selection_limits().is_none() {
                    if !use_freely(run, c) || !run.can_use_consumable(c, &[], UseFrom::Pack) {
                        continue;
                    }
                    (
                        0,
                        if c.kind == ConsumableKind::Soul {
                            9.0
                        } else {
                            4.0
                        },
                    )
                } else {
                    let Some(t) = targets(run, c, best_mask()) else {
                        continue;
                    };
                    let ids: Vec<u32> = t.iter().map(|&p| run.hand[p]).collect();
                    if !run.can_use_consumable(c, &ids, UseFrom::Pack) {
                        continue;
                    }
                    (t.iter().fold(0u64, |m, &p| m | 1 << p), 4.0)
                }
            }
            PackCard::Card(card) => {
                let v = match (card.enhancement, card.edition, card.seal) {
                    (Enhancement::None, Edition::None, Seal::None) => continue,
                    (_, Edition::Polychrome, _) | (_, _, Seal::Red) => 3.0,
                    _ => 2.0,
                };
                (0, v)
            }
        };
        if choice.is_none_or(|b| value > b.2) {
            choice = Some((i, mask, value));
        }
    }
    match choice {
        Some((i, mask, _)) => (MoveKind::PackChoose, i as u64 | mask << 8),
        None => (MoveKind::PackSkip, 0),
    }
}

fn round_move(run: &Run, salt: u64) -> (MoveKind, u64) {
    let mut cache: Option<(f64, u64)> = None;
    let mut best = || *cache.get_or_insert_with(|| best_play(run, salt));
    let mut mask_only = || best().1;
    if let Some((i, mask)) = consumable_to_use(run, &mut mask_only) {
        return (MoveKind::UseConsumable, i as u64 | mask << 8);
    }
    let (score, mask) = best();
    let target = run.blind.as_ref().map_or(0.0, |b| b.chips);
    let needed = target - run.round_chips;
    if score >= needed || run.hands_left <= 1 {
        return (MoveKind::Play, mask);
    }
    if run.discards_left > 0 {
        // As the greedy bot: discard up to 5 of the lowest cards outside the best hand.
        return match greedy_move(run, salt) {
            (MoveKind::Discard, m) => (MoveKind::Discard, m),
            _ => (MoveKind::Play, mask),
        };
    }
    // No discard left and the best hand falls short. If playing it with every hand left can't reach the
    // target either, dig: play the low cards, to draw new ones for a bigger hand.
    if score * f64::from(run.hands_left) >= needed {
        return (MoveKind::Play, mask);
    }
    let junk: Vec<usize> = positions(run, mask, false).into_iter().take(5).collect();
    let forced = run.forced_card();
    if junk.is_empty() || forced.is_some_and(|f| !junk.contains(&f)) {
        return (MoveKind::Play, mask);
    }
    (MoveKind::Play, junk.iter().fold(0u64, |m, &p| m | 1 << p))
}

/// The teacher's move.
pub fn teacher_move(run: &Run, salt: u64) -> (MoveKind, u64) {
    match run.phase {
        Phase::BlindSelect => (MoveKind::SelectBlind, 0),
        Phase::RoundEval => (MoveKind::CashOut, 0),
        Phase::Shop => shop_move(run),
        Phase::Pack => pack_move(run, salt),
        Phase::GameOver => (MoveKind::SelectBlind, 0),
        Phase::Round => round_move(run, salt),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bm_rng::SeedSampler;
    use crate::env::apply;
    use crate::RunConfig;

    fn play(seed: &str, bot: fn(&Run, u64) -> (MoveKind, u64)) -> (Run, Vec<MoveKind>) {
        let mut run = Run::new(RunConfig::new(seed));
        let mut kinds = Vec::new();
        let mut step = 0u64;
        while run.phase != Phase::GameOver && !run.won && step < 3000 {
            let (k, a) = bot(&run, step);
            assert!(
                apply(&mut run, k, a).is_ok(),
                "{seed}: refused {k:?} {a:#x} in {:?}",
                run.phase
            );
            assert!(
                !matches!(k, MoveKind::BuyAndUse | MoveKind::RerollBoss),
                "a move BalatroBot can't make"
            );
            kinds.push(k);
            step += 1;
        }
        (run, kinds)
    }

    #[test]
    fn teacher_moves_are_always_legal_and_checkable() {
        let mut seeds = SeedSampler::new(4242);
        let mut seen = std::collections::HashSet::new();
        for _ in 0..40 {
            let (_, kinds) = play(&seeds.next_seed(), teacher_move);
            seen.extend(kinds);
        }
        for k in [
            MoveKind::SellJoker,
            MoveKind::Reroll,
            MoveKind::UseConsumable,
            MoveKind::RedeemVoucher,
            MoveKind::OpenBooster,
            MoveKind::PackChoose,
        ] {
            assert!(seen.contains(&k), "the teacher never made a {k:?}");
        }
    }

    #[test]
    fn the_teacher_goes_further_than_the_greedy_bot() {
        let mut seeds = SeedSampler::new(777);
        let (mut t, mut g) = (0i64, 0i64);
        for _ in 0..60 {
            let seed = seeds.next_seed();
            let score = |r: &Run| if r.won { 9 } else { r.ante as i64 };
            t += score(&play(&seed, teacher_move).0);
            g += score(&play(&seed, crate::bots::greedy_move).0);
        }
        assert!(
            t > g,
            "teacher {t} vs greedy {g} (antes summed over the same 60 seeds)"
        );
    }
}
