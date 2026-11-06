//! The teacher: the bot that plays before the network can. It is the network's warm start (the network
//! first learns to copy it, then improves on it with PPO) and a number to beat. Like every bot here it
//! decides from the screen and from fair copies only (`observe::fair_copy`), never from the run's future.
//!
//! It decides by value (`value.rs`): one number for a position, counting scoring power against the blinds
//! ahead (measured by playing the coming rounds out with the run's own deck and jokers), what the run will
//! gain by itself (growing jokers, levels, income, cards made) and money as scoring power for later shops.
//! Nothing below is a fixed rule about what to buy or build: each decision tries its options and keeps the
//! one the value rates highest.
//! - **Shop**: buying (a joker after selling a weaker one when slots are full; Planets used at once, or kept
//!   with Observatory), vouchers, packs (judged on sample contents), rerolls (judged on sample shops and what
//!   they'd buy), using or selling held cards, selling jokers whose sale does something; then leaving with
//!   the jokers set for what happens as the shop closes and the blind starts (a copier on Perkeo or
//!   Cartomancer, a weak joker fed to Ceremonial Dagger...).
//! - **Packs**: each card, each Tarot or Spectral on each target idea (leaning towards the build: the main
//!   rank and suit, face cards when they pay, Death copying the most valuable card), or skipping.
//! - **Blinds**: play it, or skip it for its tag when the run after the skip is clearly worth more.
//! - **Rounds** (planner.rs): a hand that beats the blind is there, so take what the round still offers
//!   while keeping it (throw Mail-In Rebate's rank, Purple Seals, faces for Faceless; play spare cards for
//!   Square Joker, Sixth Sense, DNA, Gold Seals; keep Steel, Gold, Blue Seals and faces for Reserved Parking),
//!   then win; otherwise plan which hand to build, by how often each beats the blind on fair copies, then by
//!   what it adds. Tarots in a round are used when the value says the deck is better for them. Jokers are
//!   set in scoring order before a hand and for what pays before a throw; played cards are ordered to
//!   score most. Bosses: a joker sold to Verdant Leaf, Luchador sold when its boss is hard, five-card hands
//!   for The Psychic, discards first against The Needle.
//! - **Joker order** for scoring: adders first, multipliers last; Blueprint just left of the best joker it
//!   can copy; Brainstorm off the left end with the best joker it can copy leftmost, and Blueprint next to
//!   it, so both copy that joker; Ceremonial Dagger rightmost unless feeding it is worth more.
//!
//! The earlier rule-based teacher (fixed joker values and rules) is still here as the option "classic",
//! for comparisons. It never makes moves BalatroBot has no call for (buy and use, boss reroll), so its
//! runs stay checkable against the real game.

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

/// A list of moves made one after another.
type Moves = Vec<(MoveKind, u64)>;
/// A round move, and whether it is a hand meant to keep the round going.
type Choice = ((MoveKind, u64), bool);
/// A joker order (positions) with the score it gives a hand.
type Scored = (Vec<usize>, f64);

/// Options, each off until switched on by name (the baseline job's `--teacher-options`), for A/B tests on
/// the same seeds: a candidate change is tried behind an option, and becomes the teacher if it wins more.
pub const OPTIONS: &[&str] = &[
    // Trials of the stakes' jokers (`avoided`): never buy or take a joker that is rental (Gold Stake), eternal
    // (Black Stake on) or perishable (Orange Stake on). If a stake's runs go further without them, the value
    // overrates them.
    "avoid-rental",
    "avoid-eternal",
    "avoid-perishable",
    // With a residual stake correction (`baseline --stake-net`): correct the value at the stakes above White
    // only, so White's play stays the value networks' own, move for move.
    "correct-stakes-only",
    // The earlier teacher: fixed joker values and rules in the shop and packs, Tarots on fixed targets,
    // the planner judging rounds by beating the blind only (no value model).
    "classic",
    // Skip blinds for their tags when the value says so: the skip and the blind played out on fair copies to the
    // next ante and the positions reached compared (`value_blind_move`; "skip-margin", "skip-copies",
    // "skip-gate", "skip-x"). The older form, which compared the value right after the skip with the value now,
    // lost (on 400 seeds with round 1's learned value, 1.4 skips a run won 35.75% against 38.5%): the networks had
    // never seen a skipped blind or a held tag and misjudged the position, and the lost shop was only guessed at.
    "skips",
    // Jokers to sell picked by the fixed judgement, not by what the run would lose without each.
    "static-sells",
    // No joker or card moves for events (leaving the shop, throws, the order of played cards).
    "no-juggle",
    // Twice the deals and sample contents behind each decision between rounds (value.rs `Scales`): less
    // noise, at about three times the time (on 400 test runs with the round-1 learned value: 43.0% won
    // against 38.5%, p = 0.12).
    "steady",
    // Half the deals and sample contents (rounded up): about 2.3 times as fast, and weaker (on the same 400
    // runs: 35.5% won, and 0.86 fewer blinds beaten a run).
    "light",
    // Twice the sample contents behind pack and reroll decisions only, the noisiest ones: on 750 shop and
    // pack positions its decisions matched a teacher with four times everything more often than "steady"'s
    // did (68.0% against 66.7%; the usual teacher's 62.9%), at about 1.3 times the usual time; three or four
    // times the contents matched no more often.
    "looks",
    // Twice the tries on fair copies behind each plan for a round (planner.rs): on the same 400 test runs,
    // 42.0% won against 38.5% (p = 0.23), at about 1.1 times the time.
    "steady-rounds",
    // "looks" and "steady-rounds" together: on the same 400 test runs, 45.0% won against 38.5% (p = 0.03), at
    // about 1.45 times the time. The teacher the recordings use (scripts/jobs.py: recording_options).
    "precise",
    // With a joker that pays on the round's final hand (Dusk retriggers its cards, Acrobat x3): when the best
    // hand beats the blind only as the final hand, play single spare cards until one hand is left, then the
    // best; and the value's sample rounds try that way of playing too (the discards dig for the final hand
    // first). Not under the bosses that single cards spoil the final hand for (`play_down_spoiled`). From a
    // player's winning runs, who played rounds down to Dusk on purpose (where the bot would sell Dusk).
    "final-hand",
    // The quick search for a round's best hand (planner.rs `quick_best`: the value's sample rounds, the
    // planner's tries) also tries five cards of one rank, a Five of a Kind or a Flush Five, which copies
    // (Death, DNA, Cryptid) make: without it a deck of copies is judged by its Four of a Kind. From a player's
    // winning run built on Five of a Kind (played nine times).
    "five-kind",
    // A held Tarot or Spectral that changes cards (Death, Cryptid, The Hanged Man, the enhancements, seals,
    // suits, Strength) priced by what it would do to the deck used on the cards the build wants (value.rs
    // `fix_dollars`: the deck's mean card worth gained), never below its usual flat price. From a player's runs,
    // which fix the deck towards its strongest cards (47 to 69 Tarots used a run; Death copying the best card,
    // The Hanged Man clipping the worst), where the bot prices a held Death at $6 whatever the deck.
    "deck-fix",
    // A card's x mult (Glass x2, Polychrome x1.5) counted by the times it scores, retriggers included
    // (value.rs `card_triggers`), in what a card is worth: the cards Death copies, the throws keep, the
    // enhancements go on. A player's view: Glass is the strongest enhancement late, a free win with a retrigger.
    "glass",
    // With Card Sharp (x3 for a hand type already played this round): when the best hand falls short, the planner
    // also tries playing each hand type first and the best hand after it, so a cheap Pair can make the next Pair
    // score three times as much (planner.rs `sharp_primers`). A player's view: Card Sharp "should be a 100%
    // win"; the teacher's runs holding it at the end win 77%.
    "card-sharp",
    // A player's rule, "focus on one hand": his wins pile the Planets on the hand the build plays (Pair to level
    // 27), where the teacher's went only a third to its main hand (on 300 runs: 10 levels a run, its main hand
    // ending its wins at a median level 5; when a Celestial pack offered that hand's Planet it took another or
    // skipped 29% of the time, swayed by the learned value's large, noisy differences between Planets). The run's
    // focus is the hand it has played most (High Card aside, as the run's info shows it), once played four
    // times (`focus_hand`). A Celestial pack's Black Hole, else the focus's Planet, is taken whenever offered;
    // the focus's Planet is bought in the shop and used whenever it leaves $4 (any money from ante 5).
    // The teacher's own play since 2 November (with "fair-skip": +1.64 points on 8,000 seeds, 64.24% against
    // 62.60%, p = 0.001), so the name now changes nothing; "no-focus" switches it off.
    "focus",
    "no-focus",
    // A pack's options judged against the run once the pack is skipped, as each pick is judged once the pack has
    // closed, not against the position in the pack, whose learned value also reads the cards on offer (on 8 test
    // runs it rated a Buffoon pack's position 0.07 above the run after skipping it, so a joker had to beat that
    // too; an Arcana pack's 0.05 below). (A shop's options stay in the shop, so they are judged against it.)
    // The teacher's own play since 2 November, with "focus"; "no-fair-skip" switches it off.
    "fair-skip",
    "no-fair-skip",
];

/// "focus" is on unless "no-focus" is (the teacher's own play since 2 November).
fn focus_on() -> bool {
    !on("no-focus")
}

/// "fair-skip" is on unless "no-fair-skip" is.
fn fair_skip_on() -> bool {
    !on("no-fair-skip")
}

/// Plays of a hand before it can be the run's focus ("focus").
const FOCUS_PLAYS: u32 = 4;
/// Money the focus's Planet must leave in the shop before ante 5 ("focus").
const FOCUS_KEEP: i64 = 4;

/// "focus": the run's focus hand, the hand played most this run (High Card aside; a tie to the higher level),
/// once it has been played `FOCUS_PLAYS` times.
pub(crate) fn focus_hand(run: &Run) -> Option<HandType> {
    HandType::ALL
        .iter()
        .copied()
        .filter(|&h| h != HandType::HighCard)
        .max_by_key(|&h| {
            let i = run.hand_levels.get(h);
            (i.played, i.level, std::cmp::Reverse(h as usize))
        })
        .filter(|&h| run.hand_levels.get(h).played >= FOCUS_PLAYS)
}

/// "commit=p", recordings only (whole-build exploration): with chance `p` a run commits, from ante `COMMIT_ANTE` on
/// (the shop after ante 2's boss: the run has found its feet), to one hand type as its focus, whatever it has played
/// so far: its Planets taken and bought as the focus's are. Before that it plays as usual. The
/// hand and whether the run commits come from the run's own number (the top bits of the salt every decision of a
/// recorded run gets: `examples/baseline.rs`), never from the game, so they say nothing about the run. The
/// recordings then hold whole runs of builds the teacher would not choose, and the learned value sees how they end
/// (round 7's "explore" only takes single moves the value already rates nearly as high, along its own habits).
const COMMIT_ANTE: i32 = 3;
const COMMIT_HANDS: [HandType; 7] = [
    HandType::Pair,
    HandType::TwoPair,
    HandType::ThreeOfAKind,
    HandType::Straight,
    HandType::Flush,
    HandType::FullHouse,
    HandType::FourOfAKind,
];

thread_local! {
    /// The run's commitment once decided (`committed`), forgotten at each run's start (`new_run`).
    static COMMIT: std::cell::Cell<Option<Option<HandType>>> = const { std::cell::Cell::new(None) };
}

/// "commit": the hand this run is committed to, if any (decided at its first decision, kept for the run).
fn committed(salt: u64) -> Option<HandType> {
    let p = param("commit").unwrap_or(0.0);
    if p <= 0.0 {
        return None;
    }
    if let Some(c) = COMMIT.with(|c| c.get()) {
        return c;
    }
    let mut z = (salt >> 20) ^ 0xC0FF_EE00_D15E_A5E5;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    let c = ((z % 10_000) as f64 / 10_000.0 < p)
        .then(|| COMMIT_HANDS[(z / 10_000 % COMMIT_HANDS.len() as u64) as usize]);
    COMMIT.with(|cell| cell.set(Some(c)));
    c
}

/// The hand the focus moves serve: the committed one ("commit", from ante `COMMIT_ANTE`), else the focus
/// (`focus_hand`).
fn run_focus(run: &Run, salt: u64) -> Option<HandType> {
    let c = if run.ante >= COMMIT_ANTE {
        committed(salt)
    } else {
        None
    };
    c.or_else(|| focus_hand(run))
}

/// "focus": the card to take from a Celestial pack: a Black Hole (every hand a level), else the focus's Planet.
fn focus_pick(run: &Run, salt: u64) -> Option<usize> {
    let p = run.pack.as_ref()?;
    if BOOSTERS[p.booster as usize].kind != PackKind::Celestial {
        return None;
    }
    let consumable = |i: usize| match p.cards.get(i) {
        Some(PackCard::Consumable(c)) => Some(c.kind),
        _ => None,
    };
    let n = p.cards.len();
    if let Some(i) = (0..n).find(|&i| consumable(i) == Some(ConsumableKind::BlackHole)) {
        return Some(i);
    }
    let f = run_focus(run, salt)?;
    (0..n).find(|&i| consumable(i).and_then(|k| k.planet_hand()) == Some(f))
}

/// "focus": in the shop, using a held Planet of the focus (unless held Planets pay), else buying one from the
/// row when it leaves `FOCUS_KEEP` (any money from ante 5).
fn focus_shop_move(run: &Run, salt: u64) -> Option<(MoveKind, u64)> {
    let f = run_focus(run, salt)?;
    if !planets_pay_held(run) {
        if let Some(i) = run.consumables.iter().position(|c| {
            c.kind.planet_hand() == Some(f) && run.can_use_consumable(c, &[], UseFrom::Slots)
        }) {
            return Some((MoveKind::UseConsumable, i as u64));
        }
    }
    let shop = run.shop.as_ref()?;
    shop.row
        .iter()
        .enumerate()
        .find_map(|(i, r)| match &r.item {
            RowItem::Consumable(c)
                if c.kind.set() == ConsumableSet::Planet
                    && c.kind.planet_hand() == Some(f)
                    && consumable_room(run, c.edition)
                    && r.cost <= run.spendable()
                    && (run.spendable() - r.cost >= FOCUS_KEEP || run.ante >= 5) =>
            {
                Some((MoveKind::Buy, i as u64))
            }
            _ => None,
        })
}

/// The options in force: the switches on, and the value formula's numbers set (`name=value`, the names in
/// `value::PARAMS`).
#[derive(Clone, Debug, Default)]
struct Set {
    on: Vec<&'static str>,
    params: Vec<(&'static str, f64)>,
}

static ON: std::sync::OnceLock<Set> = std::sync::OnceLock::new();

thread_local! {
    static LOCAL: std::cell::RefCell<Option<Set>> = const { std::cell::RefCell::new(None) };
}

fn known(names: &[String]) -> Result<Set, String> {
    let mut set = Set::default();
    for n in names {
        if let Some((name, value)) = n.split_once('=') {
            let Some(&(k, _)) = crate::value::PARAMS.iter().find(|(k, _)| *k == name) else {
                let names: Vec<&str> = crate::value::PARAMS.iter().map(|(k, _)| *k).collect();
                return Err(format!(
                    "unknown number {name:?} in teacher option {n:?}; known: {}",
                    names.join(", ")
                ));
            };
            match value.parse::<f64>() {
                Ok(v) if v.is_finite() => set.params.push((k, v)),
                _ => return Err(format!("teacher option {n:?} needs a number after '='")),
            }
            continue;
        }
        match OPTIONS.iter().find(|&&k| k == n) {
            Some(&k) => set.on.push(k),
            None => {
                return Err(format!(
                    "unknown teacher option {n:?}; known: {}, or name=number for {}",
                    OPTIONS.join(", "),
                    crate::value::PARAMS
                        .iter()
                        .map(|(k, _)| *k)
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            }
        }
    }
    Ok(set)
}

/// Check options without switching them on (an unknown name, or a number that isn't one, is an error).
pub fn check_options(names: &[String]) -> Result<(), String> {
    known(names).map(|_| ())
}

/// Switch options on for the whole program (once, before any run).
pub fn set_options(names: &[String]) -> Result<(), String> {
    ON.set(known(names)?)
        .map_err(|_| "the teacher's options are already set".to_string())
}

/// The options in force for the whole program, as given (switches, then `name=value` numbers).
pub fn options() -> Vec<String> {
    ON.get().map_or_else(Vec::new, |s| {
        s.on.iter()
            .map(|o| o.to_string())
            .chain(s.params.iter().map(|(k, v)| format!("{k}={v}")))
            .collect()
    })
}

/// Run `f` with these options on this thread only (tests).
pub fn with_options<T>(names: &[&str], f: impl FnOnce() -> T) -> T {
    let names: Vec<String> = names.iter().map(|s| s.to_string()).collect();
    let set = known(&names).expect("known options");
    let before = LOCAL.with(|l| l.replace(Some(set)));
    let out = f();
    LOCAL.with(|l| *l.borrow_mut() = before);
    out
}

/// A number of the value formula as an option set it (`name=value`), if one did.
pub(crate) fn param(name: &str) -> Option<f64> {
    let find = |s: &Set| {
        s.params
            .iter()
            .rev()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| *v)
    };
    if let Some(v) = LOCAL.with(|l| l.borrow().as_ref().map(&find)) {
        return v;
    }
    ON.get().and_then(find)
}

/// Whether an option is switched on.
pub(crate) fn on(name: &str) -> bool {
    if let Some(v) = LOCAL.with(|l| l.borrow().as_ref().map(|s| s.on.contains(&name))) {
        return v;
    }
    ON.get().is_some_and(|s| s.on.contains(&name))
}

/// Whether the teacher decides by the value model (everything but the "classic" option).
pub(crate) fn by_value() -> bool {
    !on("classic")
}

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
        | OopsAllSixes | InvisibleJoker | DNA | BurntJoker | SpaceJoker | Rocket | Cloud9
        | FlashCard | MrBones => 4.0,
        Joker | GreedyJoker | LustyJoker | WrathfulJoker | GluttonousJoker | JollyJoker
        | ZanyJoker | MadJoker | CrazyJoker | DrollJoker | SlyJoker | WilyJoker | CleverJoker
        | DeviousJoker | CraftyJoker | RoughGem | Splash | StoneJoker | MarbleJoker
        | GoldenTicket | MidasMask | Luchador | DietCola | Matador | ToDoList | Superposition
        | Seance | RiffRaff | Certificate | Astronomer | Cartomancer | Vagabond | Juggler
        | Drunkard | MerryAndy | TradingCard | ReservedParking | MailInRebate | FacelessJoker
        | ChaosTheClown | DelayedGratification | BusinessCard | GoldenJoker | ToTheMoon
        | GiftCard | Satellite | TurtleBean => 3.0,
        SixthSense | Showman | Hallucination | EightBall | Troubadour | Egg => 2.0,
        // Ceremonial Dagger destroys the joker to its right at every blind, so the teacher keeps it rightmost,
        // where it never grows. (Madness destroys a random one at every Small and Big Blind, yet valuing it
        // low cost wins: 202 against 230 in 4,000 runs.)
        CeremonialDagger => 2.0,
        CreditCard => 1.0,
    }
}

/// Jokers whose effect multiplies (applied in the joker phase): they belong at the right end.
pub(crate) fn multiplies(k: JokerKind) -> bool {
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

/// Fair copies behind each hand's score when cards in hand lie face down: what they might be, dealt again
/// in each (`observe::fair_copy`), so a hand is judged by its mean score over them.
const FACE_DOWN_DEALS: u64 = 4;

/// The best hand to play now (as the greedy bot judges it): (score, mask).
fn best_play(run: &Run, salt: u64) -> (f64, u64) {
    let deals = if run.hand.iter().any(|&id| run.card(id).face_down) {
        FACE_DOWN_DEALS
    } else {
        1
    };
    let copies: Vec<Run> = (0..deals)
        .map(|k| fair_copy(run, salt.wrapping_add(k.wrapping_mul(0x9E37_79B9))))
        .collect();
    let mut best = (f64::MIN, 1u64);
    for mask in candidate_masks(run.hand.len(), run.forced_card()) {
        let s = copies.iter().map(|c| preview_score(c, mask)).sum::<f64>() / deals as f64;
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

/// The order the teacher wants its jokers in, as their current positions. Jokers score left to right, so
/// adders go first and multipliers last (they multiply what the others add). Then:
/// - Blueprint copies its right neighbour: it goes just left of the best joker it can copy.
/// - Brainstorm copies the leftmost joker (leftmost itself, it copies nothing): it moves off the left end,
///   the best joker it can copy moves there, and every Blueprint goes just left of Brainstorm, copying
///   Brainstorm and so the same best joker. All the copies sit together after the adders, and after the
///   other multipliers too when the copied joker multiplies.
/// - Ceremonial Dagger destroys its right neighbour at every blind: it goes rightmost, where it can't.
///
/// The order depends only on which jokers there are, never on where they stand, so moving towards it ends.
/// Before each hand the teacher measures the order instead (`hand_order`); this one is for deciding, and
/// for the shop.
pub(crate) fn joker_order(run: &Run) -> Vec<usize> {
    use JokerKind::{Blueprint, Brainstorm, CeremonialDagger};
    let j = &run.jokers;
    let all_of = |k: JokerKind| -> Vec<usize> {
        let mut v: Vec<usize> = (0..j.len()).filter(|&i| j[i].kind == k).collect();
        v.sort_by_key(|&i| j[i].sort_id);
        v
    };
    let (prints, storms, daggers) = (
        all_of(Blueprint),
        all_of(Brainstorm),
        all_of(CeremonialDagger),
    );
    let mut rest: Vec<usize> = (0..j.len())
        .filter(|&i| !matches!(j[i].kind, Blueprint | Brainstorm | CeremonialDagger))
        .collect();
    // Adders first, Polychrome ones last among them (their x1.5 comes after their own effect, so it
    // multiplies every adder before them), then multipliers.
    rest.sort_by_key(|&i| {
        (
            multiplies(j[i].kind),
            j[i].edition == Edition::Polychrome,
            j[i].kind as usize,
            j[i].sort_id,
        )
    });
    let value =
        |i: usize| worth(j[i].kind, j[i].edition) + if multiplies(j[i].kind) { 1.0 } else { 0.0 };
    let target = rest
        .iter()
        .copied()
        .filter(|&i| j[i].kind.info().blueprint_compat)
        .max_by(|&a, &b| {
            value(a)
                .total_cmp(&value(b))
                .then(j[b].sort_id.cmp(&j[a].sort_id))
        });
    let mut want = Vec::with_capacity(j.len());
    match target {
        Some(t) if !storms.is_empty() => {
            rest.retain(|&i| i != t);
            let split = if multiplies(j[t].kind) {
                rest.len()
            } else {
                rest.iter()
                    .position(|&i| multiplies(j[i].kind))
                    .unwrap_or(rest.len())
            };
            want.push(t);
            want.extend(&rest[..split]);
            want.extend(&prints);
            want.extend(&storms);
            want.extend(&rest[split..]);
        }
        Some(t) => {
            let at = rest.iter().position(|&i| i == t).unwrap_or(rest.len());
            want.extend(&rest[..at]);
            want.extend(&prints);
            want.extend(&rest[at..]);
        }
        None => {
            want.extend(&rest);
            want.extend(&prints);
            want.extend(&storms);
        }
    }
    want.extend(&daggers);
    want
}

/// One move towards `joker_order`, if the jokers aren't in it yet (and none is face down).
pub(crate) fn reorder(run: &Run) -> Option<(MoveKind, u64)> {
    if run.jokers.len() < 2 || run.jokers.iter().any(|j| j.face_down) {
        return None;
    }
    let want = joker_order(run);
    let at = (0..want.len()).find(|&i| want[i] != i)?;
    Some((MoveKind::MoveJoker, (want[at] | at << 8) as u64))
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

/// Verdant Leaf debuffs every card until a joker is sold: the joker it would miss least goes.
pub(crate) fn verdant_sale(run: &Run) -> Option<(MoveKind, u64)> {
    let leaf = run
        .blind
        .as_ref()
        .is_some_and(|b| b.active_boss() == Some(crate::blinds::Boss::VerdantLeaf));
    if !leaf {
        return None;
    }
    let w = sellable_jokers(run, 1).first().copied()?;
    Some((MoveKind::SellJoker, w as u64))
}

/// The value teacher's use of held cards in a round: Planets at once; any other consumable (on each
/// target idea) when the value model says the run is better for it, judged as if the round began again.
fn value_consumable_in_round(run: &Run, salt: u64) -> Option<(MoveKind, u64)> {
    use crate::value::value_in_round;
    if run.consumables.is_empty() || run.hand.is_empty() {
        return None;
    }
    for (i, c) in run.consumables.iter().enumerate() {
        if c.kind.set() == ConsumableSet::Planet
            && !planets_pay_held(run)
            && run.can_use_consumable(c, &[], UseFrom::Slots)
        {
            return Some((MoveKind::UseConsumable, i as u64));
        }
    }
    let vs = value_salt(run, salt);
    // Each option: its moves, the consumable it uses, and its quick value.
    let mut options: Vec<(Moves, usize, f64)> = Vec::new();
    for (i, c) in run.consumables.iter().enumerate() {
        let use_move = |mask: u64| (MoveKind::UseConsumable, i as u64 | mask << 8);
        if c.kind.selection_limits().is_none() {
            if run.can_use_consumable(c, &[], UseFrom::Slots) {
                options.push((vec![use_move(0)], i, 0.0));
            }
            continue;
        }
        for t in target_sets(run, c) {
            if run.forced_card().is_some_and(|f| !t.contains(&f)) {
                continue; // Cerulean Bell: every selection must include its card
            }
            let ids: Vec<u32> = t.iter().map(|&p| run.hand[p]).collect();
            if run.can_use_consumable(c, &ids, UseFrom::Slots) {
                options.push((use_on(run, c, &t, &use_move), i, 0.0));
            }
        }
    }
    if options.is_empty() {
        return None;
    }
    // A held card's worth stands for using it well later. It is used now when that realises at least half
    // of that worth: the run after using it beats the halfway mark between the card gone unused and the
    // card kept.
    let with_quick = value_in_round(run, vs, crate::value::quick_samples());
    let with_full = std::cell::OnceCell::new();
    let without = |i: usize, samples: usize| {
        let mut c = run.clone();
        c.consumables.remove(i);
        let gone = value_in_round(&c, vs, samples);
        let kept = if samples == crate::value::quick_samples() {
            with_quick
        } else {
            *with_full.get_or_init(|| value_in_round(run, vs, crate::value::samples()))
        };
        gone + 0.5 * (kept - gone).max(0.0)
    };
    let mut base_quick = std::collections::HashMap::new();
    for o in &mut options {
        let b = *base_quick
            .entry(o.1)
            .or_insert_with(|| without(o.1, crate::value::quick_samples()));
        let v = after(run, vs ^ 0x5EED, &o.0).map_or(f64::MIN, |c| {
            value_in_round(&c, vs, crate::value::quick_samples())
        });
        o.2 = v - b;
    }
    options.retain(|o| o.2 > MIN_GAIN);
    options.sort_by(|a, b| b.2.total_cmp(&a.2));
    options.truncate(finalists());
    let mut best = (MIN_GAIN, None);
    let mut base_full = std::collections::HashMap::new();
    for (moves, i, _) in options {
        let b = *base_full
            .entry(i)
            .or_insert_with(|| without(i, crate::value::samples()));
        let v = after(run, vs ^ 0x5EED, &moves).map_or(f64::MIN, |c| {
            value_in_round(&c, vs, crate::value::samples())
        });
        if v - b > best.0 {
            best = (v - b, Some(moves[0]));
        }
    }
    best.1
}

/// A salt that stays the same while only the jokers' order changes, so a decision and the joker moves
/// made for it agree from one call to the next.
fn decision_salt(run: &Run, salt: u64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (
        salt >> 20,
        run.ante,
        run.round,
        run.hands_left,
        run.discards_left,
    )
        .hash(&mut h);
    // The cards in hand as the player sees them (a card face down is only a card).
    let mut cards: Vec<u32> = run
        .hand
        .iter()
        .map(|&id| if run.card(id).face_down { u32::MAX } else { id })
        .collect();
    cards.sort_unstable();
    cards.hash(&mut h);
    run.round_chips.to_bits().hash(&mut h);
    run.consumables.len().hash(&mut h);
    h.finish()
}

/// Before playing `mask`: the order of the played cards that scores most (cards score left to right, so
/// it matters with Hanging Chad, Photograph, Glass and Mult cards...): each card tried first and last.
/// Returns the first card move towards it, if it beats the order in hand by more than a hair.
fn card_order(run: &Run, mask: u64, salt: u64) -> Option<(MoveKind, u64)> {
    if on("no-juggle") {
        return None;
    }
    let played: Vec<usize> = (0..run.hand.len())
        .filter(|&i| mask >> i & 1 == 1)
        .collect();
    if played.len() < 2 || run.hand.iter().any(|&id| run.card(id).face_down) {
        return None;
    }
    // Judged on a fair copy: chance effects (Lucky cards, Misprint...) must not peek at the run's streams.
    let fair = fair_copy(run, salt ^ 0xCA2D);
    let base = crate::bots::preview_score(&fair, mask);
    let mut best = (base * 1.001 + 1.0, None);
    let ids: Vec<u32> = played.iter().map(|&i| run.hand[i]).collect();
    for &p in &played {
        for to in [played[0], played[played.len() - 1]] {
            if p == to {
                continue;
            }
            let mut c = fair.clone();
            if c.move_hand_card(p, to).is_err() {
                continue;
            }
            // The same cards, at their new positions.
            let m = (0..c.hand.len())
                .filter(|&i| ids.contains(&c.hand[i]))
                .fold(0u64, |m, i| m | 1 << i);
            let v = crate::bots::preview_score(&c, m);
            if v > best.0 {
                best = (v, Some((MoveKind::MoveCard, (p | to << 8) as u64)));
            }
        }
    }
    best.1
}

/// Luchador sold at the start of a boss round when the boss makes the blind much harder to beat (its sale
/// disables the boss): judged by the best-hand plan's tries with and without the boss. Only a Luchador in
/// sight: face down (Amber Acorn) the player knows it holds one but not which card it is, and at the round's
/// start nothing has shown it yet.
fn luchador_sale(run: &Run, salt: u64) -> Option<(MoveKind, u64)> {
    run.blind.as_ref().and_then(|b| b.active_boss())?;
    if run.hands_played_round > 0 || run.discards_used_round > 0 {
        return None;
    }
    let l = run
        .jokers
        .iter()
        .position(|j| j.kind == JokerKind::Luchador && !j.debuffed && !j.face_down)?;
    let with_boss = crate::planner::beat_rate(run, salt);
    if with_boss >= 0.9 {
        return None;
    }
    let sold = after(run, salt ^ 0x1C4A, &[(MoveKind::SellJoker, l as u64)])?;
    (crate::planner::beat_rate(&sold, salt) > with_boss + 0.15)
        .then_some((MoveKind::SellJoker, l as u64))
}

/// The value teacher's move in a round: decided with the jokers in scoring order (so the decision doesn't
/// depend on where they stand), then the jokers set for it: for a hand, the scoring order; for a throw,
/// the order that makes it pay most (a copier on Mail-In Rebate, Faceless Joker or Burnt Joker).
fn value_round_move(run: &Run, salt: u64) -> (MoveKind, u64) {
    let salt = decision_salt(run, salt);
    if run.jokers.iter().any(|j| j.face_down) || run.jokers.len() < 2 {
        return round_choice(run, salt).0;
    }
    let scoring = joker_order(run);
    let (m, keep_going) =
        cached_choice(run, salt, || round_choice(&with_order(run, &scoring), salt));
    let want = match m.0 {
        MoveKind::Play if on("no-juggle") => Some(scoring),
        MoveKind::Play => Some(hand_order(run, m.1, salt, keep_going)),
        MoveKind::Discard => Some(discard_order(run, m.1, salt).unwrap_or(scoring)),
        _ => None,
    };
    if let Some(r) = want.and_then(|w| move_towards(run, &w)) {
        return r;
    }
    // Cards set to score the most, unless the hand is meant to score little.
    if m.0 == MoveKind::Play && !keep_going {
        if let Some(r) = card_order(run, m.1, salt) {
            return r;
        }
    }
    // Positions in the decision refer to the jokers in scoring order: map a sale back to the real order.
    if m.0 == MoveKind::SellJoker {
        let real = joker_order(run)[m.1 as usize];
        return (MoveKind::SellJoker, real as u64);
    }
    m
}

thread_local! {
    /// The round decision while the jokers are being moved for it (moving them changes nothing else).
    static CHOICES: std::cell::RefCell<std::collections::HashMap<u64, Choice>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// `decide()`, remembered for this position with the jokers in any order.
fn cached_choice(
    run: &Run,
    salt: u64,
    decide: impl FnOnce() -> ((MoveKind, u64), bool),
) -> ((MoveKind, u64), bool) {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (
        salt,
        run.money,
        run.hands_left,
        run.discards_left,
        run.round,
    )
        .hash(&mut h);
    run.round_chips.to_bits().hash(&mut h);
    run.hand.hash(&mut h);
    for c in &run.consumables {
        (c.kind as u16, c.sort_id).hash(&mut h);
    }
    let mut jokers: Vec<_> = run
        .jokers
        .iter()
        .map(|j| {
            (
                j.sort_id,
                j.kind as u16,
                j.edition as u8,
                j.debuffed,
                j.counter,
                j.mult.to_bits(),
                j.x_mult.to_bits(),
                j.chips.to_bits(),
            )
        })
        .collect();
    jokers.sort_unstable();
    jokers.hash(&mut h);
    let key = h.finish();
    if let Some(c) = CHOICES.with(|c| c.borrow().get(&key).copied()) {
        return c;
    }
    let c = decide();
    CHOICES.with(|m| {
        let mut m = m.borrow_mut();
        if m.len() > 64 {
            m.clear();
        }
        m.insert(key, c);
    });
    c
}

fn round_move(run: &Run, salt: u64) -> (MoveKind, u64) {
    if by_value() {
        value_round_move(run, salt)
    } else {
        round_move_inner(run, salt)
    }
}

fn round_move_inner(run: &Run, salt: u64) -> (MoveKind, u64) {
    round_choice(run, salt).0
}

/// The round move, and whether it is a hand played only for what playing it gives, with the round meant to
/// go on (`finish`'s extra hands): such a hand should score little, so it doesn't end the round early.
fn round_choice(run: &Run, salt: u64) -> ((MoveKind, u64), bool) {
    let plain = |m: (MoveKind, u64)| (m, false);
    if let Some(m) = verdant_sale(run) {
        return plain(m);
    }
    if by_value() {
        if let Some(m) = luchador_sale(run, salt) {
            return plain(m);
        }
    }
    if by_value() {
        if let Some(m) = value_consumable_in_round(run, salt) {
            return plain(m);
        }
    }
    let mut cache: Option<(f64, u64)> = None;
    let mut best = || *cache.get_or_insert_with(|| best_play(run, salt));
    let mut mask_only = || best().1;
    if !by_value() {
        if let Some((i, mask)) = consumable_to_use(run, &mut mask_only) {
            return plain((MoveKind::UseConsumable, i as u64 | mask << 8));
        }
    }
    let (score, mask) = best();
    let target = run.blind.as_ref().map_or(0.0, |b| b.chips);
    let needed = target - run.round_chips;
    // A hand that beats the blind is played at once (the value teacher first takes what the round still
    // offers, if that keeps the win); so is the last hand once the discards are gone. With discards left,
    // even the last hand (The Needle's only one) is worth improving first.
    if score >= needed && by_value() {
        return crate::planner::finish_choice(run, (score, mask), salt);
    }
    if score >= needed || (run.hands_left <= 1 && run.discards_left == 0) {
        return plain((MoveKind::Play, mask));
    }
    if on("final-hand") {
        if let Some(m) = play_down(run, mask, needed, salt) {
            return plain(m);
        }
    }
    if run.forced_card().is_some() && run.discards_left > 0 {
        // Cerulean Bell: as the greedy bot (every selection must hold the boss's card).
        return plain(match greedy_move(run, salt) {
            (MoveKind::Discard, m) => (MoveKind::Discard, m),
            _ => (MoveKind::Play, mask),
        });
    }
    // The best hand now falls short: build towards the hand that beats the blind most often.
    plain(crate::planner::plan(run, (score, mask), salt))
}

// ------------------------------------------------------------------ joker order for each hand

/// The jokers in the order that scores least, the opposite of the scoring order: multipliers (and
/// Polychrome jokers) first, then the adders, Brainstorm leftmost and Blueprint rightmost, where they copy
/// nothing. The planner plays its extra hands this way when it tries them out.
pub(crate) fn low_order(run: &Run) -> Vec<usize> {
    use JokerKind::{Blueprint, Brainstorm};
    let j = &run.jokers;
    let by_id = |v: &mut Vec<usize>| v.sort_by_key(|&i| j[i].sort_id);
    let mut storms: Vec<usize> = (0..j.len()).filter(|&i| j[i].kind == Brainstorm).collect();
    let mut prints: Vec<usize> = (0..j.len()).filter(|&i| j[i].kind == Blueprint).collect();
    by_id(&mut storms);
    by_id(&mut prints);
    let mut rest: Vec<usize> = (0..j.len())
        .filter(|&i| !matches!(j[i].kind, Blueprint | Brainstorm))
        .collect();
    rest.sort_by_key(|&i| {
        (
            !(multiplies(j[i].kind) || j[i].edition == Edition::Polychrome),
            j[i].kind as usize,
            j[i].sort_id,
        )
    });
    storms.extend(rest);
    storms.extend(prints);
    storms
}

/// Most jokers whose every order is tried for a hand (720 orders); with more, a search from the scoring
/// and least-scoring orders, moving one joker at a time while that helps.
const ALL_ORDERS_UP_TO: usize = 6;

/// Every order of the jokers, as permutations of the current positions, listed by the jokers themselves
/// (by creation, never by where they stand now), so the choice stays the same while they are moved to it.
fn all_orders(run: &Run) -> Vec<Vec<usize>> {
    let n = run.jokers.len();
    let mut base: Vec<usize> = (0..n).collect();
    base.sort_by_key(|&i| run.jokers[i].sort_id);
    let mut out = Vec::new();
    let mut p: Vec<usize> = (0..n).collect();
    loop {
        out.push(p.iter().map(|&k| base[k]).collect());
        let Some(i) = (1..n).rev().find(|&i| p[i - 1] < p[i]) else {
            break;
        };
        let j = (i..n).rev().find(|&j| p[j] > p[i - 1]).unwrap_or(i);
        p.swap(i - 1, j);
        p[i..].reverse();
    }
    out
}

/// Joker orders tried for a hand, with their scores: every order (up to six jokers), else those met on a
/// climb from the scoring order (and, when `low`, a descent from the least-scoring order), each step
/// moving the one joker that helps most.
fn scored_orders(
    run: &Run,
    low: bool,
    score: &mut dyn FnMut(&[usize]) -> f64,
) -> Vec<(Vec<usize>, f64)> {
    let n = run.jokers.len();
    if n <= ALL_ORDERS_UP_TO {
        return all_orders(run)
            .into_iter()
            .map(|o| {
                let s = score(&o);
                (o, s)
            })
            .collect();
    }
    let here: Vec<usize> = (0..n).collect();
    let s_here = score(&here);
    let mut out: Vec<(Vec<usize>, f64)> = vec![(here, s_here)];
    let starts = if low {
        vec![(joker_order(run), 1.0), (low_order(run), -1.0)]
    } else {
        vec![(joker_order(run), 1.0)]
    };
    for (start, sign) in starts {
        let s0 = score(&start);
        let mut at = (start, s0);
        out.push(at.clone());
        for _ in 0..4 {
            let mut step: Option<(Vec<usize>, f64)> = None;
            for i in 0..n {
                for to in 0..n {
                    if i == to {
                        continue;
                    }
                    let mut o = at.0.clone();
                    let x = o.remove(i);
                    o.insert(to, x);
                    let s = score(&o);
                    out.push((o.clone(), s));
                    if sign * s > sign * step.as_ref().map_or(at.1, |b| b.1) {
                        step = Some((o, s));
                    }
                }
            }
            match step {
                Some(b) => at = b,
                None => break,
            }
        }
    }
    out
}

/// What each copier copies in `order` (positions in `order`), as joker positions, chains followed: Blueprint
/// its right neighbour, Brainstorm the leftmost joker.
fn copies_in(run: &Run, order: &[usize]) -> Vec<Option<usize>> {
    use JokerKind::{Blueprint, Brainstorm};
    let kind = |p: usize| run.jokers[order[p]].kind;
    let resolve = |mut p: usize| -> Option<usize> {
        for _ in 0..=order.len() {
            let next = match kind(p) {
                Blueprint => p + 1,
                Brainstorm => 0,
                _ => return Some(order[p]),
            };
            if next >= order.len() || next == p || !kind(next).info().blueprint_compat {
                return None;
            }
            p = next;
        }
        None
    };
    (0..order.len())
        .filter(|&p| matches!(kind(p), Blueprint | Brainstorm))
        .map(resolve)
        .collect()
}

thread_local! {
    /// Chosen hand orders (as the jokers' creation ids), while the jokers are being moved to them.
    static HAND_ORDERS: std::cell::RefCell<std::collections::HashMap<u64, Vec<u64>>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Before playing `mask`: the jokers' order for it, measured on a fair copy (every order of up to six
/// jokers). To win, or to score as much as the hand can: the order that scores most (adders before
/// multipliers and Polychrome jokers, copiers on the best joker for this hand, The Duo and Baron where they
/// pay...). For a hand played only for what playing it gives, with the round meant to go on
/// (`keep_going`): an order that scores under what the blind still needs, so the round doesn't end early:
/// the opposite of the scoring order. Among the orders that do the job, the copiers go where this hand pays
/// the run most (Blueprint on Hiker, Space Joker or Business Card...), judged by what playing it adds.
fn hand_order(run: &Run, mask: u64, salt: u64, keep_going: bool) -> Vec<usize> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (
        salt,
        mask,
        keep_going,
        run.round_chips.to_bits(),
        run.hands_left,
    )
        .hash(&mut h);
    run.blind.as_ref().map(|b| b.chips.to_bits()).hash(&mut h);
    run.hand.hash(&mut h);
    let mut ids: Vec<_> = run
        .jokers
        .iter()
        .map(|j| (j.sort_id, j.kind as u16, j.edition as u8, j.debuffed))
        .collect();
    ids.sort_unstable();
    ids.hash(&mut h);
    let key = h.finish();
    let as_positions = |sorts: &[u64]| -> Option<Vec<usize>> {
        sorts
            .iter()
            .map(|s| run.jokers.iter().position(|j| j.sort_id == *s))
            .collect()
    };
    if let Some(v) = HAND_ORDERS.with(|c| c.borrow().get(&key).cloned()) {
        if let Some(o) = as_positions(&v) {
            return o;
        }
    }
    let o = measure_hand_order(run, mask, salt, keep_going);
    let sorts: Vec<u64> = o.iter().map(|&i| run.jokers[i].sort_id).collect();
    HAND_ORDERS.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > 256 {
            c.clear();
        }
        c.insert(key, sorts);
    });
    o
}

/// A winning hand keeps this margin over what the blind needs when its order is chosen for what it pays
/// rather than for its score (chance effects may score less than the copy did).
const WIN_MARGIN: f64 = 1.25;

fn measure_hand_order(run: &Run, mask: u64, salt: u64, keep_going: bool) -> Vec<usize> {
    let fair = fair_copy(run, salt ^ 0x0DE5);
    let needed = run.blind.as_ref().map_or(0.0, |b| b.chips) - run.round_chips;
    let arranged = |o: &[usize]| {
        let mut c = fair.clone();
        c.jokers = o.iter().map(|&i| fair.jokers[i].clone()).collect();
        c
    };
    let mut probe = fair.clone();
    let scored = scored_orders(run, keep_going, &mut |o: &[usize]| {
        probe.jokers = o.iter().map(|&i| fair.jokers[i].clone()).collect();
        crate::bots::preview_score(&probe, mask)
    });
    let top = scored.iter().map(|o| o.1).fold(f64::MIN, f64::max);
    // The orders that do the job: under the target when the round is to go on; to win, over it with a
    // margin (chance effects may score less than the copy did); a hand that can't win scores what it can.
    let ok: Box<dyn Fn(f64) -> bool> = if keep_going {
        Box::new(move |s| s < needed)
    } else if top >= needed * WIN_MARGIN {
        Box::new(move |s| s >= needed * WIN_MARGIN)
    } else {
        Box::new(move |s| s >= top)
    };
    // Better for the hand's purpose: the least score to go on, the most otherwise (the most headroom against
    // chance); on a tie, the first found (the orders are listed by the jokers themselves, so it is the same
    // order wherever they stand).
    let better = |a: &Scored, b: &Scored| {
        let (x, y) = if keep_going { (b.1, a.1) } else { (a.1, b.1) };
        x > y
    };
    let extreme = || {
        let mut out: Option<&Scored> = None;
        for o in &scored {
            if out.is_none_or(|b| better(o, b)) {
                out = Some(o);
            }
        }
        out.map_or_else(|| (0..run.jokers.len()).collect(), |o| o.0.clone())
    };
    // One candidate per way the copiers copy: the best of those that do the job.
    let mut groups: Vec<(Vec<Option<usize>>, &Scored)> = Vec::new();
    for o in scored.iter().filter(|o| ok(o.1)) {
        let k = copies_in(run, &o.0);
        match groups.iter_mut().find(|g| g.0 == k) {
            Some(g) => {
                if better(o, g.1) {
                    g.1 = o;
                }
            }
            None => groups.push((k, o)),
        }
    }
    if groups.len() < 2 {
        return groups.first().map_or_else(extreme, |g| g.1 .0.clone());
    }
    // What playing the hand adds to the run with each (money, cards, jokers grown, levels...).
    let h = crate::bots::preview(&arranged(&groups[0].1 .0), mask);
    let reference = (h.chips.max(1.0), h.mult.max(1.0));
    let mut best: Option<(f64, &Scored)> = None;
    for (_, o) in groups {
        let start = arranged(&o.0);
        let mut end = start.clone();
        if crate::env::apply(&mut end, MoveKind::Play, mask).is_err() {
            continue;
        }
        let g = crate::value::round_gain(&start, &end, reference);
        if best
            .as_ref()
            .is_none_or(|b| g > b.0 + 1e-9 || (g > b.0 - 1e-9 && better(o, b.1)))
        {
            best = Some((g, o));
        }
    }
    best.map_or_else(extreme, |b| b.1 .0.clone())
}

// ------------------------------------------------------------------ joker order by event (option "value")

/// One move towards the order `want` (a permutation of the current joker positions), if not there yet.
fn move_towards(run: &Run, want: &[usize]) -> Option<(MoveKind, u64)> {
    if run.jokers.iter().any(|j| j.face_down) {
        return None;
    }
    let at = (0..want.len()).find(|&i| want[i] != i)?;
    Some((MoveKind::MoveJoker, (want[at] | at << 8) as u64))
}

/// The jokers in the order `want` (positions of the current jokers), on a copy.
fn with_order(run: &Run, want: &[usize]) -> Run {
    let mut c = run.clone();
    c.jokers = want.iter().map(|&i| run.jokers[i].clone()).collect();
    c
}

/// Orders to try for an event: the scoring order, and from it each copier (Blueprint, Brainstorm) set to
/// copy each joker in `events` (Blueprint just left of it; for Brainstorm, that joker leftmost).
fn copy_orders(run: &Run, events: &[JokerKind]) -> Vec<Vec<usize>> {
    use JokerKind::{Blueprint, Brainstorm};
    let base = joker_order(run);
    let mut out = vec![base.clone()];
    let j = &run.jokers;
    let targets: Vec<usize> = (0..j.len())
        .filter(|&i| events.contains(&j[i].kind))
        .collect();
    // Every copier on one event joker: it leftmost (Brainstorm copies it) with each Blueprint chained just
    // left of Brainstorm (Blueprint copying Brainstorm copies it too); with no Brainstorm, the Blueprints
    // chained just left of it.
    let prints: Vec<usize> = (0..j.len()).filter(|&i| j[i].kind == Blueprint).collect();
    let storms: Vec<usize> = (0..j.len()).filter(|&i| j[i].kind == Brainstorm).collect();
    if prints.len() + storms.len() >= 2 {
        for &t in &targets {
            let rest: Vec<usize> = base
                .iter()
                .copied()
                .filter(|i| *i != t && !prints.contains(i) && !storms.contains(i))
                .collect();
            let mut o = Vec::with_capacity(j.len());
            if storms.is_empty() {
                o.extend(&rest);
                o.extend(&prints);
                o.push(t);
            } else {
                o.push(t);
                o.extend(&rest);
                o.extend(&prints);
                o.extend(&storms);
            }
            if !out.contains(&o) {
                out.push(o);
            }
        }
    }
    for c in (0..j.len()).filter(|&i| matches!(j[i].kind, Blueprint | Brainstorm)) {
        for &t in &targets {
            let mut o: Vec<usize> = base.iter().copied().filter(|&i| i != c && i != t).collect();
            if j[c].kind == Blueprint {
                let at = o.len().min(
                    o.iter()
                        .position(|&i| multiplies(j[i].kind))
                        .unwrap_or(o.len()),
                );
                o.insert(at, t);
                o.insert(at, c);
            } else {
                o.insert(0, t);
                o.push(c);
            }
            if !out.contains(&o) {
                out.push(o);
            }
        }
    }
    out
}

/// Jokers that act when the shop is left or the blind starts, worth copying then.
const SHOP_EXIT_EVENTS: &[JokerKind] = &[
    JokerKind::Perkeo,
    JokerKind::Cartomancer,
    JokerKind::RiffRaff,
    JokerKind::MarbleJoker,
    JokerKind::Certificate,
    JokerKind::Burglar,
];

/// Jokers that pay or grow when cards are thrown, worth copying for a discard.
const DISCARD_EVENTS: &[JokerKind] = &[
    JokerKind::MailInRebate,
    JokerKind::FacelessJoker,
    JokerKind::BurntJoker,
];

/// The order to leave the shop in: the scoring order, a copier on a joker that acts as the shop closes or
/// the blind starts (Perkeo, Cartomancer, Riff-raff...), or a weak joker fed to Ceremonial Dagger, whichever
/// the value model rates highest (its play-outs start by leaving the shop and choosing the blind, so these
/// effects happen in them).
fn exit_order(run: &Run, vs: u64) -> Option<Vec<usize>> {
    use crate::value::value;
    if on("no-juggle") {
        return None;
    }
    if run.jokers.len() < 2 || run.jokers.iter().any(|j| j.face_down) {
        return None;
    }
    let mut orders = copy_orders(run, SHOP_EXIT_EVENTS);
    // Ceremonial Dagger fed the weakest joker (put right of it) instead of standing rightmost.
    let base = joker_order(run);
    for d in (0..run.jokers.len()).filter(|&i| run.jokers[i].kind == JokerKind::CeremonialDagger) {
        for f in weakest_jokers(run, vs, 2) {
            if f == d || run.jokers[f].kind == JokerKind::CeremonialDagger {
                continue;
            }
            let mut o: Vec<usize> = base.iter().copied().filter(|&i| i != f).collect();
            let at = o.iter().position(|&i| i == d).map_or(o.len(), |p| p + 1);
            o.insert(at, f);
            if !orders.contains(&o) {
                orders.push(o);
            }
        }
    }
    if orders.len() < 2 {
        return None;
    }
    // The value of each order, plus what leaving and starting the blind make with it (Perkeo's copies,
    // Cartomancer's Tarots: the value's play-outs don't hold on to them).
    let made = |start: &Run| -> f64 {
        let mut c = fair_copy(start, vs ^ 0xE817);
        if crate::env::apply(&mut c, MoveKind::LeaveShop, 0).is_err() {
            return 0.0;
        }
        if c.phase == Phase::BlindSelect {
            let _ = crate::env::apply(&mut c, MoveKind::SelectBlind, 0);
        }
        crate::value::holdings_gain(start, &c)
    };
    let mut best = (f64::MIN, None);
    for (k, o) in orders.into_iter().enumerate() {
        let start = with_order(run, &o);
        let v = value(&start, vs) + made(&start) - if k == 0 { 0.0 } else { MIN_GAIN };
        if v > best.0 {
            best = (v, Some(o));
        }
    }
    best.1
}

/// Before throwing `mask`: the order that makes the throw pay most (a copier on Mail-In Rebate, Faceless
/// Joker or Burnt Joker), judged on fair copies by what the throw adds.
fn discard_order(run: &Run, mask: u64, salt: u64) -> Option<Vec<usize>> {
    if on("no-juggle") {
        return None;
    }
    if run.jokers.len() < 2 || run.jokers.iter().any(|j| j.face_down) {
        return None;
    }
    let orders = copy_orders(run, DISCARD_EVENTS);
    if orders.len() < 2 {
        return None;
    }
    let reference = crate::bots::preview(&fair_copy(run, salt ^ 0xD15C), best_play(run, salt).1);
    let reference = (reference.chips.max(1.0), reference.mult.max(1.0));
    let mut best = (f64::MIN, None);
    for (k, o) in orders.into_iter().enumerate() {
        let start = with_order(run, &o);
        let mut c = fair_copy(&start, salt ^ 0xD15C);
        if crate::env::apply(&mut c, MoveKind::Discard, mask).is_err() {
            continue;
        }
        let g =
            crate::value::round_gain(&start, &c, reference) - if k == 0 { 0.0 } else { MIN_GAIN };
        if g > best.0 {
            best = (g, Some(o));
        }
    }
    best.1
}

// ------------------------------------------------------------------ decisions by value (option "value")

/// The least gain in value worth a move (below it, estimates differ by noise).
const MIN_GAIN: f64 = 0.01;
/// Options judged in full after the quick look (the best-looking ones; `finalists=N` sets another number).
const FINALISTS: usize = 3;

fn finalists() -> usize {
    param("finalists").map_or(FINALISTS, |x| x.round().max(1.0) as usize)
}
/// Sample contents per pack estimate, and sample shops per reroll estimate (as the options "steady" and
/// "light" scale them: `value::Scales`).
const LOOKS: usize = 3;
const REROLL_LOOKS: usize = 3;

fn looks(n: usize) -> u64 {
    crate::value::scale(n, crate::value::scales().looks) as u64
}

/// The salt for value estimates: the same through a shop visit, so its options and its successive
/// decisions see the same deals and agree with each other.
fn value_salt(run: &Run, salt: u64) -> u64 {
    (salt >> 20).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ ((run.round as u64) << 8) ^ run.ante as u64
}

/// The run after these moves, on a fair copy: random outcomes (a reroll's new cards, a pack's contents, a
/// Tarot's luck) come from fresh streams, never the run's own.
fn after(run: &Run, salt: u64, moves: &[(MoveKind, u64)]) -> Option<Run> {
    let mut c = fair_copy(run, salt);
    for &(k, a) in moves {
        crate::env::apply(&mut c, k, a).ok()?;
    }
    Some(c)
}

/// Whether held Planets pay (Observatory: each gives x1.5 to its hand), so a Planet may be kept, not used.
fn planets_pay_held(run: &Run) -> bool {
    run.vouchers[VoucherKind::Observatory.index()]
}

/// Buying this row item; a Planet is used at once (unless held Planets pay).
fn buy_moves(run: &Run, i: usize) -> Vec<(MoveKind, u64)> {
    let mut m = vec![(MoveKind::Buy, i as u64)];
    if let Some(RowItem::Consumable(c)) = run
        .shop
        .as_ref()
        .and_then(|s| s.row.get(i))
        .map(|r| &r.item)
    {
        if c.kind.set() == ConsumableSet::Planet && !planets_pay_held(run) {
            m.push((MoveKind::UseConsumable, run.consumables.len() as u64));
        }
    }
    m
}

/// A consumable in the shop row that needs no targets, bought and used at once (judged beside buying it to
/// hold): Hermit or Temperance for their money, Judgement or The Emperor for what they make...
fn buy_and_use_moves(run: &Run, i: usize) -> Option<Vec<(MoveKind, u64)>> {
    let c = match run.shop.as_ref()?.row.get(i).map(|r| &r.item)? {
        RowItem::Consumable(c) => c,
        _ => return None,
    };
    if c.kind.set() == ConsumableSet::Planet || c.kind.selection_limits().is_some() {
        return None;
    }
    Some(vec![
        (MoveKind::Buy, i as u64),
        (MoveKind::UseConsumable, run.consumables.len() as u64),
    ])
}

/// Held jokers that may go (not Eternal), weakest first by the fixed judgement.
fn sellable_jokers(run: &Run, n: usize) -> Vec<usize> {
    let mut v: Vec<usize> = (0..run.jokers.len())
        .filter(|&i| !run.jokers[i].eternal && !run.jokers[i].face_down)
        .collect();
    v.sort_by(|&a, &b| {
        worth(run.jokers[a].kind, run.jokers[a].edition)
            .total_cmp(&worth(run.jokers[b].kind, run.jokers[b].edition))
    });
    v.truncate(n);
    v
}

thread_local! {
    static WEAKEST: std::cell::RefCell<std::collections::HashMap<(u64, u64), Vec<usize>>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Held jokers that may go, the one the run would miss least first, by the value model: the value of the
/// position without each (judged as if the round began again, in a round). Cached per position and salt.
fn weakest_jokers(run: &Run, vs: u64, n: usize) -> Vec<usize> {
    use std::hash::{Hash, Hasher};
    if on("static-sells") {
        return sellable_jokers(run, n);
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (run.phase as u8, run.ante, run.round).hash(&mut h);
    for j in &run.jokers {
        (
            j.sort_id,
            j.kind as u16,
            j.edition as u8,
            j.mult.to_bits(),
            j.x_mult.to_bits(),
            j.chips.to_bits(),
        )
            .hash(&mut h);
    }
    let key = (h.finish(), vs);
    let cached = WEAKEST.with(|w| w.borrow().get(&key).cloned());
    let order = cached.unwrap_or_else(|| {
        let mut v: Vec<(usize, f64)> = (0..run.jokers.len())
            .filter(|&i| !run.jokers[i].eternal && !run.jokers[i].face_down)
            .map(|i| {
                let mut c = run.clone();
                c.jokers.remove(i);
                // Full estimates: a sale rests on this order, and quick ones are noisy enough to put
                // a strong joker last.
                let without = if run.phase == Phase::Round {
                    crate::value::value_in_round(&c, vs, crate::value::samples())
                } else {
                    crate::value::value(&c, vs)
                };
                (i, without)
            })
            .collect();
        v.sort_by(|a, b| b.1.total_cmp(&a.1));
        let order: Vec<usize> = v.into_iter().map(|(i, _)| i).collect();
        WEAKEST.with(|w| {
            let mut w = w.borrow_mut();
            if w.len() > 256 {
                w.clear();
            }
            w.insert(key, order.clone());
        });
        order
    });
    order.into_iter().take(n).collect()
}

/// What the deck and jokers lean towards: the commonest rank, the suit the deck and suit jokers favour,
/// and whether face cards pay (face jokers).
fn leanings(run: &Run) -> (u8, Suit, bool) {
    use JokerKind::*;
    let mut ranks = [0f64; 15];
    let mut suits = [0f64; 4];
    for &id in &run.playing_cards {
        let k = run.card(id);
        ranks[k.rank.id() as usize] += 1.0;
        suits[k.suit as usize] += 1.0;
    }
    // What the jokers pay for in each rank (a plain card of it).
    for (r, slot) in ranks.iter_mut().enumerate().skip(2) {
        if let Some(rank) = crate::card::Rank::ALL
            .iter()
            .copied()
            .find(|k| k.id() as usize == r)
        {
            let card = crate::card::Card::new(0, Suit::Spades, rank);
            *slot += crate::value::joker_bonus(run, &card) / 4.0;
        }
    }
    let mut faces = false;
    for j in &run.jokers {
        match j.kind {
            LustyJoker | Bloodstone => suits[Suit::Hearts as usize] += 6.0,
            GreedyJoker | RoughGem => suits[Suit::Diamonds as usize] += 6.0,
            WrathfulJoker | Arrowhead => suits[Suit::Spades as usize] += 6.0,
            GluttonousJoker | OnyxAgate => suits[Suit::Clubs as usize] += 6.0,
            Photograph | SmileyFace | ScaryFace | SockAndBuskin | MidasMask | BusinessCard
            | ReservedParking | Pareidolia | Caino | Triboulet => faces = true,
            _ => {}
        }
    }
    let rank = (2..15)
        .max_by(|&a, &b| ranks[a].total_cmp(&ranks[b]).then(a.cmp(&b)))
        .unwrap_or(14) as u8;
    let suit = Suit::ALL
        .iter()
        .copied()
        .max_by(|&a, &b| suits[a as usize].total_cmp(&suits[b as usize]))
        .unwrap_or(Suit::Spades);
    (rank, suit, faces)
}

/// Hand positions to try a card-changing consumable on: several ideas of where it helps, each a candidate
/// the value model judges by what the changed deck then scores. Ideas lean towards the run's build: its
/// commonest rank, its suit, face cards when face jokers pay, the most valuable cards to copy or keep.
/// Each set is given with the moves that use it: Death copies the right card of the two onto the left one,
/// so the hand is first re-ordered when the card to copy is on the left.
fn target_sets(run: &Run, c: &Consumable) -> Vec<Vec<usize>> {
    use ConsumableKind::*;
    let Some((lo, hi)) = c.kind.selection_limits() else {
        return vec![Vec::new()];
    };
    let n = run.hand.len();
    let card = |i: usize| run.card(run.hand[i]);
    let worth = |i: usize| crate::value::card_value(run, run.hand[i]);
    let (main_rank, main_suit, faces) = leanings(run);
    let mut by_worth: Vec<usize> = (0..n).filter(|&i| !card(i).face_down).collect();
    by_worth.sort_by(|&a, &b| worth(a).total_cmp(&worth(b)));
    let low = by_worth.clone();
    let mut high = by_worth.clone();
    high.reverse();
    let plain = |i: &usize| card(*i).enhancement == Enhancement::None;
    let first = |v: &[usize], k: usize| v.iter().copied().take(k).collect::<Vec<usize>>();
    let pick = |pred: &dyn Fn(usize) -> bool, from: &[usize]| -> Vec<usize> {
        from.iter().copied().filter(|&i| pred(i)).collect()
    };
    // Cards a build plays: of the main rank, of the main suit, face cards when they pay.
    let on_rank = pick(&|i| card(i).rank.id() == main_rank, &high);
    let on_suit = pick(&|i| card(i).suit == main_suit, &high);
    let face = pick(&|i| card(i).rank.is_face(), &high);
    let mut sets: Vec<Vec<usize>> = Vec::new();
    let mut add = |mut v: Vec<usize>| {
        v.sort_unstable();
        v.dedup();
        if v.len() >= lo && v.len() <= hi && !sets.contains(&v) {
            sets.push(v);
        }
    };
    let plain_of = |v: &[usize]| v.iter().copied().filter(plain).collect::<Vec<usize>>();
    match c.kind {
        // Enhancements that pay when the card scores: on cards the build plays.
        Magician | Empress | Hierophant | Justice | Lovers => {
            add(first(&plain_of(&high), hi));
            add(first(&plain_of(&on_rank), hi));
            add(first(&plain_of(&on_suit), hi));
            if faces {
                add(first(&plain_of(&face), hi));
            }
        }
        // Steel and Gold pay while held, Stone wherever: on cards the build doesn't play, or its best.
        Chariot | Devil | Tower => {
            add(first(&plain_of(&low), hi));
            add(first(&plain_of(&high), hi));
            add(first(&plain_of(&on_rank), hi));
            if faces {
                add(first(&plain_of(&face), hi));
            }
        }
        // One rank up: towards the main rank, towards face cards, or the highest.
        Strength => {
            add(first(
                &pick(&|i| card(i).rank.id() + 1 == main_rank, &low),
                hi,
            ));
            if faces {
                add(first(
                    &pick(&|i| matches!(card(i).rank.id(), 10..=12), &low),
                    hi,
                ));
            }
            add(first(&pick(&|i| card(i).rank.id() < 14, &high), hi));
            add(first(&low, hi));
        }
        // Thin the deck: its least valuable cards, or cards off the build.
        HangedMan => {
            add(first(&low, 2));
            add(first(&low, 1));
            add(first(&pick(&|i| card(i).suit != main_suit, &low), 2));
            if faces {
                add(first(&pick(&|i| !card(i).rank.is_face(), &low), 2));
            }
            add(first(&pick(&|i| card(i).rank.id() != main_rank, &low), 2));
        }
        // Suit changes: off-suit cards, the least valuable or the best.
        Star | Moon | Sun | World => {
            let to = c.kind.tarot_suit().unwrap_or(main_suit);
            let off = pick(
                &|i| card(i).suit != to && card(i).enhancement != Enhancement::Wild,
                &low,
            );
            add(first(&off, hi));
            let mut off_high = off.clone();
            off_high.reverse();
            add(first(&off_high, hi));
            add(first(&pick(&|i| card(i).rank.id() == main_rank, &off), hi));
        }
        // Death: the least valuable cards become copies of the most valuable, of the main rank or suit, or a
        // face card. (Order in hand is dealt with by `death_moves`.)
        Death => {
            // The deck's commonest exact card in hand too (one strong card copied over and over: The
            // Idol, Flush Five).
            let mut copies: std::collections::HashMap<(u8, u8), usize> =
                std::collections::HashMap::new();
            for &id in &run.playing_cards {
                let k = run.card(id);
                *copies.entry((k.rank.id(), k.suit as u8)).or_insert(0) += 1;
            }
            let commonest = high.iter().copied().max_by_key(|&i| {
                copies
                    .get(&(card(i).rank.id(), card(i).suit as u8))
                    .copied()
                    .unwrap_or(0)
            });
            let sources = [
                high.first().copied(),
                on_rank.first().copied(),
                on_suit
                    .iter()
                    .copied()
                    .find(|&i| card(i).rank.id() == main_rank),
                if faces { face.first().copied() } else { None },
                commonest,
            ];
            for &w in low.iter().take(2) {
                for g in sources.iter().flatten() {
                    if *g != w && worth(*g) > worth(w) {
                        add(vec![w, *g]);
                    }
                }
            }
        }
        // Seals, editions and copies: on the most valuable card, a main-rank card, a face card.
        Talisman | DejaVu | Trance | Medium | Cryptid | Aura => {
            let unsealed = |i: &usize| match c.kind {
                Cryptid => true,
                Aura => card(*i).edition == Edition::None,
                _ => card(*i).seal == Seal::None,
            };
            add(first(
                &high.iter().copied().filter(unsealed).collect::<Vec<_>>(),
                1,
            ));
            add(first(
                &on_rank.iter().copied().filter(unsealed).collect::<Vec<_>>(),
                1,
            ));
            if faces {
                add(first(
                    &face.iter().copied().filter(unsealed).collect::<Vec<_>>(),
                    1,
                ));
            }
            add(first(
                &low.iter().copied().filter(unsealed).collect::<Vec<_>>(),
                1,
            ));
        }
        _ => add(first(&low, hi)),
    }
    // For Death, keep each pair as (to become a copy, to copy): `death_moves` orders the hand.
    if c.kind == Death {
        let mut pairs = Vec::new();
        for &w in low.iter().take(2) {
            for v in &sets {
                if v.contains(&w) {
                    let g = if v[0] == w { v[1] } else { v[0] };
                    if !pairs.contains(&vec![w, g]) {
                        pairs.push(vec![w, g]);
                    }
                }
            }
        }
        return pairs;
    }
    sets
}

/// The moves that use a consumable on `targets` (hand positions). For Death, `targets` is (the card that
/// becomes a copy, the card copied); the game copies the right one onto the left one, so when the card to
/// copy is on the left it is first moved to the right end of the hand. `use_move` makes the consumable
/// move from a mask.
fn use_on(
    run: &Run,
    c: &Consumable,
    targets: &[usize],
    use_move: &dyn Fn(u64) -> (MoveKind, u64),
) -> Vec<(MoveKind, u64)> {
    if c.kind == ConsumableKind::Death && targets.len() == 2 {
        let (w, g) = (targets[0], targets[1]);
        if g > w {
            return vec![use_move(1 << w | 1 << g)];
        }
        let last = run.hand.len() - 1;
        let w2 = if w > g { w - 1 } else { w };
        return vec![
            (MoveKind::MoveCard, (g | last << 8) as u64),
            use_move(1 << w2 | 1 << last),
        ];
    }
    vec![use_move(targets.iter().fold(0u64, |m, &p| m | 1 << p))]
}

/// Every way to take a card from the open pack (Tarots and Spectrals on each target idea), and skipping.
fn pack_options(run: &Run, quick: bool) -> Vec<Vec<(MoveKind, u64)>> {
    let mut out = vec![vec![(MoveKind::PackSkip, 0)]];
    let Some(p) = run.pack.as_ref() else {
        return out;
    };
    for (i, c) in p.cards.iter().enumerate() {
        match c {
            PackCard::Joker(j) => {
                if joker_room(run, j.edition) && !avoided(j) {
                    out.push(vec![(MoveKind::PackChoose, i as u64)]);
                }
            }
            PackCard::Card(_) => out.push(vec![(MoveKind::PackChoose, i as u64)]),
            PackCard::Consumable(cons) => {
                let mut sets = target_sets(run, cons);
                if quick {
                    sets.truncate(2);
                }
                for t in sets {
                    let ids: Vec<u32> = t.iter().map(|&p| run.hand[p]).collect();
                    if run.can_use_consumable(cons, &ids, UseFrom::Pack) {
                        let choose = |mask: u64| (MoveKind::PackChoose, i as u64 | mask << 8);
                        out.push(use_on(run, cons, &t, &choose));
                    }
                }
            }
        }
    }
    out
}

/// A joker the teacher's options say never to buy or take (`avoid-rental`, `avoid-eternal`, `avoid-perishable`).
fn avoided(j: &crate::jokers::Joker) -> bool {
    (j.rental && on("avoid-rental"))
        || (j.eternal && on("avoid-eternal"))
        || (j.perishable && on("avoid-perishable"))
}

/// Ways to give joker slot room to a joker taken by `take`: at once if there's room, else after selling
/// one of the two weakest held jokers.
fn joker_entries(
    run: &Run,
    vs: u64,
    edition: Edition,
    take: (MoveKind, u64),
) -> Vec<Vec<(MoveKind, u64)>> {
    if joker_room(run, edition) {
        vec![vec![take]]
    } else {
        weakest_jokers(run, vs, 2)
            .into_iter()
            .map(|w| vec![(MoveKind::SellJoker, w as u64), take])
            .collect()
    }
}

/// What can be bought in the shop now, each as the moves that do it.
fn shop_buys(run: &Run, vs: u64) -> Vec<Vec<(MoveKind, u64)>> {
    let mut out = Vec::new();
    let Some(shop) = run.shop.as_ref() else {
        return out;
    };
    for (i, r) in shop.row.iter().enumerate() {
        if !(r.cost <= 0 || r.cost <= run.spendable()) {
            continue;
        }
        match &r.item {
            RowItem::Joker(j) if avoided(j) => {}
            RowItem::Joker(j) => {
                out.extend(joker_entries(run, vs, j.edition, (MoveKind::Buy, i as u64)))
            }
            RowItem::Consumable(c) => {
                if consumable_room(run, c.edition) {
                    out.push(buy_moves(run, i));
                    out.extend(buy_and_use_moves(run, i));
                } else if !run.consumables.is_empty() {
                    // Full slots: sell a held one (the last) to make room.
                    let j = run.consumables.len() - 1;
                    let mut m = vec![(MoveKind::SellConsumable, j as u64)];
                    let mut after_sale = run.clone();
                    after_sale.consumables.pop();
                    if let Some(u) = buy_and_use_moves(&after_sale, i) {
                        let mut mu = m.clone();
                        mu.extend(u);
                        out.push(mu);
                    }
                    m.extend(buy_moves(&after_sale, i));
                    out.push(m);
                }
            }
            _ => out.push(buy_moves(run, i)),
        }
    }
    out
}

/// A shop or pack option to judge fully: moves made on a copy, or a move whose outcome is random (a pack's
/// contents, a reroll's cards), judged on sample outcomes, each with the best follow-up found for it.
enum Judge {
    Moves(Vec<(MoveKind, u64)>),
    Sampled(Vec<(Run, Vec<(MoveKind, u64)>)>),
    /// A reroll: sample shops each with its best buy, and the option of rerolling again (`search`) while
    /// the money lasts.
    Rerolled(Vec<(Run, Vec<(MoveKind, u64)>)>, bool),
}

/// The best follow-up among `options` on a sample outcome, by the quick value: (its value, its moves; none
/// if doing nothing is best).
fn best_follow(
    c: &Run,
    vs: u64,
    options: Vec<Vec<(MoveKind, u64)>>,
) -> (f64, Vec<(MoveKind, u64)>) {
    let mut best = (crate::value::quick(c, vs), Vec::new());
    for o in options {
        if let Some(d) = after(c, vs ^ 0x77, &o) {
            let v = crate::value::quick(&d, vs);
            if v > best.0 {
                best = (v, o);
            }
        }
    }
    best
}

/// A sample outcome's full value with its best follow-up taken (if that helps), and without it.
fn sampled_value(c: &Run, follow: &[(MoveKind, u64)], vs: u64) -> (f64, f64) {
    use crate::value::value;
    let bare = value(c, vs);
    let mut v = bare;
    if !follow.is_empty() {
        if let Some(d) = after(c, vs ^ 0x77, follow) {
            v = v.max(value(&d, vs));
        }
    }
    (v, bare)
}

fn here_full(run: &Run, vs: u64) -> f64 {
    crate::value::value(run, vs)
}

/// What rerolling is worth when each sample shop either has something worth buying or is worth rerolling
/// again (at `cost`, in value, each time), while the money lasts (`again`): the value V with
/// V = mean over samples of max(that sample's value, V - cost). A search for something that changes the
/// run is worth more than one look at a shop.
fn search(best: &[f64], cost: f64, again: bool) -> f64 {
    if best.is_empty() {
        return f64::MIN;
    }
    let n = best.len() as f64;
    let mut v = best.iter().sum::<f64>() / n;
    if !again || cost <= 0.0 {
        return v;
    }
    for _ in 0..500 {
        let next = best.iter().map(|&b| b.max(v - cost)).sum::<f64>() / n;
        if (next - v).abs() < 1e-12 {
            break;
        }
        v = next;
    }
    v
}

/// Judge options in two passes: a quick value for each, then the full value for the three best that beat
/// doing nothing. Returns the first move of the winner, or None if nothing beats doing nothing.
/// Doing nothing is judged as the position `base` when given (the run after skipping a pack: "fair-skip"), else
/// as the position here.
fn choose_against(
    run: &Run,
    base: Option<&Run>,
    vs: u64,
    options: Vec<((MoveKind, u64), f64, Judge)>,
) -> Option<(MoveKind, u64)> {
    use crate::value::{quick, value};
    let here = base.unwrap_or(run);
    let here_quick = quick(here, vs);
    let mut options: Vec<_> = options
        .into_iter()
        .filter(|o| o.1 > here_quick + MIN_GAIN)
        .collect();
    if options.is_empty() {
        return None;
    }
    options.sort_by(|a, b| b.1.total_cmp(&a.1));
    options.truncate(finalists());
    let mut best = (value(here, vs) + MIN_GAIN, None);
    let mut judged = vec![best];
    for (first, _, judge) in options {
        let v = match judge {
            Judge::Moves(m) => after(run, vs ^ 0x5EED, &m).map_or(f64::MIN, |c| value(&c, vs)),
            Judge::Sampled(samples) => {
                let n = samples.len().max(1) as f64;
                samples
                    .iter()
                    .map(|(c, follow)| sampled_value(c, follow, vs).0)
                    .sum::<f64>()
                    / n
            }
            Judge::Rerolled(samples, again) => {
                let (best, bare): (Vec<f64>, Vec<f64>) = samples
                    .iter()
                    .map(|(c, follow)| sampled_value(c, follow, vs))
                    .unzip();
                let cost = here_full(run, vs) - bare.iter().sum::<f64>() / bare.len().max(1) as f64;
                search(&best, cost, again)
            }
        };
        judged.push((v, Some(first)));
        if v > best.0 {
            best = (v, Some(first));
        }
    }
    explore(run, vs, &judged, best).1
}

/// "explore=p" (for recordings, never for measuring): with chance p, a decision between options takes another
/// that the value rates nearly as high (within "explore-within" of the best, doing nothing included) in place of
/// the best. The learned value learns only from the positions its recordings reach: without this, never from
/// what a choice the teacher doesn't make leads to (a run that piles its Planets on one hand, an engine of copied
/// cards), so each round can only refine the teacher's own habits. The chance comes from a hash of the decision,
/// so a run explores the same way on any thread.
fn explore(
    run: &Run,
    vs: u64,
    judged: &[(f64, Option<(MoveKind, u64)>)],
    best: (f64, Option<(MoveKind, u64)>),
) -> (f64, Option<(MoveKind, u64)>) {
    let p = param("explore").unwrap_or(0.0);
    if p <= 0.0 {
        return best;
    }
    let within = param("explore-within").unwrap_or(EXPLORE_WITHIN);
    let others: Vec<_> = judged
        .iter()
        .copied()
        .filter(|&(v, m)| m != best.1 && v > f64::MIN && v >= best.0 - within)
        .collect();
    if others.is_empty() {
        return best;
    }
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (
        vs,
        run.ante,
        run.round,
        run.money,
        run.phase as u8,
        run.jokers.len(),
        run.consumables.len(),
    )
        .hash(&mut h);
    (
        run.reroll_cost,
        run.playing_cards.len(),
        format!("{:?}", judged.iter().map(|j| j.1).collect::<Vec<_>>()),
    )
        .hash(&mut h);
    let r = h.finish();
    if (r % 1_000_000) as f64 >= p * 1_000_000.0 {
        return best;
    }
    others[((r >> 32) % others.len() as u64) as usize]
}

/// How far below the best an option may be to be explored ("explore-within", in the value's units: log chance).
const EXPLORE_WITHIN: f64 = 0.25;

/// Options given as move lists, with their quick values.
fn judged_moves(
    run: &Run,
    vs: u64,
    lists: Vec<Vec<(MoveKind, u64)>>,
) -> Vec<((MoveKind, u64), f64, Judge)> {
    lists
        .into_iter()
        .filter_map(|m| {
            let c = after(run, vs ^ 0x5EED, &m)?;
            Some((m[0], crate::value::quick(&c, vs), Judge::Moves(m)))
        })
        .collect()
}

fn value_pack_move(run: &Run, salt: u64) -> (MoveKind, u64) {
    if focus_on() {
        if let Some(i) = focus_pick(run, salt) {
            return (MoveKind::PackChoose, i as u64);
        }
    }
    let vs = value_salt(run, salt);
    let lists: Vec<_> = pack_options(run, false)
        .into_iter()
        .filter(|m| m[0].0 != MoveKind::PackSkip)
        .collect();
    let options = judged_moves(run, vs, lists);
    // A pick that leaves the pack open (the first of a Mega pack's two) stays judged against the pack.
    let last_pick = run.pack.as_ref().is_some_and(|p| p.choices_left <= 1);
    let skipped = (fair_skip_on() && last_pick)
        .then(|| after(run, vs ^ 0x5EED, &[(MoveKind::PackSkip, 0)]))
        .flatten();
    choose_against(run, skipped.as_ref(), vs, options).unwrap_or((MoveKind::PackSkip, 0))
}

thread_local! {
    /// The shop's choice to leave, with the jokers' order to leave in (by creation ids), remembered while
    /// the jokers are moved into it: moving them changes nothing else in the shop.
    static LEAVING: std::cell::RefCell<std::collections::HashMap<u64, Vec<u64>>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// A key for a shop position with the jokers in any order.
fn shop_key(run: &Run, vs: u64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (vs, run.ante, run.round, run.money, run.reroll_cost).hash(&mut h);
    format!("{:?}", run.shop).hash(&mut h);
    let mut jokers: Vec<_> = run
        .jokers
        .iter()
        .map(|j| {
            (
                j.sort_id,
                j.kind as u16,
                j.edition as u8,
                j.debuffed,
                j.counter,
                j.sell_cost,
                j.mult.to_bits(),
                j.x_mult.to_bits(),
                j.chips.to_bits(),
            )
        })
        .collect();
    jokers.sort_unstable();
    jokers.hash(&mut h);
    for c in &run.consumables {
        (c.sort_id, c.kind as u16, c.edition as u8).hash(&mut h);
    }
    (run.playing_cards.len(), run.tags.len(), &run.vouchers).hash(&mut h);
    h.finish()
}

/// The shop move that raises the value most: buying (a Planet used at once; a joker after selling a weaker
/// one when slots are full), vouchers, packs (judged by sample contents), a reroll (judged by sample shops,
/// with what it would buy in each), using or selling held cards, or leaving.
fn value_shop_move(run: &Run, salt: u64) -> (MoveKind, u64) {
    let Some(shop) = run.shop.as_ref() else {
        return (MoveKind::LeaveShop, 0);
    };
    let vs = value_salt(run, salt);
    // Already decided to leave: go on setting the jokers for it.
    let key = shop_key(run, vs);
    let leaving = LEAVING
        .with(|c| c.borrow().get(&key).cloned())
        .and_then(|ids| {
            ids.iter()
                .map(|id| run.jokers.iter().position(|j| j.sort_id == *id))
                .collect::<Option<Vec<usize>>>()
        });
    if let Some(want) = leaving {
        return move_towards(run, &want).unwrap_or((MoveKind::LeaveShop, 0));
    }
    if focus_on() {
        if let Some(m) = focus_shop_move(run, salt) {
            return m;
        }
    }
    let options = shop_option_list(run, shop, vs);
    if let Some(m) = choose_against(run, None, vs, options) {
        return m;
    }
    // Leaving: first set the jokers for what happens as the shop closes and the blind starts (or for
    // scoring, when nothing happens then).
    let want = exit_order(run, vs).unwrap_or_else(|| joker_order(run));
    let Some(m) = move_towards(run, &want) else {
        return (MoveKind::LeaveShop, 0);
    };
    let ids: Vec<u64> = want.iter().map(|&i| run.jokers[i].sort_id).collect();
    LEAVING.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > 64 {
            c.clear();
        }
        c.insert(key, ids);
    });
    m
}

/// A shop's options with their quick values, as `value_shop_move` weighs them: buys, vouchers, held consumables
/// used or sold, sales, packs (by sample contents) and a reroll (by sample shops).
fn shop_option_list(
    run: &Run,
    shop: &crate::shop::Shop,
    vs: u64,
) -> Vec<((MoveKind, u64), f64, Judge)> {
    let afford = |cost: i64| cost <= 0 || cost <= run.spendable();
    let mut lists = shop_buys(run, vs);
    for (i, v) in shop.vouchers.iter().enumerate() {
        if afford(v.cost) {
            lists.push(vec![(MoveKind::RedeemVoucher, i as u64)]);
        }
    }
    for (i, c) in run.consumables.iter().enumerate() {
        if c.kind.selection_limits().is_none() && run.can_use_consumable(c, &[], UseFrom::Slots) {
            lists.push(vec![(MoveKind::UseConsumable, i as u64)]);
        }
        lists.push(vec![(MoveKind::SellConsumable, i as u64)]);
    }
    // Selling: the weakest joker, and jokers whose sale does something (Invisible Joker, Diet Cola).
    let mut sells = weakest_jokers(run, vs, 1);
    for (i, j) in run.jokers.iter().enumerate() {
        let acts = matches!(j.kind, JokerKind::InvisibleJoker | JokerKind::DietCola);
        if acts && !j.eternal && !sells.contains(&i) {
            sells.push(i);
        }
    }
    for w in sells {
        lists.push(vec![(MoveKind::SellJoker, w as u64)]);
    }
    let mut options = judged_moves(run, vs, lists);
    // Packs, by sample contents and the best pick in each.
    for (b, slot) in shop.boosters.iter().enumerate() {
        let Some(pk) = slot else { continue };
        if !afford(pk.cost) {
            continue;
        }
        let mut samples = Vec::new();
        let mut q = 0.0;
        for s in 0..looks(LOOKS) {
            if let Some(c) = after(
                run,
                vs ^ (0xB00 + 16 * b as u64 + s),
                &[(MoveKind::OpenBooster, b as u64)],
            ) {
                let picks = pack_options(&c, true)
                    .into_iter()
                    .filter(|m| m[0].0 != MoveKind::PackSkip)
                    .collect();
                let (v, follow) = best_follow(&c, vs, picks);
                q += v;
                samples.push((c, follow));
            }
        }
        if !samples.is_empty() {
            let n = samples.len() as f64;
            options.push((
                (MoveKind::OpenBooster, b as u64),
                q / n,
                Judge::Sampled(samples),
            ));
        }
    }
    // A reroll, by sample shops and the best buy in each (when something could still be bought after it).
    if afford(run.reroll_cost) && run.spendable() - run.reroll_cost >= 3 {
        let mut samples = Vec::new();
        let (mut best, mut bare) = (Vec::new(), Vec::new());
        for s in 0..looks(REROLL_LOOKS) {
            if let Some(c) = after(run, vs ^ (0xA11 + s), &[(MoveKind::Reroll, 0)]) {
                let (v, follow) = best_follow(&c, vs, shop_buys(&c, vs));
                best.push(v);
                bare.push(crate::value::quick(&c, vs));
                samples.push((c, follow));
            }
        }
        if !samples.is_empty() {
            // Another reroll after this one, while the money allows it (its cost goes up by one).
            let again = run.spendable() - 2 * run.reroll_cost > 3;
            let cost = crate::value::quick(run, vs) - bare.iter().sum::<f64>() / bare.len() as f64;
            let q = search(&best, cost, again);
            options.push(((MoveKind::Reroll, 0), q, Judge::Rerolled(samples, again)));
        }
    }
    options
}

/// The options the teacher would weigh here, each as its moves, the best first by its quick value: the `n` best of
/// a shop's (a pack or a reroll as its one move: its contents come when it is opened) or of an open pack's, then
/// doing nothing (leaving the shop, skipping the pack). Elsewhere, none. Nothing hidden is used; what the options
/// lead to is then found by playing each out on fair copies (`forks`).
pub fn candidates(run: &Run, salt: u64, n: usize) -> Vec<Vec<(MoveKind, u64)>> {
    let vs = value_salt(run, salt);
    let (judged, nothing) = match run.phase {
        Phase::Shop => {
            let Some(shop) = run.shop.as_ref() else {
                return Vec::new();
            };
            (shop_option_list(run, shop, vs), (MoveKind::LeaveShop, 0))
        }
        Phase::Pack => {
            let lists = pack_options(run, false)
                .into_iter()
                .filter(|m| m[0].0 != MoveKind::PackSkip)
                .collect();
            (judged_moves(run, vs, lists), (MoveKind::PackSkip, 0))
        }
        _ => return Vec::new(),
    };
    let mut judged: Vec<_> = judged.into_iter().filter(|o| o.1.is_finite()).collect();
    judged.sort_by(|a, b| b.1.total_cmp(&a.1));
    judged.truncate(n);
    let mut out: Vec<Vec<(MoveKind, u64)>> = judged
        .into_iter()
        .map(|(first, _, judge)| match judge {
            Judge::Moves(m) => m,
            Judge::Sampled(_) | Judge::Rerolled(..) => vec![first],
        })
        .collect();
    out.push(vec![nothing]);
    out
}

/// The least a skip must gain, in value, over playing the blind ("skip-margin": a skip gives up the blind's
/// reward and a shop and faces the next blind unprepared, so a close call plays).
const SKIP_MARGIN: f64 = 0.05;
/// Fair copies a skip is judged on ("skip-copies"), each played both ways to the same checkpoint.
const SKIP_COPIES: f64 = 4.0;
/// The play-outs' shop counts (the quick look and the looks), as a share of the usual ("skip-x"); the deals and
/// the rounds' tries stay the usual. At a third of the usual all round the teacher wins 12% of runs instead of
/// 56%, and its lost rounds and poor buys drown what the skip changed; halving the shop screening alone halves the
/// time and keeps the play.
const SKIP_X: f64 = 0.5;
/// What a round lost inside a play-out takes off the value the branch had at that round's start ("skip-loss"):
/// the value there already holds the blind's chance, so the loss counts once more, not as the end of the world.
const SKIP_LOSS: f64 = 2.0;
/// How many standard errors of the copies' gains the mean gain must also clear ("skip-z"): the copies' futures
/// differ (what their shops sold), and a skip taken on that noise is a blind given away. With four copies the
/// error is itself rough (three degrees of freedom), so 2.5 lets through about one noise case in thirty.
const SKIP_Z: f64 = 2.5;
/// What a run lost inside a play-out scores before any blind choice was valued (a refused first move).
const SKIP_LOST: f64 = -20.0;
/// What a run won inside a play-out scores (above any living position's value, but not so far above that one
/// win among the copies decides alone).
const SKIP_WON: f64 = 20.0;
/// Moves a play-out may take before it is given up (a stuck run counts as lost).
const SKIP_MAX_STEPS: u64 = 600;

thread_local! {
    /// How deep in skip play-outs this thread is: inside one, every blind is played (no forking within a fork,
    /// and no recursion).
    static SKIP_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Tags never worth a blind: a few dollars, a reroll, a hand size for one round. The rest are weighed.
fn tag_worth_weighing(t: crate::tags::TagKind) -> bool {
    use crate::tags::TagKind::*;
    !matches!(t, Juggle | D6 | Boss | Skip | Garbage)
}

/// Where a skip's play-outs stop, so that both ways reach the same place with the tag's work done: the ante and
/// whether at its Boss Blind's choice (true) or the next ante's first choice (false). A tag that acts in the next
/// shop (a free or edition joker, a voucher, a coupon) is spent by the Boss choice when the Small Blind is
/// skipped (the Big Blind's shop comes first), but only after the boss when the Big Blind is; one that acts at
/// once (a pack, money, levels) has acted by the Boss choice either way; Investment pays after the boss and a
/// Double tag waits for the next tag, so those go to the next ante.
fn skip_checkpoint(run: &Run) -> (i32, bool) {
    use crate::blinds::BlindSlot;
    use crate::tags::TagKind::*;
    let slot = run.on_deck();
    let tag = run.tags_offered[slot.index()];
    let at_once = matches!(
        tag,
        Standard
            | Charm
            | Meteor
            | Buffoon
            | Ethereal
            | Handy
            | Garbage
            | TopUp
            | Skip
            | Orbital
            | Economy
            | Boss
    );
    let next_shop = matches!(
        tag,
        Uncommon | Rare | Negative | Foil | Holographic | Polychrome | Voucher | Coupon | D6
    );
    // At the last ante there is no next ante: the Boss choice it is, where the value holds the boss's chance.
    let boss_choice =
        at_once || (next_shop && slot == BlindSlot::Small) || run.ante >= run.win_ante;
    (run.ante, boss_choice)
}

/// Whether a play-out has reached the checkpoint (`skip_checkpoint`).
fn at_checkpoint(run: &Run, (ante, boss_choice): (i32, bool)) -> bool {
    use crate::blinds::BlindSlot;
    run.phase == Phase::BlindSelect
        && (run.ante > ante
            || (boss_choice && run.ante == ante && run.on_deck() == BlindSlot::Boss))
}

/// At the blind choice: play the blind, or (option "skips") skip it for its tag. A skip is judged like any other
/// move, by what it leads to: on fair copies of the run (fresh hidden futures), a light teacher plays both ways,
/// the blind skipped and the blind played, to the same checkpoint (`skip_checkpoint`: the Boss Blind's choice, or
/// the next ante's first, once the tag has done its work), and the positions reached are valued, the lost shop
/// and reward and the tag's gift counted by having happened. The skip wins when it gains "skip-margin" on
/// average over "skip-copies" copies. A few tags are never worth a blind (`tag_worth_weighing`), and the Boss
/// Blind can't be skipped. The learned term at the checkpoint reads a proxy screen (`value::value_settled`): no
/// recording holds a skipped blind or a held tag, so the networks are shown the skipped blind as beaten and the
/// tags as spent, and judge the run by what it holds, which is what differs between the two ways.
fn value_blind_move(run: &Run, salt: u64) -> (MoveKind, u64) {
    let play = (MoveKind::SelectBlind, 0);
    let slot = run.on_deck();
    if !on("skips")
        || slot == crate::blinds::BlindSlot::Boss
        || SKIP_DEPTH.with(|d| d.get()) > 0
        || !tag_worth_weighing(run.tags_offered[slot.index()])
    {
        return play;
    }
    let vs = value_salt(run, salt);
    let checkpoint = skip_checkpoint(run);
    let copies = param("skip-copies").unwrap_or(SKIP_COPIES).max(1.0) as usize;
    // The run's value here, for a branch that loses a round before any blind choice of its own.
    let here = crate::value::value_settled(run, vs);
    let mut gains: Vec<f64> = Vec::with_capacity(copies);
    for i in 0..copies as u64 {
        // One copy first: when it leans against the skip, as most do, the rest are spared (which leans the mean
        // of the rest a little the skip's way; "skip-z" is set with that in mind).
        if i == 1 && gains[0] <= 0.0 {
            break;
        }
        let copy = fair_copy(run, vs ^ (0x5C1B_0000 + i));
        let a = play_to(
            copy.clone(),
            (MoveKind::SkipBlind, 0),
            vs ^ (i << 8),
            checkpoint,
            here,
        );
        let b = play_to(copy, play, vs ^ (i << 8) ^ 1, checkpoint, here);
        gains.push(a - b);
    }
    let copies = gains.len();
    let n = copies as f64;
    let gain = gains.iter().sum::<f64>() / n;
    let se = if copies > 1 {
        (gains.iter().map(|g| (g - gain).powi(2)).sum::<f64>() / (n - 1.0) / n).sqrt()
    } else {
        0.0
    };
    let needed =
        param("skip-margin").unwrap_or(SKIP_MARGIN) + param("skip-z").unwrap_or(SKIP_Z) * se;
    if std::env::var_os("BM_SKIP_DEBUG").is_some() {
        eprintln!(
            "skip? ante {} {:?} tag {:?}: play-outs gain {:.3} (se {:.3}, needed {:.3}){}",
            run.ante,
            slot,
            run.tags_offered[slot.index()],
            gain,
            se,
            needed,
            if gain > needed { " SKIP" } else { "" }
        );
    }
    if gain > needed {
        (MoveKind::SkipBlind, 0)
    } else {
        play
    }
}

/// A run played by the teacher from `first` to the checkpoint (or its end), and the value of the position
/// reached (`value_settled`): for a run lost or stuck, the value at the last blind choice it passed (`here`
/// before any) less "skip-loss"; `SKIP_WON` for one won. Inside the play-out no blind is skipped, and the counts
/// are the usual ones times "skip-x", whatever the outer decision's.
fn play_to(
    mut run: Run,
    first: (MoveKind, u64),
    salt: u64,
    checkpoint: (i32, bool),
    here: f64,
) -> f64 {
    use crate::value::{value_settled, Scales};
    if crate::env::apply(&mut run, first.0, first.1).is_err() {
        return SKIP_LOST;
    }
    let x = param("skip-x").unwrap_or(SKIP_X).max(0.05);
    let light = Scales {
        full: Scales::USUAL.full,
        quick: Scales::USUAL.quick * x,
        looks: Scales::USUAL.looks * x,
        rounds: Scales::USUAL.rounds,
    };
    let loss = param("skip-loss").unwrap_or(SKIP_LOSS);
    SKIP_DEPTH.with(|d| d.set(d.get() + 1));
    let out = crate::value::with_scales(light, || {
        let mut step = 0u64;
        // The value at the last blind choice passed: what a round lost from there is set against.
        let mut last_choice = here;
        loop {
            if run.won {
                break SKIP_WON;
            }
            if run.phase == Phase::GameOver || step >= SKIP_MAX_STEPS {
                break last_choice - loss;
            }
            if at_checkpoint(&run, checkpoint) {
                break value_settled(&run, salt);
            }
            if run.phase == Phase::BlindSelect {
                last_choice = value_settled(&run, salt);
            }
            let (k, a) = plain_move(&run, salt ^ (step << 24));
            if crate::env::apply(&mut run, k, a).is_err() {
                break last_choice - loss;
            }
            step += 1;
        }
    });
    SKIP_DEPTH.with(|d| d.set(d.get() - 1));
    out
}

/// Jokers that pay on the round's final hand: Dusk (its cards retriggered) and Acrobat (x3).
pub(crate) fn final_hand_jokers(run: &Run) -> bool {
    run.count_jokers(JokerKind::Dusk) + run.count_jokers(JokerKind::Acrobat) > 0
}

/// Bosses under which single cards played first spoil the final hand: The Mouth (one hand type a round:
/// High Card, after a single card), The Hook (two held cards thrown after each hand), The Ox (playing the
/// most played type, which single cards soon make High Card, takes all the money) and Crimson Heart (a
/// joker off for each hand, Dusk perhaps for the final one).
pub(crate) fn play_down_spoiled(run: &Run) -> bool {
    use crate::blinds::Boss;
    matches!(
        run.blind.as_ref().and_then(|b| b.active_boss()),
        Some(Boss::Mouth | Boss::Hook | Boss::Ox | Boss::CrimsonHeart)
    )
}

/// The best hand's score if it were the round's final hand.
pub(crate) fn final_hand_score(run: &Run, mask: u64) -> f64 {
    let mut last = run.clone();
    last.hands_left = 1;
    preview_score(&last, mask)
}

/// Playing the round down to its final hand ("final-hand"): with Dusk or Acrobat, when the best hand
/// (`mask`) falls short of `needed` now but beats it as the final hand (judged on a fair copy), one spare
/// card (the least worth keeping) is played on its own while more than one hand is left. Not with cards face
/// down in hand: what the final hand would score is then a guess.
fn play_down(run: &Run, mask: u64, needed: f64, salt: u64) -> Option<(MoveKind, u64)> {
    if run.hands_left <= 1
        || !final_hand_jokers(run)
        || run.forced_card().is_some()
        || play_down_spoiled(run)
        || run.hand.iter().any(|&id| run.card(id).face_down)
    {
        return None;
    }
    if final_hand_score(&fair_copy(run, salt ^ 0xF1A1), mask) < needed {
        return None;
    }
    let spare: Vec<usize> = (0..run.hand.len())
        .filter(|&i| mask >> i & 1 == 0)
        .collect();
    let junk = spare.into_iter().min_by(|&a, &b| {
        crate::value::card_value(run, run.hand[a])
            .total_cmp(&crate::value::card_value(run, run.hand[b]))
            .then(b.cmp(&a))
    })?;
    Some((MoveKind::Play, 1u64 << junk))
}

/// Moves that only put cards or jokers in another order.
pub fn reorders(kind: MoveKind) -> bool {
    matches!(
        kind,
        MoveKind::MoveCard | MoveKind::MoveJoker | MoveKind::SortRank | MoveKind::SortSuit
    )
}

/// How the teacher measures a decision (`judge_option`): a round's moves by the chance of beating the blind,
/// the rest by the value of the position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Measure {
    BeatChance,
    Value,
}

/// Tries behind the chance of beating the blind after a round's move (`judge_option`).
const JUDGE_TRIES: u64 = 40;
/// Fair copies behind the value of a shop or pack move (`judge_option`).
const JUDGE_COPIES: u64 = 4;

/// What the teacher thinks one move is worth here (to set a player's move against its own,
/// `examples/judge.rs`). In a round: the chance of beating the blind after it. At the blind choice: playing
/// against skipping, as the teacher weighs them. In a shop or a pack: the value of the position the run
/// reaches when, after this move, the teacher plays on until the shop is left (a move is often the first of a
/// plan: selling a joker to buy another), on fair copies of what is hidden, averaged. Nothing hidden is used.
pub fn judge_option(run: &Run, salt: u64, first: (MoveKind, u64)) -> (Measure, f64) {
    use crate::value::value;
    let vs = value_salt(run, salt);
    match run.phase {
        Phase::Round => {
            let p = match after(run, vs ^ 0x5EED, &[first]) {
                None => f64::NAN,
                Some(c) => match c.phase {
                    Phase::Round => crate::planner::beat_rate_with(&c, salt, JUDGE_TRIES),
                    Phase::GameOver => 0.0,
                    _ => 1.0,
                },
            };
            (Measure::BeatChance, p)
        }
        Phase::BlindSelect if first.0 == MoveKind::SelectBlind => {
            // Playing the blind pays its reward and the unused hands (about $5), as `value_blind_move` counts.
            let mut played = run.clone();
            played.money += 5;
            (Measure::Value, value(&played, vs))
        }
        Phase::BlindSelect | Phase::Shop | Phase::Pack => {
            let mut total = 0.0;
            let mut seen = 0.0;
            for k in 0..JUDGE_COPIES {
                let Some(mut c) = after(run, vs ^ (0xC0FE + k), &[first]) else {
                    continue;
                };
                // The teacher plays on to the end of the shop (or of a pack a skip's tag opened).
                let mut steps = 0;
                while matches!(c.phase, Phase::Shop | Phase::Pack) && steps < 60 {
                    let m = teacher_move(&c, salt + 1 + steps);
                    if crate::env::apply(&mut c, m.0, m.1).is_err() {
                        break;
                    }
                    steps += 1;
                }
                total += value(&c, vs);
                seen += 1.0;
            }
            (
                Measure::Value,
                if seen > 0.0 { total / seen } else { f64::NAN },
            )
        }
        Phase::RoundEval | Phase::GameOver => (Measure::Value, value(run, vs)),
    }
}

/// The teacher's own decision here: its reorders made (on a copy), then the first move that isn't one. Returns
/// the run with its reorders made, the reorders, and that move.
pub fn decision(run: &Run, salt: u64) -> (Run, Vec<(MoveKind, u64)>, (MoveKind, u64)) {
    let mut c = run.clone();
    let mut pre = Vec::new();
    for i in 0..40u64 {
        let m = teacher_move(&c, salt + i);
        if !reorders(m.0) || crate::env::apply(&mut c, m.0, m.1).is_err() {
            return (c, pre, m);
        }
        pre.push(m);
    }
    let m = teacher_move(&c, salt + 40);
    (c, pre, m)
}

/// Start a run on this thread: forget what the last runs remembered (the decisions being carried out, the
/// estimates). A remembered estimate's key leaves out a few things it hardly depends on (Fortune Teller's
/// Tarots used, say), so a run must not meet another run's: each run then plays the same however many runs
/// came before it on its thread, and a job's results don't depend on how its runs fell to its threads.
pub fn new_run() {
    COMMIT.with(|c| c.set(None));
    CHOICES.with(|c| c.borrow_mut().clear());
    HAND_ORDERS.with(|c| c.borrow_mut().clear());
    WEAKEST.with(|c| c.borrow_mut().clear());
    LEAVING.with(|c| c.borrow_mut().clear());
    crate::value::clear_caches();
}

/// The teacher's move. From ante "late-from" (6), with "late-x" above 1, every count behind its decisions is
/// scaled by that much (`value::Scales`, rounds included): the late antes are where most runs are lost, and a
/// decision there has fewer to come after it.
pub fn teacher_move(run: &Run, salt: u64) -> (MoveKind, u64) {
    let late = param("late-x").unwrap_or(1.0);
    let mut x = 1.0;
    if late > 1.0 && f64::from(run.ante) >= param("late-from").unwrap_or(LATE_FROM) {
        x = late;
    }
    // "boss-x": in a boss blind's round, every count scaled by this much (most runs are lost at a boss).
    let boss = param("boss-x").unwrap_or(1.0);
    if boss > 1.0
        && run.phase == Phase::Round
        && run
            .blind
            .as_ref()
            .is_some_and(|b| matches!(b.kind, crate::blinds::BlindKind::Boss(_)))
    {
        x = x.max(boss);
    }
    if x > 1.0 {
        let s = crate::value::scales();
        let scaled = crate::value::Scales {
            full: s.full * x,
            quick: s.quick * x,
            looks: s.looks * x,
            rounds: s.rounds * x,
        };
        return crate::value::with_scales(scaled, || plain_move(run, salt));
    }
    plain_move(run, salt)
}

/// The ante "late-x" scales the teacher's counts from, unless "late-from" sets another.
const LATE_FROM: f64 = 6.0;

fn plain_move(run: &Run, salt: u64) -> (MoveKind, u64) {
    let by_value = by_value();
    match run.phase {
        Phase::BlindSelect if by_value => value_blind_move(run, salt),
        Phase::BlindSelect => (MoveKind::SelectBlind, 0),
        Phase::RoundEval => (MoveKind::CashOut, 0),
        Phase::Shop if by_value => value_shop_move(run, salt),
        Phase::Shop => shop_move(run),
        Phase::Pack if by_value => value_pack_move(run, salt),
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
    fn a_new_run_starts_with_nothing_remembered() {
        let (run, _) = play("NEWRUN01", teacher_move);
        let _ = run;
        let remembered = || {
            crate::value::cached()
                + CHOICES.with(|c| c.borrow().len())
                + HAND_ORDERS.with(|c| c.borrow().len())
                + WEAKEST.with(|c| c.borrow().len())
                + LEAVING.with(|c| c.borrow().len())
        };
        assert!(remembered() > 0, "a run leaves estimates behind");
        new_run();
        assert_eq!(remembered(), 0);
    }

    #[test]
    fn the_teacher_judges_any_move_and_names_its_own_decision() {
        // Along a run: at each decision its own move (after any reorders) is legal and not a reorder, and every
        // move it is asked about gets a finite worth of the right kind; leaving a shop is worth the shop as
        // it stands.
        let mut run = Run::new(RunConfig::new("JUDGE001"));
        let mut step = 0u64;
        let mut judged = 0;
        while run.phase != Phase::GameOver && !run.won && step < 400 {
            new_run();
            let (mine, _, m) = decision(&run, step);
            assert!(!reorders(m.0), "a decision is not a reorder");
            assert!(
                apply(&mut mine.clone(), m.0, m.1).is_ok(),
                "its decision is legal"
            );
            if step.is_multiple_of(7) && run.phase != Phase::RoundEval {
                new_run();
                let (measure, v) = judge_option(&mine, step, m);
                assert!(v.is_finite(), "{:?} {m:?}", run.phase);
                let round = run.phase == Phase::Round;
                assert_eq!(measure == Measure::BeatChance, round);
                if round {
                    assert!((0.0..=1.0).contains(&v));
                }
                judged += 1;
            }
            if run.phase == Phase::Shop {
                new_run();
                let here = crate::value::value(&run, value_salt(&run, step));
                assert_eq!(judge_option(&run, step, (MoveKind::LeaveShop, 0)).1, here);
            }
            let (k, a) = teacher_move(&run, step);
            assert!(apply(&mut run, k, a).is_ok());
            step += 1;
        }
        assert!(judged > 10);
    }

    #[test]
    fn teacher_moves_are_always_legal_and_checkable() {
        // The value teacher (the default) and the earlier one.
        for (options, runs) in [(&[][..], 12), (&["classic"][..], 40)] {
            with_options(options, || {
                let mut seeds = SeedSampler::new(4242);
                let mut seen = std::collections::HashSet::new();
                for _ in 0..runs {
                    let (_, kinds) = play(&seeds.next_seed(), teacher_move);
                    seen.extend(kinds);
                }
                for k in [
                    MoveKind::SellJoker,
                    MoveKind::Reroll,
                    MoveKind::UseConsumable,
                    MoveKind::OpenBooster,
                    MoveKind::PackChoose,
                ] {
                    assert!(
                        seen.contains(&k),
                        "{options:?}: the teacher never made a {k:?}"
                    );
                }
            });
        }
    }

    /// A round position: the given jokers, and the hand given as (suit, rank) pairs.
    fn round_with(
        blind: crate::blinds::BlindKind,
        jokers: &[JokerKind],
        hand: &[(Suit, crate::card::Rank)],
    ) -> Run {
        let mut run = Run::scenario("ROUNDPOS", blind);
        run.hand.clear();
        for &k in jokers {
            run.add_joker(k);
        }
        for &(s, r) in hand {
            run.add_card_to_hand(s, r);
        }
        run
    }

    #[test]
    fn with_final_hand_a_round_is_played_down_to_the_hand_that_wins_it() {
        // Acrobat triples the final hand: a pair of Aces that falls short now beats the blind as the last hand,
        // so with "final-hand" a single spare card goes first; without it the teacher doesn't do that.
        use crate::blinds::BlindKind;
        use crate::card::Rank::*;
        use Suit::*;
        let mut run = round_with(
            BlindKind::Small,
            &[JokerKind::Acrobat],
            &[
                (Spades, Ace),
                (Hearts, Ace),
                (Clubs, Nine),
                (Diamonds, Seven),
                (Clubs, Five),
                (Hearts, Four),
                (Spades, Three),
                (Diamonds, Two),
            ],
        );
        let (now, mask) = best_play(&run, 7);
        let last = final_hand_score(&run, mask);
        assert!(
            last > 2.5 * now,
            "Acrobat triples the final hand: {now} now, {last} last"
        );
        run.blind.as_mut().unwrap().chips = (now + last) / 2.0;
        let (k, a) = with_options(&["final-hand"], || teacher_move(&run, 7));
        assert_eq!(k, MoveKind::Play);
        assert_eq!(a.count_ones(), 1, "one spare card");
        assert_eq!(a & mask, 0, "not one of the winning hand");
        let plain = teacher_move(&run, 7);
        assert_ne!(plain, (k, a), "not without the option");
    }

    #[test]
    fn a_round_is_not_played_down_where_single_cards_spoil_the_final_hand() {
        // The Mouth allows one hand type a round: a single card first would leave the final pair scoring
        // nothing. The same hand as above, under The Mouth.
        use crate::blinds::{BlindKind, Boss};
        use crate::card::Rank::*;
        use Suit::*;
        let mut run = round_with(
            BlindKind::Boss(Boss::Mouth),
            &[JokerKind::Acrobat],
            &[
                (Spades, Ace),
                (Hearts, Ace),
                (Clubs, Nine),
                (Diamonds, Seven),
                (Clubs, Five),
                (Hearts, Four),
                (Spades, Three),
                (Diamonds, Two),
            ],
        );
        assert!(play_down_spoiled(&run));
        let (now, mask) = best_play(&run, 7);
        let last = final_hand_score(&run, mask);
        run.blind.as_mut().unwrap().chips = (now + last) / 2.0;
        let (k, a) = with_options(&["final-hand"], || teacher_move(&run, 7));
        assert!(
            !(k == MoveKind::Play && a.count_ones() == 1 && a & mask == 0),
            "no single spare card under The Mouth: {k:?} {a:b}"
        );
    }

    #[test]
    fn a_safe_blind_is_farmed_for_mail_in_rebate() {
        use crate::blinds::BlindKind;
        use crate::card::Rank::*;
        use Suit::*;
        let mut run = Run::scenario("ROUNDPOS", BlindKind::Small);
        run.hand.clear();
        for k in [JokerKind::Joker, JokerKind::MailInRebate] {
            run.add_joker(k);
        }
        for (s, r) in [
            (Spades, Ace),
            (Spades, King),
            (Spades, Queen),
            (Spades, Jack),
            (Spades, Nine),
            (Clubs, Five),
            (Diamonds, Five),
            (Hearts, Two),
        ] {
            run.add_card_to_hand(s, r);
        }
        run.mail_rank = Five;
        let (k, mask) = teacher_move(&run, 1 << 20);
        assert_eq!(
            k,
            MoveKind::Discard,
            "the flush wins; the Fives pay $5 each when thrown first"
        );
        assert!(
            mask >> 5 & 1 == 1 && mask >> 6 & 1 == 1,
            "both Fives thrown: {mask:#b}"
        );
        assert_eq!(mask & 0b11111, 0, "the winning flush kept");
    }

    #[test]
    fn dna_copies_the_best_card_on_the_first_hand() {
        // A flush wins the Small Blind; DNA and Baron held, a Red Seal Steel King in hand: the first hand is
        // the King alone (its copy joins the deck and comes back to the hand), then the flush.
        use crate::blinds::BlindKind;
        use crate::card::Rank::*;
        use Suit::*;
        let mut run = Run::scenario("ROUNDPOS", BlindKind::Small);
        run.hand.clear();
        for k in [JokerKind::Joker, JokerKind::DNA, JokerKind::Baron] {
            run.add_joker(k);
        }
        for (s, r) in [
            (Spades, Ace),
            (Spades, Queen),
            (Spades, Jack),
            (Spades, Nine),
            (Spades, Three),
            (Clubs, Five),
            (Diamonds, Four),
        ] {
            run.add_card_to_hand(s, r);
        }
        run.add_special_card_to_hand(Hearts, King, Enhancement::Steel, Edition::None, Seal::Red);
        let before = run.playing_cards.len();
        let mut moves = 0;
        let first = loop {
            let (k, a) = teacher_move(&run, 1 << 20 | moves);
            if k == MoveKind::Play || k == MoveKind::Discard {
                break (k, a);
            }
            apply(&mut run, k, a).unwrap();
            moves += 1;
            assert!(moves < 10);
        };
        assert_eq!(first.0, MoveKind::Play);
        assert_eq!(first.1.count_ones(), 1, "one card alone: {:#b}", first.1);
        let i = first.1.trailing_zeros() as usize;
        let c = run.card(run.hand[i]);
        assert_eq!((c.rank, c.enhancement), (King, Enhancement::Steel));
        apply(&mut run, first.0, first.1).unwrap();
        assert_eq!(
            run.playing_cards.len(),
            before + 1,
            "DNA's copy joined the deck"
        );
    }

    #[test]
    fn a_reroll_is_worth_its_search() {
        // Two sample shops with nothing worth buying and one with a find: rerolling again after a miss
        // (at 0.2 a time) makes the search worth 10.6, more than one look (10.33).
        let best = [10.0, 10.0, 11.0];
        assert!((search(&best, 0.2, false) - 31.0 / 3.0).abs() < 1e-9);
        assert!((search(&best, 0.2, true) - 10.6).abs() < 1e-6);
        // Too dear to search: no better than one look.
        assert!((search(&best, 2.0, true) - 31.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn a_shop_with_a_free_slot_buys_the_strong_joker_and_keeps_its_own() {
        // Ante 2, $12, four jokers in five slots (Golden Joker, Ancient Joker, Shoot the Moon, Egg); the
        // shop has Reserved Parking and Bloodstone. Filling the last slot used to look like a loss and a
        // sale like a gain: it rerolled, bought a weak joker and sold Ancient Joker.
        use crate::jokers::Joker;
        use crate::shop::RowSlot;
        let mut run = Run::new(RunConfig::new("SHOPCASE"));
        let mut step = 0;
        while !(run.phase == Phase::Shop && run.ante == 2) {
            let (k, a) = teacher_move(&run, step);
            apply(&mut run, k, a).unwrap();
            step += 1;
        }
        run.jokers.clear();
        for k in [
            JokerKind::GoldenJoker,
            JokerKind::AncientJoker,
            JokerKind::ShootTheMoon,
            JokerKind::Egg,
        ] {
            run.add_joker(k);
        }
        run.joker_slots = 5;
        run.money = 12;
        let shop = run.shop.as_mut().unwrap();
        shop.row = vec![
            RowSlot {
                item: RowItem::Joker(Joker::new(JokerKind::ReservedParking, 9001)),
                cost: 6,
                couponed: false,
            },
            RowSlot {
                item: RowItem::Joker(Joker::new(JokerKind::Bloodstone, 9002)),
                cost: 7,
                couponed: false,
            },
        ];
        shop.boosters.clear();
        shop.vouchers.clear();
        for s in 0..12 {
            let (k, a) = teacher_move(&run, 5 << 20 | s);
            if k == MoveKind::LeaveShop {
                break;
            }
            assert_ne!(k, MoveKind::SellJoker, "nothing to sell here");
            apply(&mut run, k, a).unwrap();
        }
        let kinds: Vec<JokerKind> = run.jokers.iter().map(|j| j.kind).collect();
        assert!(kinds.contains(&JokerKind::AncientJoker), "{kinds:?}");
        assert!(kinds.contains(&JokerKind::Bloodstone), "{kinds:?}");
    }

    #[test]
    fn the_avoid_options_leave_out_the_jokers_with_their_sticker() {
        use crate::jokers::Joker;
        use crate::shop::RowSlot;
        let mut run = Run::new(RunConfig::at_stake("AVOIDSTK", 8));
        let mut step = 0;
        while run.phase != Phase::Shop {
            let (k, a) = teacher_move(&run, step);
            apply(&mut run, k, a).unwrap();
            step += 1;
        }
        run.money = 30;
        let marked = |f: fn(&mut Joker)| {
            let mut j = Joker::new(JokerKind::Bloodstone, 9100);
            f(&mut j);
            RowSlot {
                item: RowItem::Joker(j),
                cost: 5,
                couponed: false,
            }
        };
        let shop = run.shop.as_mut().unwrap();
        shop.row = vec![
            marked(|j| j.rental = true),
            marked(|j| j.eternal = true),
            marked(|j| {
                j.perishable = true;
                j.perish_tally = 5;
            }),
            marked(|_| {}),
        ];
        let buys = |opts: &[&str]| -> Vec<u64> {
            with_options(opts, || {
                shop_buys(&run, 0)
                    .iter()
                    .filter_map(|m| m.iter().find(|(k, _)| *k == MoveKind::Buy).map(|&(_, i)| i))
                    .collect()
            })
        };
        assert_eq!(buys(&[]), [0, 1, 2, 3]);
        assert_eq!(buys(&["avoid-rental"]), [1, 2, 3]);
        assert_eq!(buys(&["avoid-eternal", "avoid-perishable"]), [0, 3]);
        assert_eq!(
            buys(&["avoid-rental", "avoid-eternal", "avoid-perishable"]),
            [3]
        );
    }

    #[test]
    fn square_joker_gets_four_card_hands() {
        use crate::blinds::BlindKind;
        use crate::card::Rank::*;
        use Suit::*;
        let run = round_with(
            BlindKind::Small,
            &[JokerKind::Joker, JokerKind::Joker, JokerKind::SquareJoker],
            &[
                (Spades, Ace),
                (Hearts, Ace),
                (Diamonds, Ace),
                (Clubs, Nine),
                (Diamonds, Four),
                (Spades, Two),
                (Hearts, Three),
                (Clubs, Seven),
            ],
        );
        let (k, mask) = teacher_move(&run, 2 << 20);
        assert_eq!(k, MoveKind::Play);
        assert_eq!(
            mask.count_ones(),
            4,
            "four cards played for Square Joker: {mask:#b}"
        );
    }

    #[test]
    fn blueprint_copies_cartomancer_when_the_shop_closes() {
        // A shop with Blueprint, Cavendish and Cartomancer: before leaving, Blueprint goes just left of
        // Cartomancer (two Tarots at the blind's start); the scoring order is restored in the round.
        // The first run (of a few seeds) whose first blind is beaten, at its shop.
        let mut seeds = SeedSampler::new(31);
        let mut run = loop {
            let mut run = Run::new(RunConfig::new(&seeds.next_seed()));
            let mut step = 0;
            while run.phase != Phase::Shop && run.phase != Phase::GameOver {
                let (k, a) = teacher_move(&run, step);
                apply(&mut run, k, a).unwrap();
                step += 1;
            }
            if run.phase == Phase::Shop {
                break run;
            }
        };
        run.jokers.clear();
        for k in [
            JokerKind::Cavendish,
            JokerKind::Blueprint,
            JokerKind::Cartomancer,
        ] {
            run.add_joker(k);
        }
        run.money = 0; // little to buy: the decisions left are mostly the order and leaving
        run.consumables.clear();
        let mut moves = 0;
        loop {
            let (k, a) = teacher_move(&run, 5 << 20 | moves);
            if k == MoveKind::LeaveShop && run.phase == Phase::Shop {
                break;
            }
            apply(&mut run, k, a).unwrap();
            moves += 1;
            assert!(moves < 30);
        }
        assert!(
            run.jokers.iter().any(|j| j.kind == JokerKind::Cartomancer)
                && run.jokers.iter().any(|j| j.kind == JokerKind::Blueprint),
            "both still held"
        );
        let order: Vec<JokerKind> = run.jokers.iter().map(|j| j.kind).collect();
        let b = order
            .iter()
            .position(|&k| k == JokerKind::Blueprint)
            .unwrap();
        assert_eq!(order.get(b + 1), Some(&JokerKind::Cartomancer), "{order:?}");
        apply(&mut run, MoveKind::LeaveShop, 0).unwrap();
        apply(&mut run, MoveKind::SelectBlind, 0).unwrap();
        let made = run.consumables.len();
        assert!(made >= 2, "two Tarots at the blind's start, {made} held");
    }

    #[test]
    fn death_copies_what_the_jokers_pay_for() {
        // With Baron, a plain King is worth more to copy than a plain Ace; the Two becomes the King.
        use crate::blinds::BlindKind;
        use crate::card::Rank::*;
        use crate::content::ConsumableKind;
        use Suit::*;
        let mut run = round_with(
            BlindKind::Small,
            &[JokerKind::Baron],
            &[
                (Spades, Two),
                (Hearts, Ace),
                (Clubs, King),
                (Diamonds, Seven),
            ],
        );
        let k = crate::value::card_value(&run, run.hand[2]);
        let a = crate::value::card_value(&run, run.hand[1]);
        assert!(k > a, "with Baron a King ({k}) beats an Ace ({a})");
        let death = crate::create::Consumable {
            kind: ConsumableKind::Death,
            sort_id: 0,
            edition: Edition::None,
            extra_value: 0,
            cost: 3,
            sell_cost: 1,
        };
        let sets = target_sets(&run, &death);
        assert_eq!(sets.first(), Some(&vec![0, 2]), "{sets:?}");
        run.jokers.clear();
        let sets = target_sets(&run, &death);
        assert_eq!(
            sets.first(),
            Some(&vec![0, 1]),
            "without Baron the Ace: {sets:?}"
        );
    }

    #[test]
    fn every_copier_can_copy_one_event_joker() {
        // Blueprint chained onto Brainstorm, Brainstorm on the leftmost: both copy Cartomancer.
        let mut run = Run::scenario("CHAINORD", crate::blinds::BlindKind::Small);
        for k in [
            JokerKind::Cavendish,
            JokerKind::Blueprint,
            JokerKind::Brainstorm,
            JokerKind::Cartomancer,
        ] {
            run.add_joker(k);
        }
        let orders = copy_orders(&run, SHOP_EXIT_EVENTS);
        let kinds = |o: &Vec<usize>| o.iter().map(|&i| run.jokers[i].kind).collect::<Vec<_>>();
        assert!(
            orders.iter().any(|o| kinds(o)
                == vec![
                    JokerKind::Cartomancer,
                    JokerKind::Cavendish,
                    JokerKind::Blueprint,
                    JokerKind::Brainstorm
                ]),
            "{:?}",
            orders.iter().map(kinds).collect::<Vec<_>>()
        );
    }

    #[test]
    fn verdant_leaf_costs_the_weakest_joker() {
        use crate::blinds::{BlindKind, Boss};
        use crate::card::Rank::*;
        use Suit::*;
        let run = round_with(
            BlindKind::Boss(Boss::VerdantLeaf),
            &[JokerKind::Cavendish, JokerKind::Joker],
            &[
                (Spades, Ace),
                (Hearts, Ace),
                (Diamonds, Two),
                (Clubs, Five),
                (Diamonds, Four),
                (Spades, Nine),
                (Hearts, Three),
                (Clubs, Seven),
            ],
        );
        let (k, a) = teacher_move(&run, 3 << 20);
        assert_eq!(k, MoveKind::SellJoker);
        assert_eq!(run.jokers[a as usize].kind, JokerKind::Joker);
    }

    #[test]
    fn death_copies_the_best_card_moving_it_right_first() {
        use crate::blinds::BlindKind;
        use crate::card::Rank::*;
        use crate::content::ConsumableKind;
        use Suit::*;
        let mut run = round_with(BlindKind::Small, &[], &[]);
        let king = run.add_special_card_to_hand(
            Hearts,
            King,
            Enhancement::Glass,
            Edition::None,
            Seal::Red,
        );
        for (s, r) in [(Spades, Five), (Clubs, Six), (Diamonds, Two)] {
            run.add_card_to_hand(s, r);
        }
        let death = crate::create::Consumable {
            kind: ConsumableKind::Death,
            sort_id: 0,
            edition: Edition::None,
            extra_value: 0,
            cost: 3,
            sell_cost: 1,
        };
        let sets = target_sets(&run, &death);
        assert!(
            sets.contains(&vec![3, 0]),
            "the Two becomes the King: {sets:?}"
        );
        let moves = use_on(&run, &death, &[3, 0], &|m| {
            (MoveKind::UseConsumable, m << 8)
        });
        assert_eq!(
            moves[0],
            (MoveKind::MoveCard, 3 << 8),
            "the King moves right first"
        );
        assert_eq!(moves[1], (MoveKind::UseConsumable, (1 << 2 | 1 << 3) << 8));
        let _ = king;
    }

    #[test]
    fn jokers_are_ordered_so_copies_copy_something() {
        use crate::blinds::BlindKind;
        use JokerKind::*;
        let order = |kinds: &[JokerKind]| -> Vec<JokerKind> {
            let mut run = Run::scenario("ORDERING", BlindKind::Small);
            for &k in kinds {
                run.add_joker(k);
            }
            let mut moves = 0;
            while let Some((k, a)) = reorder(&run) {
                apply(&mut run, k, a).unwrap();
                moves += 1;
                assert!(moves <= 10, "the order must settle");
            }
            run.jokers.iter().map(|j| j.kind).collect()
        };
        // Brainstorm never leftmost: the best joker it can copy goes there, Brainstorm last.
        let o = order(&[Brainstorm, Joker, Cavendish]);
        assert_eq!(o, vec![Cavendish, Joker, Brainstorm]);
        // Blueprint just left of the best joker it can copy; adders before multipliers.
        let o = order(&[Cavendish, Blueprint, Joker, GreedyJoker]);
        assert_eq!(o.last(), Some(&Cavendish));
        assert_eq!(o[o.len() - 2], Blueprint);
        let o = order(&[Blueprint, Joker]);
        assert_eq!(
            o,
            vec![Blueprint, Joker],
            "Blueprint never rightmost while it has a joker to copy"
        );
        // Both copiers: Blueprint copies Brainstorm, which copies the leftmost, so both copy Cavendish, and
        // both copies come after the Joker's +4.
        let o = order(&[Brainstorm, Blueprint, Joker, Cavendish]);
        assert_eq!(o, vec![Cavendish, Joker, Blueprint, Brainstorm]);
        // A best joker that adds: the copies come before the multipliers.
        let o = order(&[Brainstorm, GlassJoker, Blueprint, Joker, Stuntman]);
        assert_eq!(o, vec![Stuntman, Joker, Blueprint, Brainstorm, GlassJoker]);
        // Ceremonial Dagger rightmost: it destroys the joker to its right at every blind.
        let o = order(&[CeremonialDagger, Cavendish, Joker]);
        assert_eq!(o, vec![Joker, Cavendish, CeremonialDagger]);
    }

    #[test]
    fn each_hand_gets_the_joker_order_for_its_purpose() {
        // A Polychrome Joker (+4, then x1.5), Gros Michel (+15) and Cavendish (x3), set worst first.
        use crate::blinds::BlindKind;
        use crate::card::Rank::*;
        use Suit::*;
        let mut run = round_with(
            BlindKind::Small,
            &[
                JokerKind::Cavendish,
                JokerKind::Joker,
                JokerKind::GrosMichel,
            ],
            &[
                (Spades, Ace),
                (Hearts, Ace),
                (Clubs, Two),
                (Diamonds, Seven),
            ],
        );
        run.jokers[1].edition = Edition::Polychrome;
        let mask = 0b11;
        let score = |run: &Run, o: &[usize]| {
            let mut c = fair_copy(run, 9 ^ 0x0DE5);
            c.jokers = o.iter().map(|&i| run.jokers[i].clone()).collect();
            preview_score(&c, mask)
        };
        let all = all_orders(&run);
        assert_eq!(all.len(), 6);
        let top = all.iter().map(|o| score(&run, o)).fold(0.0, f64::max);
        let low = all.iter().map(|o| score(&run, o)).fold(f64::MAX, f64::min);
        let kinds =
            |run: &Run, o: &[usize]| o.iter().map(|&i| run.jokers[i].kind).collect::<Vec<_>>();
        // A blind only the best order comes near: the adders first, the Polychrome one last of them.
        let mut hard = run.clone();
        hard.blind.as_mut().unwrap().chips = top;
        let o = hand_order(&hard, mask, 9, false);
        assert_eq!(score(&hard, &o), top);
        assert_eq!(
            kinds(&hard, &o),
            vec![
                JokerKind::GrosMichel,
                JokerKind::Joker,
                JokerKind::Cavendish
            ]
        );
        // A blind every order beats easily: still the most headroom (chance effects may score less).
        let mut easy = run.clone();
        easy.blind.as_mut().unwrap().chips = low / 2.0;
        assert_eq!(score(&easy, &hand_order(&easy, mask, 9, false)), top);
        // A hand meant to keep the round going, on a blind the best order would beat: it scores under it.
        let mut go_on = run.clone();
        go_on.blind.as_mut().unwrap().chips = (top + low) / 2.0;
        let o = hand_order(&go_on, mask, 9, true);
        assert!(
            score(&go_on, &o) < (top + low) / 2.0,
            "{:?}",
            kinds(&go_on, &o)
        );
    }

    #[test]
    fn the_teachers_joker_order_scores_the_most() {
        // Brainstorm starts leftmost (copying nothing). Of all 24 orders of these four jokers, the teacher's
        // scores the most: Cavendish copied twice, after the Joker's +4.
        use crate::blinds::BlindKind;
        use crate::card::Rank;
        use JokerKind::*;
        let kinds = [Brainstorm, Blueprint, Joker, Cavendish];
        let score = |order: &[JokerKind], settle: bool| -> (f64, Vec<JokerKind>) {
            let mut run = Run::scenario("ORDERING", BlindKind::Small);
            run.hand.clear();
            for &k in order {
                run.add_joker(k);
            }
            run.add_card_to_hand(Suit::Spades, Rank::Ace);
            run.add_card_to_hand(Suit::Hearts, Rank::Ace);
            while let Some((k, a)) = reorder(&run).filter(|_| settle) {
                apply(&mut run, k, a).unwrap();
            }
            (
                preview_score(&run, 0b11),
                run.jokers.iter().map(|j| j.kind).collect(),
            )
        };
        let (teacher, got) = score(&kinds, true);
        let mut best = (0.0, Vec::new());
        let mut perm = kinds.to_vec();
        // Heap's algorithm over the 24 orders.
        let mut c = [0usize; 4];
        let mut i = 0;
        let mut seen = vec![score(&perm, false)];
        while i < 4 {
            if c[i] < i {
                perm.swap(if i % 2 == 0 { 0 } else { c[i] }, i);
                seen.push(score(&perm, false));
                c[i] += 1;
                i = 0;
            } else {
                c[i] = 0;
                i += 1;
            }
        }
        assert_eq!(seen.len(), 24);
        for s in seen {
            if s.0 > best.0 {
                best = s;
            }
        }
        assert_eq!(got, vec![Cavendish, Joker, Blueprint, Brainstorm]);
        assert!(
            teacher >= best.0,
            "the teacher's order scores {teacher}, but {:?} scores {}",
            best.1,
            best.0
        );
    }

    #[test]
    fn the_teacher_goes_further_than_the_greedy_bot() {
        let mut seeds = SeedSampler::new(777);
        let (mut t, mut g) = (0i64, 0i64);
        for _ in 0..24 {
            let seed = seeds.next_seed();
            let score = |r: &Run| if r.won { 9 } else { r.ante as i64 };
            t += score(&play(&seed, teacher_move).0);
            g += score(&play(&seed, crate::bots::greedy_move).0);
        }
        assert!(
            t > g,
            "teacher {t} vs greedy {g} (antes summed over the same 24 seeds)"
        );
    }

    #[test]
    fn a_focus_takes_its_own_planets() {
        use crate::content::ConsumableKind::*;
        let mut run = Run::scenario("FOCUS1", crate::blinds::BlindKind::Small);
        assert_eq!(focus_hand(&run), None, "nothing played yet");
        run.hand_levels.get_mut(HandType::HighCard).played = 12;
        run.hand_levels.get_mut(HandType::Straight).played = 4;
        run.hand_levels.get_mut(HandType::Pair).played = 5;
        assert_eq!(
            focus_hand(&run),
            Some(HandType::Pair),
            "the most played, High Card aside"
        );
        // A Celestial pack offering Saturn, Mercury and Jupiter: Mercury.
        let pack = |run: &mut Run, kinds: &[ConsumableKind]| {
            let cards = kinds
                .iter()
                .map(|&k| PackCard::Consumable(run.make_consumable(k)))
                .collect();
            run.pack = Some(crate::packs::OpenPack {
                booster: 0,
                cards,
                choices_left: 1,
                return_to: Phase::Shop,
            });
            run.phase = Phase::Pack;
        };
        let celestial = BOOSTERS
            .iter()
            .position(|b| b.kind == PackKind::Celestial)
            .unwrap() as u8;
        pack(&mut run, &[Saturn, Mercury, Jupiter]);
        run.pack.as_mut().unwrap().booster = celestial;
        assert_eq!(focus_pick(&run, 0), Some(1));
        assert_eq!(
            value_pack_move(&run, 3),
            (MoveKind::PackChoose, 1),
            "the teacher's own play"
        );
        with_options(&["focus"], || {
            assert_eq!(value_pack_move(&run, 3), (MoveKind::PackChoose, 1));
        });
        assert!(with_options(&["no-focus"], || !focus_on()));
        assert!(with_options(&["no-fair-skip"], || !fair_skip_on()) && fair_skip_on());
        // A Black Hole first.
        pack(&mut run, &[Saturn, Mercury, BlackHole]);
        run.pack.as_mut().unwrap().booster = celestial;
        assert_eq!(focus_pick(&run, 0), Some(2));
        // Not the focus's: the value decides.
        pack(&mut run, &[Saturn, Venus, Jupiter]);
        run.pack.as_mut().unwrap().booster = celestial;
        assert_eq!(focus_pick(&run, 0), None);
    }

    #[test]
    fn a_committed_run_serves_one_hand_from_the_start_chosen_by_its_number_alone() {
        use crate::content::ConsumableKind::*;
        let mut run = Run::scenario("COMMIT1", crate::blinds::BlindKind::Small);
        run.hand_levels.get_mut(HandType::Pair).played = 9; // the focus, were the run not committed
        new_run();
        assert_eq!(committed(5 << 20), None, "off unless asked");
        assert_eq!(run_focus(&run, 5 << 20), Some(HandType::Pair));
        let commits: Vec<Option<HandType>> = (0..400u64)
            .map(|i| {
                new_run();
                with_options(&["commit=1"], || {
                    let c = committed(i << 20 | 7);
                    assert_eq!(
                        committed(i << 20 | 300),
                        c,
                        "the same for every decision of the run"
                    );
                    c
                })
            })
            .collect();
        assert!(
            commits.iter().all(|c| c.is_some()),
            "every run commits at 1"
        );
        for h in COMMIT_HANDS {
            assert!(commits.contains(&Some(h)), "{h:?} comes up");
        }
        let some = (0..400u64)
            .filter(|&i| {
                new_run();
                with_options(&["commit=0.25"], || committed(i << 20).is_some())
            })
            .count();
        assert!((60..140).contains(&some), "about a quarter commit: {some}");
        // Committed to Straight: from ante 3 its Planet is taken over the most played hand's; before, the run plays
        // as usual.
        let i = (0..400u64)
            .find(|&i| commits[i as usize] == Some(HandType::Straight))
            .unwrap();
        let celestial = BOOSTERS
            .iter()
            .position(|b| b.kind == PackKind::Celestial)
            .unwrap() as u8;
        let cards = [Mercury, Saturn]
            .iter()
            .map(|&k| PackCard::Consumable(run.make_consumable(k)))
            .collect();
        run.pack = Some(crate::packs::OpenPack {
            booster: celestial,
            cards,
            choices_left: 1,
            return_to: Phase::Shop,
        });
        run.phase = Phase::Pack;
        new_run();
        assert_eq!(
            focus_pick(&run, i << 20),
            Some(0),
            "Mercury, the Pair's, without commit"
        );
        for ante in [1, 2] {
            run.ante = ante;
            new_run();
            with_options(&["commit=1"], || {
                assert_eq!(focus_pick(&run, i << 20), Some(0), "ante {ante}: not yet")
            });
        }
        run.ante = COMMIT_ANTE;
        new_run();
        with_options(&["commit=1"], || {
            assert_eq!(focus_pick(&run, i << 20), Some(1), "Saturn, the Straight's")
        });
        new_run();
        assert_eq!(
            COMMIT.with(|c| c.get()),
            None,
            "forgotten at the next run's start"
        );
    }

    #[test]
    fn the_late_antes_get_more_samples_with_late_x_and_the_scales_are_restored_after() {
        use crate::value::{scales, Scales};
        let usual = scales();
        // A probe the teacher runs inside its scaled move: `scales()` as a decision in the late antes sees it.
        thread_local! { static SEEN: std::cell::Cell<Option<Scales>> = const { std::cell::Cell::new(None) }; }
        // A real run at its first shop (the teacher's move there is judged by value).
        new_run();
        let mut run = Run::new(RunConfig::new("LATE1"));
        let mut step = 0;
        while run.phase != Phase::Shop && step < 400 {
            let (k, a) = with_options(&["light"], || teacher_move(&run, step));
            apply(&mut run, k, a).expect("legal");
            step += 1;
        }
        assert!(run.phase == Phase::Shop && run.ante < 6);
        with_options(&["light", "late-x=3"], || {
            let m = teacher_move(&run, 1);
            assert!(crate::env::apply(&mut run.clone(), m.0, m.1).is_ok());
            assert_eq!(
                scales(),
                Scales::all(0.5),
                "restored (and this ante isn't late)"
            );
            let mut late = run.clone();
            late.ante = 6;
            let m = teacher_move(&late, 1);
            assert!(crate::env::apply(&mut late.clone(), m.0, m.1).is_ok());
            assert_eq!(
                scales(),
                Scales::all(0.5),
                "restored after the scaled decision"
            );
            crate::value::with_scales(
                Scales {
                    full: 1.5,
                    quick: 1.5,
                    looks: 1.5,
                    rounds: 3.0,
                },
                || SEEN.with(|c| c.set(Some(scales()))),
            );
            assert_eq!(SEEN.with(|c| c.get()).map(|s| s.rounds), Some(3.0));
        });
        with_options(&["late-x=2", "late-from=3"], || {
            let mut r = run.clone();
            r.ante = 3;
            let m = teacher_move(&r, 1);
            assert!(crate::env::apply(&mut r.clone(), m.0, m.1).is_ok());
        });
        assert_eq!(scales(), usual);
    }

    #[test]
    fn boss_x_scales_the_counts_in_a_boss_blinds_round_only() {
        use crate::value::{scales, Scales};
        thread_local! { static SEEN: std::cell::Cell<Option<Scales>> = const { std::cell::Cell::new(None) }; }
        // A probe in place of the decision: what `scales()` reads inside `teacher_move`'s scaled block is what
        // `with_scales` set; the move itself is then made with the usual counts restored.
        let boss = Run::scenario(
            "BOSS1",
            crate::blinds::BlindKind::Boss(crate::blinds::Boss::Wall),
        );
        let small = Run::scenario("BOSS1", crate::blinds::BlindKind::Small);
        with_options(&["boss-x=4"], || {
            let probe = |run: &Run| {
                let x = param("boss-x").unwrap_or(1.0);
                let in_boss = run.phase == Phase::Round
                    && run
                        .blind
                        .as_ref()
                        .is_some_and(|b| matches!(b.kind, crate::blinds::BlindKind::Boss(_)));
                if x > 1.0 && in_boss {
                    let s = scales();
                    crate::value::with_scales(
                        Scales {
                            full: s.full * x,
                            quick: s.quick * x,
                            looks: s.looks * x,
                            rounds: s.rounds * x,
                        },
                        || SEEN.with(|c| c.set(Some(scales()))),
                    );
                } else {
                    SEEN.with(|c| c.set(Some(scales())));
                }
            };
            probe(&boss);
            assert_eq!(
                SEEN.with(|c| c.get()),
                Some(Scales::all(4.0)).map(|mut s| {
                    s.rounds = 4.0;
                    s
                })
            );
            probe(&small);
            assert_eq!(SEEN.with(|c| c.get()), Some(Scales::USUAL));
            assert_eq!(scales(), Scales::USUAL, "restored");
        });
        // The real thing: a legal move in a boss round with the option on, and nothing left scaled after it.
        let mut run = boss.clone();
        for _ in 0..8 {
            run.add_card_to_hand(crate::card::Suit::Hearts, crate::card::Rank::Ten);
        }
        with_options(&["light", "boss-x=4"], || {
            let m = teacher_move(&run, 1);
            assert!(apply(&mut run.clone(), m.0, m.1).is_ok());
            assert_eq!(scales(), Scales::all(0.5));
        });
    }

    #[test]
    fn exploring_takes_another_option_rated_nearly_as_high() {
        let run = Run::scenario("EXPLORE1", crate::blinds::BlindKind::Small);
        let a = Some((MoveKind::Buy, 0));
        let b = Some((MoveKind::Buy, 1));
        let c = Some((MoveKind::Reroll, 0));
        let judged = [(1.0, None), (1.5, a), (1.4, b), (0.5, c)];
        let best = (1.5, a);
        assert_eq!(explore(&run, 9, &judged, best), best, "off");
        with_options(&["explore=1"], || {
            for vs in 0..20 {
                let got = explore(&run, vs, &judged, best);
                assert!(
                    got.1 == b,
                    "only the option within 0.25 of the best: {got:?}"
                );
                assert_eq!(got, explore(&run, vs, &judged, best), "the same each time");
            }
        });
        with_options(&["explore=1", "explore-within=0.6"], || {
            let picks: std::collections::HashSet<_> = (0..40)
                .map(|vs| format!("{:?}", explore(&run, vs, &judged, best).1))
                .collect();
            assert_eq!(
                picks.len(),
                2,
                "doing nothing and the second option, not the one rated 1.0 below"
            );
        });
        with_options(&["explore=0.1"], || {
            let n = (0..2000)
                .filter(|&vs| explore(&run, vs, &judged, best) != best)
                .count();
            assert!(
                (120..280).contains(&n),
                "about one decision in ten: {n} of 2,000"
            );
        });
    }

    #[test]
    fn a_skip_is_played_out_to_where_its_tag_has_acted() {
        use crate::blinds::{BlindSlot, SlotState};
        use crate::tags::TagKind;
        let mut run = Run::new(RunConfig::new("SKIPCHK1"));
        assert_eq!(run.phase, Phase::BlindSelect);
        assert_eq!(run.on_deck(), BlindSlot::Small);
        // A tag that acts in the next shop is spent by the Boss choice when the Small Blind is skipped (the Big
        // Blind's shop comes first); Investment and Double go to the next ante.
        run.tags_offered[0] = TagKind::Foil;
        assert_eq!(skip_checkpoint(&run), (1, true));
        run.tags_offered[0] = TagKind::Investment;
        assert_eq!(skip_checkpoint(&run), (1, false));
        run.tags_offered[0] = TagKind::Double;
        assert_eq!(skip_checkpoint(&run), (1, false));
        // Skipping the Big Blind: the next shop is after the boss, so a shop tag goes to the next ante; a pack or
        // money has acted by the Boss choice.
        run.slots[0] = SlotState::Defeated;
        run.slots[1] = SlotState::Select;
        assert_eq!(run.on_deck(), BlindSlot::Big);
        run.tags_offered[1] = TagKind::Foil;
        assert_eq!(skip_checkpoint(&run), (1, false));
        run.tags_offered[1] = TagKind::Charm;
        assert_eq!(skip_checkpoint(&run), (1, true));
        // At the last ante there is no next ante: the Boss choice always.
        run.ante = run.win_ante;
        run.tags_offered[1] = TagKind::Investment;
        assert_eq!(skip_checkpoint(&run), (run.win_ante, true));
        // Where a play-out stops.
        let mut at_boss = Run::new(RunConfig::new("SKIPCHK1"));
        at_boss.slots = [SlotState::Defeated, SlotState::Defeated, SlotState::Select];
        assert!(at_checkpoint(&at_boss, (1, true)));
        assert!(!at_checkpoint(&at_boss, (1, false)));
        let mut next = Run::new(RunConfig::new("SKIPCHK1"));
        next.ante = 2;
        assert!(at_checkpoint(&next, (1, true)));
        assert!(at_checkpoint(&next, (1, false)));
        assert!(
            !at_checkpoint(&at_boss, (2, true)),
            "an earlier ante's boss choice is not the next ante's"
        );
        // Tags never worth a blind.
        assert!(!tag_worth_weighing(TagKind::Juggle));
        assert!(!tag_worth_weighing(TagKind::D6));
        assert!(tag_worth_weighing(TagKind::Investment));
        assert!(tag_worth_weighing(TagKind::Negative));
    }

    #[test]
    fn skips_are_only_weighed_at_the_top_and_never_at_the_boss() {
        use crate::blinds::{BlindSlot, SlotState};
        use crate::tags::TagKind;
        new_run();
        let mut run = Run::new(RunConfig::new("SKIPTOP1"));
        run.tags_offered = [TagKind::Juggle, TagKind::Investment];
        with_options(&["light", "skips"], || {
            // A Juggle tag is never weighed: the blind is played without play-outs.
            assert_eq!(value_blind_move(&run, 7), (MoveKind::SelectBlind, 0));
            // Inside a play-out, nothing is skipped (no forking within a fork).
            run.slots = [SlotState::Defeated, SlotState::Select, SlotState::Upcoming];
            SKIP_DEPTH.with(|d| d.set(1));
            assert_eq!(value_blind_move(&run, 7), (MoveKind::SelectBlind, 0));
            SKIP_DEPTH.with(|d| d.set(0));
            // The Boss Blind can't be skipped.
            run.slots = [SlotState::Defeated, SlotState::Defeated, SlotState::Select];
            assert_eq!(run.on_deck(), BlindSlot::Boss);
            assert_eq!(value_blind_move(&run, 7), (MoveKind::SelectBlind, 0));
        });
        // Without the option, the blind is played.
        let plain = Run::new(RunConfig::new("SKIPTOP1"));
        with_options(&["light"], || {
            assert_eq!(value_blind_move(&plain, 7), (MoveKind::SelectBlind, 0))
        });
    }

    #[test]
    fn a_skip_play_out_reaches_the_checkpoint_and_values_it() {
        use crate::tags::TagKind;
        new_run();
        let run = Run::new(RunConfig::new("SKIPOUT1"));
        with_options(&["light"], || {
            let here = crate::value::value_settled(&run, 3);
            // The blind played, to this ante's Boss choice: a living position's value, not a loss.
            let played = play_to(run.clone(), (MoveKind::SelectBlind, 0), 3, (1, true), here);
            assert!(played > SKIP_LOST + 1.0 && played < SKIP_WON, "{played}");
            // The blind skipped for a Foil tag, to the same place.
            let mut r = run.clone();
            r.tags_offered[0] = TagKind::Foil;
            let skipped = play_to(r, (MoveKind::SkipBlind, 0), 3, (1, true), here);
            assert!(skipped > SKIP_LOST + 1.0 && skipped < SKIP_WON, "{skipped}");
            assert_eq!(SKIP_DEPTH.with(|d| d.get()), 0, "the depth is restored");
        });
    }
}
