//! A bot's baseline: it plays fresh-seed runs on every thread and reports as it goes.
//!
//!     cargo run --release -p bm-core --example baseline -- [--bot greedy|teacher] [--runs N] [--seconds S]
//!         [--threads T] [--seed BASE] [--out results/baseline/greedy.json]
//!         [--value-net models/value-1.bin] [--value-weight W] [--record DIR]
//!
//! `greedy` is the simple bot (bots.rs); `teacher` the rule-based one written to teach the network
//! (teacher.rs). `--value-net` gives the teacher a learned value (learned.rs; `--value-weight` scales it).
//!
//! `--record DIR` keeps the bot's training data for the learned value: every position the teacher values
//! between rounds (blind choice, shop, open packs) as the value network's input (sparse, half precision),
//! labelled with how its run ended. Chunks of 250 runs in run order (`chunk-NNNNN-{idx,val,ptr,lab}.npy`
//! and `-runs.json`), then `manifest.json` once every run is in, so the same seeds give the same files.
//! Each chunk also keeps every move of its runs (`-mkind.npy`, `-marg.npy`; `runs.json` gives each run's
//! count), and the manifest says so (`"moves_kept": true`).
//! `--record-moves DIR` keeps every move instead, for the network that learns to play (`make train`): each
//! position's screen input (`observe::encode`), the move kinds it allowed and the bot's move
//! (`-mov.npy`: kind, argument, legal kinds as bits), in the same chunks.
//!
//! `--replay DIR --record NEW` plays the runs of a recording made with `--record` again, each by its own
//! moves (no bot, no search: seconds for a whole round), and records their positions as this code encodes
//! them: a new value input needs no new recording. Each run must end as it was recorded, after the same
//! moves; if one doesn't (the engine's rules changed since), it says which and leaves NEW without a
//! manifest. The manifest keeps how the runs were played (bot, seed base, networks, options).
//!
//! `--offsets` (with `--record` and `--value-net`) also keeps, for every recorded position, the value networks'
//! log-odds of winning as the teacher uses them (`learned::base_logit`), in `chunk-NNNNN-off.npy`: what a residual
//! stake correction learns on top of (`train_value.py --offset`). `--offsets-into DIR --value-net NETS` adds them
//! to a recording made without: its runs are played again by their moves (each must end as recorded, with as many
//! positions), and only `-off.npy` files and `offsets.json` are written into DIR, nothing else touched.
//! `--stake-net NETS` (input layout 3) corrects the value networks' chance by the stake (`learned::StakeMode`):
//! `--stake-mode residual` (the default: networks trained with `--offset`) or `delta`.
//!
//! `--forks DIR` (with `--bot teacher`, usually on a `--replay`) forks decisions as the runs go (`forks.rs`): at a
//! share `--fork-chance` (0.1) of the shops entered and packs opened in antes `--fork-antes` (2-6), the teacher's
//! `--fork-best` (3) best options and doing nothing (and its own choice and the run's move, if others) are each
//! played out to the end by the teacher on `--fork-copies` (6) fair copies that share their seeds across the
//! options (`--fork-play-options light`: the play-outs with that option added, the same for every option). DIR gets the positions the options leave and how each play-out ended, in the recording's chunks
//! (`chunk-NNNNN-{idx,val,ptr,lab}.npy`, `FORK_LABELS`), for `train_value.py --forks`; the report says how often
//! the teacher's own choice was the option that went best and what its choices gave away (`forks`).
//!
//! `--diagnose OUT.csv` (usually on a `--replay`, with the teacher's networks) writes one row per run about how
//! it went: where it ended and by how much it fell short, its build then, and ante by ante its biggest hand, money
//! and x mult jokers; for a share `--deep-share` (0.25) of the lost runs, the blind they lost is played again from
//! its start by the teacher with `--deep` (4) times the usual samples, to tell a build that couldn't have scored it
//! from play that fell short (`scripts/diagnose.py` reads the table). A diagnosis, never the bot's play or its
//! training: that replay sees the run's own cards.
//!
//! With `BM_WATCH_DIR` set (the job runner sets it), it writes there for the live viewer:
//! `progress.json` about once a second (runs, runs a second, win rate, final antes, what ended the
//! runs, the jokers bought and held, the hands played, the best runs so far), and runs to replay in
//! `samples/` (a seed and every move; the viewer replays them move by move with the same engine):
//! the best runs so far (`best-*.json`, kept up to date as better runs turn up) and every few seconds a
//! typical one picked at random (`typical-*.json`). Best means: won first, and among wins the biggest
//! single hand; among losses, the furthest blind reached, then how close the run came to its target.
//! The final numbers go to `--out`; for up to 50,000 runs they include `run_stages`, one letter per run in
//! seed order for paired comparisons between bots on the same seeds: the blind the run ended at, `A` (ante
//! 1's small blind) to `X` (ante 8's boss), or `Y` for a win.
//!
//! Seeds come from a fixed sampler (`--seed`), so a rerun plays the same runs. The bot sees only what
//! the screen shows (bots.rs works on fair copies).

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bm_core::blinds::BlindKind;
use bm_core::bots::greedy_move;
use bm_core::env::{apply, legal_kinds, run_seed, MoveKind};
use bm_core::forks::{fork, Fork};
use bm_core::joker_data::{JokerKind, N_JOKERS};
use bm_core::learned::{self, le_bytes, npy_bytes, Rows};
use bm_core::observe::{
    encode, encode_value, screen, OBS_LEN, VALUE_INPUT_VERSION, VALUE_LAYOUTS, VALUE_LEN,
};
use bm_core::packs::PackCard;
use bm_core::poker::HandType;
use bm_core::shop::RowItem;
use bm_core::state::STAKES;
use bm_core::teacher::teacher_move;
use bm_core::{Phase, Run, RunConfig};

const MAX_STEPS: u64 = 3000;
const SAMPLE_EVERY: Duration = Duration::from_secs(3);
const KEEP_TYPICAL: usize = 40;
const KEEP_BEST: usize = 30;

/// One run's summary.
#[derive(Default)]
struct RunSummary {
    seed: String,
    won: bool,
    /// Final ante (the ante lost in; 9 for a win).
    end: usize,
    lost_to: &'static str,
    moves: u64,
    blinds_beaten: u64,
    money_end: i64,
    rerolls: u64,
    skips: u64,
    packs: u64,
    vouchers: u64,
    refused: bool,
    bought: Vec<JokerKind>,
    held: Vec<JokerKind>,
    hands: [u64; 12],
    /// The biggest single hand of the run (Balatro's "best hand").
    best_hand: f64,
    /// Blinds reached: (ante - 1) * 3 + the blind lost to (0 small, 1 big, 2 boss); 24 for a win.
    stage: u32,
    /// How close the run came to the target of the blind it lost to (1 for a win).
    closeness: f64,
    /// Every move, when this run may become a sample.
    record: Option<Vec<(u8, u64)>>,
    /// One CSV row about how the run ended (`--runs-csv`), for studying where the bot falls short.
    detail: Option<String>,
    /// The positions the teacher valued (`--record`).
    recorded: Option<Recorded>,
    /// `--replay`: the recorded moves ran out before the run ended.
    unfinished: bool,
    /// `--endless`, for a win: blinds beaten after the win before the run was lost (0 to `ENDLESS_DEPTH`; that
    /// many if it reached `ENDLESS_LAST_ANTE`'s end).
    endless: Option<u32>,
    /// `--forks`: the decisions forked, with the step each was at.
    forks: Vec<(u64, Fork)>,
    /// `--diagnose`: the run's row of the table.
    diagnosis: Option<String>,
}

impl RunSummary {
    /// Better runs sort higher: won first; among wins the biggest hand; among losses the furthest
    /// blind, then the closest to its target, then the biggest hand.
    fn key(&self) -> (bool, u32, f64, f64) {
        if self.won {
            (true, self.stage, self.best_hand, self.best_hand)
        } else {
            (false, self.stage, self.closeness, self.best_hand)
        }
    }

    fn better_than(&self, o: &RunSummary) -> bool {
        let (a, b) = (self.key(), o.key());
        (a.0, a.1)
            .cmp(&(b.0, b.1))
            .then(a.2.total_cmp(&b.2))
            .then(a.3.total_cmp(&b.3))
            == std::cmp::Ordering::Greater
    }

    fn result(&self) -> String {
        if self.won {
            format!("won the run, best hand {}", thousands(self.best_hand))
        } else {
            format!(
                "lost at ante {} to {} ({:.0}% of its target)",
                self.end,
                self.lost_to,
                100.0 * self.closeness
            )
        }
    }
}

fn thousands(x: f64) -> String {
    let s = format!("{:.0}", x.max(0.0));
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn blind_name(kind: BlindKind) -> &'static str {
    match kind {
        BlindKind::Small => "Small Blind",
        BlindKind::Big => "Big Blind",
        BlindKind::Boss(b) => b.name(),
    }
}

/// A bot: its move for a run (it sees only the screen and fair copies).
type Bot = fn(&Run, u64) -> (MoveKind, u64);

/// One run by `bot`, start to end.
/// The header of the `--runs-csv` table (one row per run, written by `detail_row`).
const DETAIL_HEADER: &str = "seed,won,ante,lost_to,stage,closeness,money_end,jokers,n_jokers,n_xmult,\
main_hand,main_level,hands_played,rerolls,packs,vouchers,skips,best_hand,deck,hand_size,joker_slots";

fn detail_row(run: &Run, s: &RunSummary) -> String {
    let jokers: Vec<&str> = run.jokers.iter().map(|j| j.kind.name()).collect();
    let n_xmult = run.jokers.iter().filter(|j| j.x_mult > 1.0).count();
    let (main, played) = HandType::ALL
        .iter()
        .map(|&h| (h, run.hand_levels.get(h).played))
        .max_by_key(|&(h, n)| (n, std::cmp::Reverse(h as usize)))
        .unwrap_or((HandType::HighCard, 0));
    format!(
        "{},{},{},{},{},{:.3},{},{},{},{},{},{},{},{},{},{},{},{:.0},{},{},{}",
        s.seed,
        u8::from(s.won),
        s.end,
        s.lost_to,
        s.stage,
        s.closeness,
        run.money,
        jokers.join(";"),
        jokers.len(),
        n_xmult,
        main.name(),
        run.hand_levels.get(main).level,
        played,
        s.rerolls,
        s.packs,
        s.vouchers,
        s.skips,
        s.best_hand,
        run.playing_cards.len(),
        run.hand_size_real,
        run.joker_slots
    )
}

/// `script`: the run's recorded moves (`--replay`), played in place of the bot's.
fn play(seed: String, salt: u64, keep: bool, bot: Bot, script: Option<&[(u8, u64)]>) -> RunSummary {
    bm_core::teacher::new_run();
    let mut run = Run::new(RunConfig::at_stake(&seed, STAKE.load(Ordering::Relaxed)));
    let mut s = RunSummary {
        record: keep.then(Vec::new),
        recorded: RECORD.load(Ordering::Relaxed).then(Recorded::default),
        ..RunSummary::default()
    };
    let mut input = Vec::with_capacity(VALUE_LEN);
    let moves = RECORD_MODE.load(Ordering::Relaxed) == 2;
    let endless = ENDLESS.load(Ordering::Relaxed);
    // `--endless`: the run at its win and what the summary had counted by then (the reports count up to the win).
    let mut at_win: Option<(Run, WinCounts)> = None;
    let forking = FORKS.load(Ordering::Relaxed);
    let diagnose = DIAGNOSE.load(Ordering::Relaxed);
    // `--diagnose`: the run at the start of the blind it is in, its biggest hand in that blind, and ante by ante its
    // biggest hand, its money and its x mult jokers as the ante began.
    let mut diag = diagnose.then(Diagnosis::default);
    let mut prev_phase = run.phase;
    let mut step = 0u64;
    while run.phase != Phase::GameOver
        && !run.won
        && step < MAX_STEPS
        && !(at_win.is_some() && run.ante > ENDLESS_LAST_ANTE)
    {
        if let Some(d) = diag.as_mut() {
            d.ante_begins(&run);
        }
        if forking
            && at_win.is_none()
            && (run.phase == Phase::Shop || run.phase == Phase::Pack)
            && prev_phase != run.phase
            && fork_here(&run, salt, step)
        {
            let made = script.and_then(|m| m.get(step as usize)).map(|&(k, a)| {
                (
                    MoveKind::from_index(i64::from(k)).expect("a recorded move's kind"),
                    a,
                )
            });
            let (best, copies) = (
                FORK_BEST.load(Ordering::Relaxed) as usize,
                FORK_COPIES.load(Ordering::Relaxed) as usize,
            );
            let fork_salt = (salt << 12 | step) ^ (FORK_SALT.load(Ordering::Relaxed) << 36);
            if let Some(f) = fork(&run, made, fork_salt, best, copies) {
                s.forks.push((step, f));
            }
        }
        prev_phase = run.phase;
        let keep_this = match s.recorded.as_mut() {
            Some(r) if at_win.is_none() && (moves || learned::values_here(&run)) => {
                if moves {
                    encode(&screen(&run), &mut input);
                } else {
                    encode_value(&screen(&run), &mut input);
                }
                r.rows.push(&input);
                if OFFSETS.load(Ordering::Relaxed) {
                    r.off
                        .push(learned::base_logit(&input).expect("--offsets needs --value-net"));
                }
                r.at.push([
                    step as f32,
                    phase_code(run.phase),
                    run.ante as f32,
                    stage_now(&run),
                ]);
                true
            }
            _ => false,
        };
        let (k, a) = match script {
            None => bot(&run, salt << 20 | step),
            Some(m) => match m.get(step as usize) {
                Some(&(k, a)) => (
                    MoveKind::from_index(i64::from(k)).expect("a recorded move's kind"),
                    a,
                ),
                None => {
                    s.unfinished = true;
                    break;
                }
            },
        };
        if keep_this && moves {
            let legal = legal_kinds(&run)
                .iter()
                .enumerate()
                .fold(0i64, |m, (i, &ok)| m | (i64::from(ok) << i));
            if let Some(r) = s.recorded.as_mut() {
                r.moves.push([k as i64, a as i64, legal]);
            }
        }
        match k {
            MoveKind::Buy | MoveKind::BuyAndUse => {
                if let Some(slot) = run.shop.as_ref().and_then(|sh| sh.row.get(a as usize)) {
                    if let RowItem::Joker(j) = &slot.item {
                        s.bought.push(j.kind);
                    }
                }
            }
            MoveKind::PackChoose => {
                if let Some(PackCard::Joker(j)) = run
                    .pack
                    .as_ref()
                    .and_then(|p| p.cards.get((a & 0xFF) as usize))
                {
                    s.bought.push(j.kind);
                }
            }
            MoveKind::Reroll => s.rerolls += 1,
            MoveKind::SkipBlind => s.skips += 1,
            MoveKind::OpenBooster => s.packs += 1,
            MoveKind::RedeemVoucher => s.vouchers += 1,
            MoveKind::CashOut => {
                s.blinds_beaten += 1;
                if let Some(d) = diag.as_mut() {
                    d.blind_beaten();
                }
            }
            _ => {}
        }
        let chips_before = run.round_chips;
        if let Err(why) = apply(&mut run, k, a) {
            // Never expected: the bots only make legal moves (tested). Say which, for a fix.
            eprintln!(
                "refused: seed {seed}, move {step}: {k:?} {a:#x} in {:?}: {}",
                run.phase, why.0
            );
            s.refused = true;
            break;
        }
        if k == MoveKind::Play {
            if let Some(h) = run.last_hand {
                s.hands[h as usize] += 1;
            }
            let scored = if run.round_chips >= chips_before {
                run.round_chips - chips_before
            } else {
                run.round_chips
            };
            s.best_hand = s.best_hand.max(scored);
            if let Some(d) = diag.as_mut() {
                d.hand_scored(&run, scored);
            }
        }
        if k == MoveKind::SelectBlind && run.phase == Phase::Round {
            if let Some(d) = diag.as_mut() {
                d.blind_begins(&run);
            }
        }
        if let Some(r) = s.record.as_mut() {
            r.push((k as u8, a));
        }
        step += 1;
        if endless && run.won && at_win.is_none() {
            // Played on: the win is kept as it stands, the run aims for the next showdown ante.
            at_win = Some((run.clone(), WinCounts::of(&s)));
            run.won = false;
            run.win_ante = ENDLESS_WIN_ANTE;
        }
    }
    s.seed = seed;
    s.moves = step;
    if let Some((won_run, counts)) = at_win {
        s.endless = Some(endless_depth(&run));
        counts.restore(&mut s);
        run = won_run;
    }
    s.won = run.won;
    s.end = if run.won {
        9
    } else {
        run.ante.clamp(1, 8) as usize
    };
    s.lost_to = if run.won {
        ""
    } else {
        run.blind.as_ref().map_or("unknown", |b| blind_name(b.kind))
    };
    if run.won {
        s.stage = 24;
        s.closeness = 1.0;
    } else {
        let slot = run.blind.as_ref().map_or(0, |b| match b.kind {
            BlindKind::Small => 0,
            BlindKind::Big => 1,
            BlindKind::Boss(_) => 2,
        });
        s.stage = (run.ante.clamp(1, 8) as u32 - 1) * 3 + slot;
        s.closeness = run.blind.as_ref().map_or(0.0, |b| {
            if b.chips > 0.0 {
                (run.round_chips / b.chips).clamp(0.0, 1.0)
            } else {
                0.0
            }
        });
    }
    s.money_end = run.money;
    s.held = run.jokers.iter().map(|j| j.kind).collect();
    if DETAIL.load(Ordering::Relaxed) {
        s.detail = Some(detail_row(&run, &s));
    }
    if let Some(d) = diag {
        s.diagnosis = Some(d.row(&run, &s, salt));
    }
    s
}

/// Whether to fork the decision at this step of this run (`--fork-chance`): by a hash, so a run forks the same
/// decisions on any thread.
fn fork_here(run: &Run, salt: u64, step: u64) -> bool {
    let (lo, hi) = (
        FORK_ANTE_LO.load(Ordering::Relaxed),
        FORK_ANTE_HI.load(Ordering::Relaxed),
    );
    if run.ante < i32::from(lo) || run.ante > i32::from(hi) {
        return false;
    }
    let h = (salt.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ step.wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ FORK_SALT
            .load(Ordering::Relaxed)
            .wrapping_mul(0x94D0_49BB_1331_11EB))
    .rotate_left(23)
    .wrapping_mul(0xD6E8_FEB8_6659_FD93);
    (h % 1_000_000) < FORK_CHANCE_PPM.load(Ordering::Relaxed)
}

/// `--diagnose`: what a run's row of the table is built from as the run goes.
#[derive(Default)]
struct Diagnosis {
    /// The run as the blind it is in began (the blind it loses is played again from here, `--deep`).
    blind_start: Option<Run>,
    /// The biggest hand of the blind it is in.
    blind_best: f64,
    /// By ante (1 to 8): the biggest hand, and money, x mult jokers and jokers as the ante began.
    ante_best: [f64; 9],
    ante_money: [i64; 9],
    ante_xmult: [u8; 9],
    ante_jokers: [u8; 9],
    seen_ante: i32,
    /// The hand type of the biggest hand in the blind it is in, and how many blinds each hand type cleared as
    /// the blind's biggest hand (the scoring hands, not the throwaway plays).
    blind_best_hand: Option<HandType>,
    win_hands: [u32; 12],
    /// The hand type of the run's biggest hand.
    best_hand_type: Option<HandType>,
    best_hand: f64,
}

/// Jokers whose x mult is above one now (a scaling joker counts once it has grown).
fn xmult_jokers(run: &Run) -> usize {
    run.jokers.iter().filter(|j| j.x_mult > 1.0).count()
}

impl Diagnosis {
    fn ante_begins(&mut self, run: &Run) {
        if run.ante > self.seen_ante && (1..=8).contains(&run.ante) {
            self.seen_ante = run.ante;
            let a = run.ante as usize;
            self.ante_money[a] = run.money;
            self.ante_xmult[a] = xmult_jokers(run) as u8;
            self.ante_jokers[a] = run.jokers.len() as u8;
        }
    }

    fn blind_begins(&mut self, run: &Run) {
        self.blind_start = Some(run.clone());
        self.blind_best = 0.0;
        self.blind_best_hand = None;
    }

    fn hand_scored(&mut self, run: &Run, scored: f64) {
        if scored >= self.blind_best {
            self.blind_best = scored;
            self.blind_best_hand = run.last_hand;
        }
        if scored >= self.best_hand {
            self.best_hand = scored;
            self.best_hand_type = run.last_hand;
        }
        if (1..=8).contains(&run.ante) {
            let a = run.ante as usize;
            self.ante_best[a] = self.ante_best[a].max(scored);
        }
    }

    /// The blind was beaten: its biggest hand's type cleared it.
    fn blind_beaten(&mut self) {
        if let Some(h) = self.blind_best_hand {
            self.win_hands[h as usize] += 1;
        }
    }

    /// The lost blind played again from its start by the teacher with `DEEP` times the usual samples (the run's
    /// own cards: a diagnosis of the play, never the bot's play): whether it beat the blind, and its biggest hand.
    fn deep(&self, salt: u64) -> Option<(bool, f64)> {
        let x = DEEP_X.load(Ordering::Relaxed) as f64 / 100.0;
        let start = self.blind_start.as_ref()?;
        if x <= 1.0 {
            return None;
        }
        let s = bm_core::value::scales();
        let scaled = bm_core::value::Scales {
            full: s.full * x,
            quick: s.quick * x,
            looks: s.looks * x,
            rounds: s.rounds * x,
        };
        bm_core::value::with_scales(scaled, || {
            bm_core::teacher::new_run();
            let mut run = start.clone();
            let mut best = 0.0f64;
            let mut step = 0u64;
            while run.phase == Phase::Round && step < 400 {
                let (k, a) = teacher_move(&run, (salt ^ 0xDEE9) << 20 | step);
                let before = run.round_chips;
                if apply(&mut run, k, a).is_err() {
                    break;
                }
                if k == MoveKind::Play {
                    let scored = if run.round_chips >= before {
                        run.round_chips - before
                    } else {
                        run.round_chips
                    };
                    best = best.max(scored);
                }
                step += 1;
            }
            bm_core::teacher::new_run();
            Some((
                run.phase != Phase::GameOver && run.phase != Phase::Round,
                best,
            ))
        })
    }

    fn row(&self, run: &Run, s: &RunSummary, salt: u64) -> String {
        let needed = run.blind.as_ref().map_or(0.0, |b| b.chips);
        let (main, _) = HandType::ALL
            .iter()
            .map(|&h| (h, run.hand_levels.get(h).played))
            .max_by_key(|&(h, n)| (n, std::cmp::Reverse(h as usize)))
            .unwrap_or((HandType::HighCard, 0));
        let xmult_total: f64 = run
            .jokers
            .iter()
            .filter(|j| j.x_mult > 1.0)
            .map(|j| j.x_mult)
            .product();
        let deep = if s.won {
            None
        } else {
            let share = DEEP_SHARE_PPM.load(Ordering::Relaxed);
            let h = salt.wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(31) % 1_000_000;
            (h < share).then(|| self.deep(salt)).flatten()
        };
        let by_ante = |f: &dyn Fn(usize) -> String| (1..=8).map(f).collect::<Vec<_>>().join(",");
        // The hand type that cleared most blinds as the blind's biggest hand (ties to the higher type).
        let (win_hand, win_blinds) = HandType::ALL
            .iter()
            .map(|&h| (h, self.win_hands[h as usize]))
            .max_by_key(|&(h, n)| (n, h as usize))
            .unwrap_or((HandType::HighCard, 0));
        let jokers: Vec<&str> = run.jokers.iter().map(|j| j.kind.name()).collect();
        format!(
            "{},{},{},{},{},{},{:.3},{:.0},{:.0},{:.0},{},{},{},{},{},{:.3},{},{},{},{},{:.0},{},{},{},{},{},{},{},{},{},{},{},{},{:.0}",
            s.seed,
            salt,
            u8::from(s.won),
            s.end,
            s.lost_to,
            s.stage,
            s.closeness,
            needed,
            run.round_chips,
            self.blind_best,
            run.hands_left,
            run.discards_left,
            run.money,
            run.jokers.len(),
            xmult_jokers(run),
            xmult_total,
            main.name(),
            run.hand_levels.get(main).level,
            run.playing_cards.len(),
            run.hand_size_real,
            s.best_hand,
            by_ante(&|a| format!("{:.0}", self.ante_best[a])),
            by_ante(&|a| self.ante_money[a].to_string()),
            by_ante(&|a| self.ante_xmult[a].to_string()),
            by_ante(&|a| self.ante_jokers[a].to_string()),
            win_hand.name(),
            win_blinds,
            run.hand_levels.get(win_hand).level,
            self.best_hand_type.map_or("", |h| h.name()),
            jokers.join(";"),
            run.vouchers.iter().filter(|&&v| v).count(),
            deep.map_or("-1".to_string(), |(w, _)| u8::from(w).to_string()),
            deep.map_or(-1.0, |(_, b)| b),
            needed
        )
    }
}

/// The header of `--diagnose`'s table (`Diagnosis::row`).
const DIAGNOSIS_HEADER: &str = "seed,run,won,ante,lost_to,stage,closeness,needed,scored,blind_best,hands_left,\
discards_left,money_end,n_jokers,n_xmult,xmult_total,main_hand,main_level,deck,hand_size,best_hand,\
ante_best_1,ante_best_2,ante_best_3,ante_best_4,ante_best_5,ante_best_6,ante_best_7,ante_best_8,\
ante_money_1,ante_money_2,ante_money_3,ante_money_4,ante_money_5,ante_money_6,ante_money_7,ante_money_8,\
ante_xmult_1,ante_xmult_2,ante_xmult_3,ante_xmult_4,ante_xmult_5,ante_xmult_6,ante_xmult_7,ante_xmult_8,\
ante_jokers_1,ante_jokers_2,ante_jokers_3,ante_jokers_4,ante_jokers_5,ante_jokers_6,ante_jokers_7,ante_jokers_8,\
win_hand,win_hand_blinds,win_hand_level,best_hand_type,jokers,vouchers,deep_won,deep_best,deep_needed";

/// What a run's summary had counted when it won, so an endless run's reports count only up to its win.
struct WinCounts {
    blinds_beaten: u64,
    rerolls: u64,
    skips: u64,
    packs: u64,
    vouchers: u64,
    bought: usize,
    hands: [u64; 12],
    best_hand: f64,
}

impl WinCounts {
    fn of(s: &RunSummary) -> WinCounts {
        WinCounts {
            blinds_beaten: s.blinds_beaten,
            rerolls: s.rerolls,
            skips: s.skips,
            packs: s.packs,
            vouchers: s.vouchers,
            bought: s.bought.len(),
            hands: s.hands,
            best_hand: s.best_hand,
        }
    }

    fn restore(self, s: &mut RunSummary) {
        s.blinds_beaten = self.blinds_beaten;
        s.rerolls = self.rerolls;
        s.skips = self.skips;
        s.packs = self.packs;
        s.vouchers = self.vouchers;
        s.bought.truncate(self.bought);
        s.hands = self.hands;
        s.best_hand = self.best_hand;
    }
}

/// How far an endless run got after its win: the blinds it beat from ante 9 on (the one it lost at not counted),
/// or all of them through `ENDLESS_LAST_ANTE`.
fn endless_depth(run: &Run) -> u32 {
    if run.ante > ENDLESS_LAST_ANTE {
        return ENDLESS_DEPTH;
    }
    let ante = (run.ante.max(9) - 9) as u32 * 3;
    let here = if run.phase == Phase::GameOver {
        run.blind.as_ref().map_or(0, |b| match b.kind {
            BlindKind::Small => 0,
            BlindKind::Big => 1,
            BlindKind::Boss(_) => 2,
        })
    } else {
        learned::blinds_behind(run) as u32 % 3
    };
    (ante + here).min(ENDLESS_DEPTH)
}

/// Whether runs keep a `--runs-csv` row.
static DETAIL: AtomicBool = AtomicBool::new(false);
/// `--forks DIR`: fork decisions as the runs go (forks.rs), and how (`--fork-chance` in parts per million,
/// `--fork-antes`, `--fork-best`, `--fork-copies`).
static FORKS: AtomicBool = AtomicBool::new(false);
static FORK_CHANCE_PPM: AtomicU64 = AtomicU64::new(100_000);
static FORK_ANTE_LO: AtomicU8 = AtomicU8::new(2);
static FORK_ANTE_HI: AtomicU8 = AtomicU8::new(6);
static FORK_BEST: AtomicU64 = AtomicU64::new(3);
static FORK_COPIES: AtomicU64 = AtomicU64::new(6);
/// `--fork-salt N`: which decisions are forked and the copies' seeds differ by it, so another night on the same
/// recording forks other decisions, not the same ones again.
static FORK_SALT: AtomicU64 = AtomicU64::new(0);
/// `--diagnose OUT.csv`: a row about every run; `--deep X` (times 100) the samples the lost blind is played again
/// with, for a share `--deep-share` (parts per million) of the lost runs.
static DIAGNOSE: AtomicBool = AtomicBool::new(false);
static DEEP_X: AtomicU64 = AtomicU64::new(400);
static DEEP_SHARE_PPM: AtomicU64 = AtomicU64::new(250_000);

/// The columns of a fork recording's `lab.npy` (f32), one row per option and copy: the run and step the fork was
/// at, the fork's number in its run, the option's number and how many there were, the copy's number, the ante and
/// phase, whether the option was the teacher's own choice and the run's move, the quick value the teacher gave it,
/// how the play-out ended (won, and the blinds behind the run), and the loaded networks' log-odds for the position
/// it left (NaN without networks).
const FORK_LABELS: [&str; 14] = [
    "run",
    "step",
    "fork",
    "option",
    "options",
    "copy",
    "ante",
    "phase",
    "chosen",
    "made",
    "quick",
    "won",
    "end_stage",
    "logit",
];

/// `--endless`: a won run plays on, as the game's Endless mode lets a player, until it loses or finishes ante
/// `ENDLESS_LAST_ANTE`; how far it got is a win's measure (`RunSummary::endless`, the recordings' `endless` label).
/// The win is counted as before, and everything the reports count stops at it; the positions recorded are the
/// win's own. A replay of an endless recording plays on the same way.
static ENDLESS: AtomicBool = AtomicBool::new(false);
/// The last ante an endless run plays (the blinds grow about 5x, 13x and 40x from ante 9 to 12: few runs get
/// that far).
const ENDLESS_LAST_ANTE: i32 = 12;
/// The most blinds a win can beat after it (antes 9 to 12).
const ENDLESS_DEPTH: u32 = (ENDLESS_LAST_ANTE as u32 - 8) * 3;
/// The ante a won run aims for once it plays on: the game's next showdown boss comes at ante 16 (the boss rules
/// keep it there), and the teacher's formula plans for the blinds up to it.
const ENDLESS_WIN_ANTE: i32 = 16;

/// Whether runs keep their valued positions (`--record`).
static RECORD: AtomicBool = AtomicBool::new(false);

/// The stake every run is played at (`--stake`: 1 White ... 8 Gold).
static STAKE: AtomicU8 = AtomicU8::new(1);

/// Whether recorded positions keep the value networks' log-odds (`--offsets`, `--offsets-into`).
static OFFSETS: AtomicBool = AtomicBool::new(false);

/// Runs per chunk of recorded positions.
const RUNS_PER_CHUNK: u64 = 250;

/// The columns of a chunk's `lab.npy` (f32), one row per position.
const LABELS: [&str; 9] = [
    "run",
    "step",
    "phase",
    "ante",
    "stage",
    "won",
    "end_stage",
    "end_ante",
    "endless",
];

/// A run's valued positions: the network's input, and where in the run each was (step, phase, ante,
/// stage).
#[derive(Default)]
struct Recorded {
    rows: Rows,
    at: Vec<[f32; 4]>,
    /// With `--offsets`: each position's log-odds of winning by the value networks.
    off: Vec<f32>,
    /// With `--record-moves`: each position's move (kind, argument) and the kinds it allowed (bits).
    moves: Vec<[i64; 3]>,
    /// In a fork recording (`--forks`): each row's labels (`FORK_LABELS`), one row per option and copy.
    fork_lab: Vec<[f32; FORK_LABELS.len()]>,
}

/// A run's forks as rows for the recorder: the positions the options left, each labelled.
fn fork_rows(run: u64, s: &RunSummary) -> Recorded {
    let mut rec = Recorded::default();
    for (n, (step, f)) in s.forks.iter().enumerate() {
        for (i, o) in f.options.iter().enumerate() {
            for (c, (row, end)) in o.rows.iter().zip(&o.ends).enumerate() {
                rec.rows.push(row);
                let logit = learned::base_logit(row).unwrap_or(f32::NAN);
                rec.fork_lab.push([
                    run as f32,
                    *step as f32,
                    n as f32,
                    i as f32,
                    f.options.len() as f32,
                    c as f32,
                    f.ante as f32,
                    phase_code(f.phase),
                    f32::from(u8::from(o.chosen)),
                    f32::from(u8::from(o.made)),
                    o.quick as f32,
                    f32::from(u8::from(end.won)),
                    end.stage as f32,
                    logit,
                ]);
            }
        }
    }
    rec
}

/// What `--record` keeps: 1 the positions the teacher values, 2 every move.
static RECORD_MODE: AtomicU64 = AtomicU64::new(0);

/// The phase as `observe::encode` numbers it.
fn phase_code(p: Phase) -> f32 {
    match p {
        Phase::BlindSelect => 0.0,
        Phase::Round => 1.0,
        Phase::RoundEval => 2.0,
        Phase::Shop => 3.0,
        Phase::GameOver => 4.0,
        Phase::Pack => 5.0,
    }
}

/// Blinds behind the run: 3 per ante before this one, and this ante's beaten or skipped (0 to 23).
fn stage_now(run: &Run) -> f32 {
    learned::blinds_behind(run) as f32
}

/// One finished run's recording, waiting for the rest of its chunk.
struct RecordedRun {
    run: u64,
    seed: String,
    won: bool,
    end_stage: u32,
    end_ante: usize,
    /// `--endless`: how far a win got after it (`RunSummary::endless`); -1 when not known (a loss, or a recording
    /// that didn't play on).
    endless: f32,
    rec: Recorded,
    /// Every move of the run (kind, argument): with the seed they play the run again (`--replay`).
    moves: Vec<(u8, u64)>,
}

/// Recorded runs grouped into chunks by run number; a chunk is written once all its runs are in.
struct Recorder {
    dir: PathBuf,
    target: u64,
    open: BTreeMap<u64, Vec<RecordedRun>>,
    chunks: u64,
    rows: u64,
    runs: u64,
    /// `--offsets-into`: only each chunk's `-off.npy` (and `offsets.json`), into a recording already there.
    only_offsets: bool,
    /// `--forks`: the rows are forked options' positions, labelled with `FORK_LABELS`; no moves kept.
    forks: bool,
}

impl Recorder {
    fn new(dir: PathBuf, target: u64) -> Recorder {
        std::fs::create_dir_all(&dir).expect("make the record folder");
        // A rerun replaces the last recording.
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with("chunk-") || name == "manifest.json" {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        Recorder {
            dir,
            target,
            open: BTreeMap::new(),
            chunks: 0,
            rows: 0,
            runs: 0,
            only_offsets: false,
            forks: false,
        }
    }

    /// `--offsets-into DIR`: the recording in DIR stays as it is; only the offsets are added beside its chunks.
    fn offsets_into(dir: PathBuf, target: u64) -> Recorder {
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if (name.starts_with("chunk-") && name.ends_with("-off.npy"))
                    || name == "offsets.json"
                {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        Recorder {
            dir,
            target,
            open: BTreeMap::new(),
            chunks: 0,
            rows: 0,
            runs: 0,
            only_offsets: true,
            forks: false,
        }
    }

    fn add(&mut self, r: RecordedRun) {
        let c = r.run / RUNS_PER_CHUNK;
        let want = RUNS_PER_CHUNK.min(self.target - c * RUNS_PER_CHUNK) as usize;
        let runs = self.open.entry(c).or_default();
        runs.push(r);
        if runs.len() == want {
            let runs = self.open.remove(&c).expect("just added");
            self.write(c, runs);
        }
    }

    /// Write the chunks still open (runs stopped early by `--seconds`).
    fn finish(&mut self) {
        let open = std::mem::take(&mut self.open);
        for (c, runs) in open {
            self.write(c, runs);
        }
    }

    fn write(&mut self, chunk: u64, mut runs: Vec<RecordedRun>) {
        runs.sort_by_key(|r| r.run);
        let mut rows = Rows::default();
        let mut lab: Vec<f32> = Vec::new();
        let mut mov: Vec<i64> = Vec::new();
        let (mut kinds, mut args): (Vec<u8>, Vec<u64>) = (Vec::new(), Vec::new());
        let mut list = Vec::new();
        for r in &runs {
            rows.extend(&r.rec.rows);
            for m in &r.rec.moves {
                mov.extend_from_slice(m);
            }
            kinds.extend(r.moves.iter().map(|&(k, _)| k));
            args.extend(r.moves.iter().map(|&(_, a)| a));
            for fl in &r.rec.fork_lab {
                lab.extend_from_slice(fl);
            }
            for at in &r.rec.at {
                lab.extend_from_slice(&[
                    r.run as f32,
                    at[0],
                    at[1],
                    at[2],
                    at[3],
                    f32::from(u8::from(r.won)),
                    r.end_stage as f32,
                    r.end_ante as f32,
                    r.endless,
                ]);
            }
            list.push(format!(
                "{{\"run\": {}, \"seed\": {}, \"won\": {}, \"end_stage\": {}, \"rows\": {}, \"moves\": {}}}",
                r.run,
                esc(&r.seed),
                r.won,
                r.end_stage,
                r.rec.rows.len(),
                r.moves.len()
            ));
        }
        let n = rows.len();
        let labels = if self.forks {
            FORK_LABELS.len()
        } else {
            LABELS.len()
        };
        assert_eq!(lab.len(), n * labels, "labels for every row");
        let name = |part: &str| self.dir.join(format!("chunk-{chunk:05}-{part}"));
        let off: Vec<f32> = runs
            .iter()
            .flat_map(|r| r.rec.off.iter().copied())
            .collect();
        let off_file = || {
            assert_eq!(off.len(), n, "an offset for every recorded position");
            (
                "off.npy",
                npy_bytes("<f4", &[n], &le_bytes(&off, f32::to_le_bytes)),
            )
        };
        if self.only_offsets {
            // Beside the chunk already there: it must hold as many positions (the runs played again alike).
            let held = npy_rows(&name("lab.npy"));
            assert_eq!(
                held,
                n,
                "{}: {held} positions recorded, {n} played again",
                name("lab.npy").display()
            );
            let (part, bytes) = off_file();
            let path = name(part);
            let tmp = path.with_extension("tmp");
            std::fs::write(&tmp, bytes).expect("write offsets");
            std::fs::rename(&tmp, &path).expect("write offsets");
            self.chunks += 1;
            self.rows += n as u64;
            self.runs += runs.len() as u64;
            return;
        }
        let files = [
            (
                "idx.npy",
                npy_bytes(
                    "<u2",
                    &[rows.idx.len()],
                    &le_bytes(&rows.idx, u16::to_le_bytes),
                ),
            ),
            (
                "val.npy",
                npy_bytes(
                    "<f2",
                    &[rows.val.len()],
                    &le_bytes(&rows.val, u16::to_le_bytes),
                ),
            ),
            (
                "ptr.npy",
                npy_bytes(
                    "<i8",
                    &[rows.ptr.len()],
                    &le_bytes(&rows.ptr, i64::to_le_bytes),
                ),
            ),
            (
                "lab.npy",
                npy_bytes("<f4", &[n, labels], &le_bytes(&lab, f32::to_le_bytes)),
            ),
            (
                "runs.json",
                format!("[\n{}\n]\n", list.join(",\n")).into_bytes(),
            ),
        ];
        let mut files = Vec::from(files);
        if OFFSETS.load(Ordering::Relaxed) {
            files.push(off_file());
        }
        if self.forks {
            // Forked options' positions: no moves to keep.
        } else if RECORD_MODE.load(Ordering::Relaxed) == 2 {
            assert_eq!(mov.len(), 3 * n, "a move for every recorded position");
            files.push((
                "mov.npy",
                npy_bytes("<i8", &[n, 3], &le_bytes(&mov, i64::to_le_bytes)),
            ));
        } else {
            // Every move of every run, in run order (`runs.json` gives each run's count): the runs can be
            // played again move by move, so their positions can be encoded afresh (`--replay`).
            files.push(("mkind.npy", npy_bytes("|u1", &[kinds.len()], &kinds)));
            files.push((
                "marg.npy",
                npy_bytes("<u8", &[args.len()], &le_bytes(&args, u64::to_le_bytes)),
            ));
        }
        for (part, bytes) in files {
            let path = name(part);
            let tmp = path.with_extension("tmp");
            std::fs::write(&tmp, bytes).expect("write recorded positions");
            std::fs::rename(&tmp, &path).expect("write recorded positions");
        }
        self.chunks += 1;
        self.rows += n as u64;
        self.runs += runs.len() as u64;
        if self.forks {
            // A fork recording can be read while it is still being made (a night of forks, trained on at any hour):
            // after each chunk, a manifest that says how many chunks are whole so far.
            self.fork_manifest(" \"partial\": true", true);
        }
    }

    /// A fork recording's manifest: the rows' layout and labels, how the forks were made, and `about` (the teacher,
    /// or that the recording is still being made).
    fn fork_manifest(&self, about: &str, partial: bool) {
        let labels: Vec<String> = FORK_LABELS.iter().map(|l| esc(l)).collect();
        let layouts: Vec<String> = VALUE_LAYOUTS
            .iter()
            .map(|&(v, n)| format!("[{v}, {n}]"))
            .collect();
        let text = format!(
            "{{\n \"format\": 1,\n \"kind\": \"forks\",\n \"input\": \"value\",\n \"input_version\": \
             {VALUE_INPUT_VERSION},\n \"inputs\": {VALUE_LEN},\n \"layouts\": [{}],\n \"runs\": {},\n \
             \"rows\": {},\n \"chunks\": {},\n \"runs_per_chunk\": {RUNS_PER_CHUNK},\n \"labels\": [{}],\n \
             \"fork_chance\": {},\n \"fork_antes\": [{}, {}],\n \"fork_best\": {},\n \"fork_copies\": {},\n \
             \"fork_salt\": {},\n \"stake\": {stake},\n{about}\n}}\n",
            layouts.join(", "),
            self.runs,
            self.rows,
            self.chunks,
            labels.join(", "),
            num(FORK_CHANCE_PPM.load(Ordering::Relaxed) as f64 / 1e6),
            FORK_ANTE_LO.load(Ordering::Relaxed),
            FORK_ANTE_HI.load(Ordering::Relaxed),
            FORK_BEST.load(Ordering::Relaxed),
            FORK_COPIES.load(Ordering::Relaxed),
            FORK_SALT.load(Ordering::Relaxed),
            stake = STAKE.load(Ordering::Relaxed),
        );
        debug_assert!(partial == about.contains("\"partial\": true"));
        write_atomic(&self.dir.join("manifest.json"), &text);
    }

    /// `offsets`: the value networks the offsets are the log-odds of (`--offsets`), if any.
    fn manifest(&self, about: &str, offsets: Option<&str>) {
        if self.only_offsets {
            let text = format!(
                "{{\n \"offset_net\": {},\n \"offset_net_hash\": {},\n \"runs\": {},\n \"rows\": {},\n \
                 \"chunks\": {}\n}}\n",
                offsets.map_or("null".into(), esc),
                offsets.map_or("null".into(), |o| esc(&nets_hash(o))),
                self.runs,
                self.rows,
                self.chunks
            );
            write_atomic(&self.dir.join("offsets.json"), &text);
            return;
        }
        if self.forks {
            self.fork_manifest(about, false);
            return;
        }
        let labels: Vec<String> = LABELS.iter().map(|l| esc(l)).collect();
        let (input, inputs) = if RECORD_MODE.load(Ordering::Relaxed) == 2 {
            ("screen", OBS_LEN)
        } else {
            ("value", VALUE_LEN)
        };
        let moves_kept = RECORD_MODE.load(Ordering::Relaxed) == 1;
        // The input layouts the rows hold (`VALUE_LAYOUTS`: each one's floats start the next one's), so the trainer
        // can read them as an older layout too. The screen's encoding is the same in every layout.
        let layouts: Vec<String> = VALUE_LAYOUTS
            .iter()
            .map(|&(v, n)| format!("[{v}, {}]", if input == "screen" { OBS_LEN } else { n }))
            .collect();
        let text = format!(
            "{{\n \"format\": 1,\n \"input\": \"{input}\",\n \"input_version\": {VALUE_INPUT_VERSION},\n \
             \"inputs\": {inputs},\n \"layouts\": [{}],\n \"runs\": {},\n \"rows\": {},\n \"chunks\": {},\n \
             \"runs_per_chunk\": {RUNS_PER_CHUNK},\n \"labels\": [{}],\n \"moves_kept\": {moves_kept},\n \
             \"endless\": {endless},\n \"endless_last_ante\": {ENDLESS_LAST_ANTE},\n \
             \"stake\": {stake},\n \"offset_net\": {offset},\n \"offset_net_hash\": {hash},\n{about}\n}}\n",
            layouts.join(", "),
            self.runs,
            self.rows,
            self.chunks,
            labels.join(", "),
            stake = STAKE.load(Ordering::Relaxed),
            endless = ENDLESS.load(Ordering::Relaxed),
            offset = offsets.map_or("null".into(), esc),
            hash = offsets.map_or("null".into(), |o| esc(&nets_hash(o))),
        );
        write_atomic(&self.dir.join("manifest.json"), &text);
    }
}

/// A recorded run to play again move by move (`--replay DIR`), and how it ended when it was recorded.
struct Replayed {
    seed: String,
    moves: Vec<(u8, u64)>,
    won: bool,
    end_stage: u32,
    /// Positions recorded.
    rows: usize,
}

/// The data of a `.npy` file this program wrote (version 1.0, its own dtype and shape).
fn npy_data(path: &Path) -> Vec<u8> {
    let b = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert!(
        b.len() >= 10 && b.starts_with(b"\x93NUMPY\x01\x00"),
        "{}: not a .npy file this program wrote",
        path.display()
    );
    let header = u16::from_le_bytes([b[8], b[9]]) as usize;
    b[10 + header..].to_vec()
}

/// The first dimension of a `.npy` file this program wrote (its rows), read from the header alone.
fn npy_rows(path: &Path) -> usize {
    use std::io::Read;
    let mut f = std::fs::File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut head = [0u8; 10];
    f.read_exact(&mut head)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert!(
        head.starts_with(b"\x93NUMPY\x01\x00"),
        "{}: not a .npy file this program wrote",
        path.display()
    );
    let mut header = vec![0u8; u16::from_le_bytes([head[8], head[9]]) as usize];
    f.read_exact(&mut header)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let text = String::from_utf8_lossy(&header);
    let shape = text
        .split("'shape': (")
        .nth(1)
        .unwrap_or_else(|| panic!("{}: no shape in its header", path.display()));
    shape[..shape.find([',', ')']).expect("a shape")]
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("{}: its shape isn't a number of rows", path.display()))
}

/// Whether a recording holds every position's offsets by these networks already (`--offsets-into` wrote them:
/// `offsets.json` names their fingerprint and as many positions as the manifest).
fn offsets_there(dir: &Path, nets: &str) -> bool {
    let (Ok(about), Ok(manifest)) = (
        std::fs::read_to_string(dir.join("offsets.json")),
        std::fs::read_to_string(dir.join("manifest.json")),
    ) else {
        return false;
    };
    let line = |text: &str, key: &str| {
        text.lines()
            .find(|l| l.trim_start().starts_with(&format!("\"{key}\": ")))
            .map(|l| field(l, key).trim_matches('"').to_string())
    };
    let chunks = std::fs::read_dir(dir).map_or(0, |rd| {
        rd.flatten()
            .filter(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                n.starts_with("chunk-") && n.ends_with("-off.npy")
            })
            .count()
    });
    line(&about, "offset_net_hash") == Some(nets_hash(nets))
        && line(&about, "rows").is_some()
        && line(&about, "rows") == line(&manifest, "rows")
        && line(&manifest, "chunks").and_then(|c| c.parse::<usize>().ok()) == Some(chunks)
}

/// A short fingerprint of a comma-separated list of network files (FNV-1a over their bytes, in order), so a
/// recording's offsets can be matched with the networks they came from.
fn nets_hash(paths: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in paths.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let bytes = std::fs::read(p).unwrap_or_else(|e| panic!("{p}: {e}"));
        for b in bytes {
            h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
        h = (h ^ 0xff).wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// A field's text in one line of `runs.json` (written by `Recorder::write`: one run to a line).
fn field<'a>(line: &'a str, key: &str) -> &'a str {
    let tag = format!("\"{key}\": ");
    let at = line
        .find(&tag)
        .unwrap_or_else(|| panic!("no {key:?} in runs.json line {line:?}"))
        + tag.len();
    let rest = &line[at..];
    rest[..rest.find([',', '}']).unwrap_or(rest.len())].trim()
}

/// The runs of a recording made with `--record`, in run order, and the lines of its manifest that say how it
/// was played (bot, seed base, networks, options), for the new recording's manifest.
fn load_replay(dir: &Path) -> (Vec<Replayed>, String, u8) {
    let manifest = std::fs::read_to_string(dir.join("manifest.json")).unwrap_or_else(|e| {
        panic!(
            "{}: no manifest.json ({e}): the recording is missing or didn't finish",
            dir.display()
        )
    });
    assert!(
        manifest.contains("\"moves_kept\": true"),
        "{}: recorded before recordings kept every move, so its runs can't be played again",
        dir.display()
    );
    let about: Vec<&str> = manifest
        .lines()
        .filter(|l| {
            [
                "bot",
                "seed_base",
                "value_net",
                "value_weight",
                "teacher_options",
                "stake_net",
                "stake_mode",
                "wins",
            ]
            .iter()
            .any(|k| l.starts_with(&format!(" \"{k}\": ")))
        })
        .map(|l| l.trim_end_matches(','))
        .collect();
    let mut chunks: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("chunk-") && n.ends_with("-runs.json"))
        })
        .collect();
    chunks.sort();
    let mut runs = Vec::new();
    for c in chunks {
        let stem = c
            .to_string_lossy()
            .trim_end_matches("-runs.json")
            .to_string();
        let kinds = npy_data(Path::new(&format!("{stem}-mkind.npy")));
        let args: Vec<u64> = npy_data(Path::new(&format!("{stem}-marg.npy")))
            .chunks_exact(8)
            .map(|b| u64::from_le_bytes(b.try_into().expect("8 bytes")))
            .collect();
        assert_eq!(
            kinds.len(),
            args.len(),
            "{stem}: as many kinds as arguments"
        );
        let mut at = 0;
        for line in std::fs::read_to_string(&c)
            .expect("read runs.json")
            .lines()
            .filter(|l| l.trim_start().starts_with('{'))
        {
            let run: usize = field(line, "run").parse().expect("a run number");
            assert_eq!(run, runs.len(), "{stem}: runs in order, none missing");
            let n: usize = field(line, "moves").parse().expect("a move count");
            let moves = kinds[at..at + n]
                .iter()
                .copied()
                .zip(args[at..at + n].iter().copied())
                .collect();
            at += n;
            runs.push(Replayed {
                seed: field(line, "seed").trim_matches('"').to_string(),
                moves,
                won: field(line, "won") == "true",
                end_stage: field(line, "end_stage").parse().expect("an end stage"),
                rows: field(line, "rows").parse().expect("a position count"),
            });
        }
        assert_eq!(at, kinds.len(), "{stem}: every move belongs to a run");
    }
    let from = dir
        .file_name()
        .map_or(String::new(), |n| n.to_string_lossy().into_owned());
    let about = format!("{},\n \"replayed_from\": {}", about.join(",\n"), esc(&from));
    // The stake the runs were played at (recordings from before stakes were named are White Stake's).
    let stake = manifest
        .lines()
        .find(|l| l.starts_with(" \"stake\": "))
        .map_or(1, |l| field(l, "stake").parse().expect("a stake number"));
    if manifest
        .lines()
        .any(|l| l.trim().trim_end_matches(',') == "\"endless\": true")
    {
        ENDLESS.store(true, Ordering::Relaxed);
    }
    (runs, about, stake)
}

/// Everything counted so far.
struct Totals {
    runs: u64,
    wins: u64,
    moves: u64,
    refused: u64,
    ante: [u64; 10],
    lost_to: HashMap<&'static str, u64>,
    bought: Vec<u64>,
    held: Vec<u64>,
    held_won: Vec<u64>,
    hands: [u64; 12],
    blinds_beaten: u64,
    money_end: i64,
    rerolls: u64,
    skips: u64,
    packs: u64,
    vouchers: u64,
    jokers_bought: u64,
    /// `--endless`: wins that played on, the blinds they beat after winning, and how many beat each number.
    endless_wins: u64,
    endless_blinds: u64,
    endless_hist: [u64; ENDLESS_DEPTH as usize + 1],
    /// `--forks`: decisions forked, those where the teacher's own choice went best (and where the run's move did),
    /// what its choices gave away in win rate, and the play-outs made.
    forks: u64,
    fork_agree: u64,
    fork_made_agree: u64,
    fork_regret: f64,
    fork_playouts: u64,
    /// `--diagnose`: lost blinds played again with deeper search, and how many were beaten.
    deep: u64,
    deep_won: u64,
}

impl Totals {
    fn new() -> Totals {
        Totals {
            runs: 0,
            wins: 0,
            moves: 0,
            refused: 0,
            ante: [0; 10],
            lost_to: HashMap::new(),
            bought: vec![0; N_JOKERS],
            held: vec![0; N_JOKERS],
            held_won: vec![0; N_JOKERS],
            hands: [0; 12],
            blinds_beaten: 0,
            money_end: 0,
            rerolls: 0,
            skips: 0,
            packs: 0,
            vouchers: 0,
            jokers_bought: 0,
            endless_wins: 0,
            endless_blinds: 0,
            endless_hist: [0; ENDLESS_DEPTH as usize + 1],
            forks: 0,
            fork_agree: 0,
            fork_made_agree: 0,
            fork_regret: 0.0,
            fork_playouts: 0,
            deep: 0,
            deep_won: 0,
        }
    }

    fn add(&mut self, s: &RunSummary) {
        self.runs += 1;
        self.wins += s.won as u64;
        self.moves += s.moves;
        self.refused += s.refused as u64;
        self.ante[s.end] += 1;
        if !s.won {
            *self.lost_to.entry(s.lost_to).or_insert(0) += 1;
        }
        for k in &s.bought {
            self.bought[k.index()] += 1;
        }
        for k in &s.held {
            self.held[k.index()] += 1;
            if s.won {
                self.held_won[k.index()] += 1;
            }
        }
        for (t, n) in self.hands.iter_mut().zip(s.hands) {
            *t += n;
        }
        self.blinds_beaten += s.blinds_beaten;
        self.money_end += s.money_end;
        self.rerolls += s.rerolls;
        self.skips += s.skips;
        self.packs += s.packs;
        self.vouchers += s.vouchers;
        self.jokers_bought += s.bought.len() as u64;
        if let Some(d) = s.endless {
            self.endless_wins += 1;
            self.endless_blinds += u64::from(d);
            self.endless_hist[d.min(ENDLESS_DEPTH) as usize] += 1;
        }
        for (_, f) in &s.forks {
            self.forks += 1;
            let best = f.best();
            self.fork_agree += u64::from(f.chosen() == Some(best));
            self.fork_made_agree += u64::from(f.options.iter().position(|o| o.made) == Some(best));
            self.fork_regret += f.regret().unwrap_or(0.0);
            self.fork_playouts += (f.options.len() * f.copies) as u64;
        }
        if let Some(d) = &s.diagnosis {
            // The row ends ",deep_won,deep_best,deep_needed": deep_won is -1 when the blind wasn't played again.
            let mut tail = d.rsplit(',');
            let (_, _, won) = (tail.next(), tail.next(), tail.next());
            match won {
                Some("1") => {
                    self.deep += 1;
                    self.deep_won += 1;
                }
                Some("0") => self.deep += 1,
                _ => {}
            }
        }
    }

    fn merge(&mut self, o: &Totals) {
        self.runs += o.runs;
        self.wins += o.wins;
        self.moves += o.moves;
        self.refused += o.refused;
        for (a, b) in self.ante.iter_mut().zip(o.ante) {
            *a += b;
        }
        for (k, v) in &o.lost_to {
            *self.lost_to.entry(k).or_insert(0) += v;
        }
        for (a, b) in self.bought.iter_mut().zip(&o.bought) {
            *a += b;
        }
        for (a, b) in self.held.iter_mut().zip(&o.held) {
            *a += b;
        }
        for (a, b) in self.held_won.iter_mut().zip(&o.held_won) {
            *a += b;
        }
        for (a, b) in self.hands.iter_mut().zip(o.hands) {
            *a += b;
        }
        self.blinds_beaten += o.blinds_beaten;
        self.money_end += o.money_end;
        self.rerolls += o.rerolls;
        self.skips += o.skips;
        self.packs += o.packs;
        self.vouchers += o.vouchers;
        self.jokers_bought += o.jokers_bought;
        self.endless_wins += o.endless_wins;
        self.endless_blinds += o.endless_blinds;
        for (a, b) in self.endless_hist.iter_mut().zip(o.endless_hist) {
            *a += b;
        }
        self.forks += o.forks;
        self.fork_agree += o.fork_agree;
        self.fork_made_agree += o.fork_made_agree;
        self.fork_regret += o.fork_regret;
        self.fork_playouts += o.fork_playouts;
        self.deep += o.deep;
        self.deep_won += o.deep_won;
    }

    fn mean_ante(&self) -> f64 {
        if self.runs == 0 {
            return 0.0;
        }
        self.ante
            .iter()
            .enumerate()
            .map(|(a, n)| (a as f64) * (*n as f64))
            .sum::<f64>()
            / self.runs as f64
    }
}

/// Wilson 95% interval for k successes in n.
fn wilson(k: u64, n: u64) -> (f64, f64) {
    if n == 0 {
        return (0.0, 0.0);
    }
    let (n, p, z) = (n as f64, k as f64 / n as f64, 1.96f64);
    let d = 1.0 + z * z / n;
    let c = (p + z * z / (2.0 * n)) / d;
    let h = z * (p * (1.0 - p) / n + z * z / (4.0 * n * n)).sqrt() / d;
    ((c - h).max(0.0), (c + h).min(1.0))
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for ch in s.chars() {
        match ch {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn num(x: f64) -> String {
    if x.is_finite() {
        format!("{x:.6}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    } else {
        "null".into()
    }
}

/// The most common jokers in a count, as [name, count, count per run of `per`].
fn joker_table(counts: &[u64], per: u64, top: usize) -> String {
    let mut v: Vec<(usize, u64)> = counts
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, n)| *n > 0)
        .collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let rows: Vec<String> = v
        .into_iter()
        .take(top)
        .map(|(i, n)| {
            format!(
                "[{}, {n}, {}]",
                esc(JokerKind::ALL[i].name()),
                num(n as f64 / per.max(1) as f64)
            )
        })
        .collect();
    format!("[{}]", rows.join(", "))
}

struct Report<'a> {
    t: &'a Totals,
    elapsed: f64,
    rate: f64,
    target: u64,
    threads: usize,
    history: &'a [(f64, u64, f64, f64)],
    /// The best runs so far, as a JSON list.
    best: String,
    bot: &'a str,
    done: bool,
}

fn report_json(r: &Report) -> String {
    let t = r.t;
    let runs = t.runs.max(1) as f64;
    let (lo, hi) = wilson(t.wins, t.runs);
    let mut lost: Vec<(&&str, &u64)> = t.lost_to.iter().collect();
    lost.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let lost: Vec<String> = lost
        .iter()
        .take(12)
        .map(|(k, v)| format!("[{}, {v}]", esc(k)))
        .collect();
    let hands: Vec<String> = HandType::ALL
        .iter()
        .filter(|h| t.hands[**h as usize] > 0)
        .map(|h| format!("[{}, {}]", esc(h.name()), t.hands[*h as usize]))
        .collect();
    let hist: Vec<String> = r
        .history
        .iter()
        .map(|(s, n, a, w)| format!("[{}, {n}, {}, {}]", num(*s), num(*a), num(*w)))
        .collect();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64());
    let stake = STAKE.load(Ordering::Relaxed);
    let mut o = String::new();
    let _ = write!(
        o,
        "{{\n \"kind\": \"baseline\",\n \"bot\": {},\n \"deck\": \"Red Deck\",\n \"stake\": {},\n \"stake_number\": {},\n \
         \"done\": {},\n \"updated\": {},\n \"elapsed\": {},\n \"threads\": {},\n \"target\": {},\n \"runs\": {},\n \
         \"rate\": {},\n \"moves\": {},\n \"wins\": {},\n \"win_rate\": {},\n \"win_ci\": [{}, {}],\n \"mean_ante\": {},\n \
         \"ante_hist\": {:?},\n \"blinds_beaten_per_run\": {},\n \"refused_moves\": {},\n \"lost_to\": [{}],\n \
         \"hands_played\": [{}],\n \"jokers_bought\": {},\n \"jokers_held_at_end\": {},\n \"jokers_in_wins\": {},\n \
         \"per_run\": {{\"moves\": {}, \"jokers_bought\": {}, \"rerolls\": {}, \"skips\": {}, \"packs\": {}, \
         \"vouchers\": {}, \"money_at_end\": {}}},\n \"best_runs\": {},\n \"history\": [{}]\n}}\n",
        esc(r.bot),
        esc(STAKES[usize::from(stake) - 1]),
        stake,
        r.done,
        num(now),
        num(r.elapsed),
        r.threads,
        r.target,
        t.runs,
        num(r.rate),
        t.moves,
        t.wins,
        num(t.wins as f64 / runs),
        num(lo),
        num(hi),
        num(t.mean_ante()),
        &t.ante[1..],
        num(t.blinds_beaten as f64 / runs),
        t.refused,
        lost.join(", "),
        hands.join(", "),
        joker_table(&t.bought, t.runs, 20),
        joker_table(&t.held, t.runs, 20),
        joker_table(&t.held_won, t.wins, 20),
        num(t.moves as f64 / runs),
        num(t.jokers_bought as f64 / runs),
        num(t.rerolls as f64 / runs),
        num(t.skips as f64 / runs),
        num(t.packs as f64 / runs),
        num(t.vouchers as f64 / runs),
        num(t.money_end as f64 / runs),
        r.best,
        hist.join(", "),
    );
    if FORKS.load(Ordering::Relaxed) {
        // The judge test: how often the teacher's own choice was the option whose play-outs went best, and what
        // its choices gave away.
        let end = o.rfind("\n}").expect("the report ends with its brace");
        let n = t.forks.max(1) as f64;
        o.replace_range(
            end..,
            &format!(
                ",\n \"forks\": {{\"forked\": {}, \"playouts\": {}, \"copies\": {}, \"chosen_best\": {}, \
                 \"made_best\": {}, \"mean_regret\": {}}}\n}}\n",
                t.forks,
                t.fork_playouts,
                FORK_COPIES.load(Ordering::Relaxed),
                num(t.fork_agree as f64 / n),
                num(t.fork_made_agree as f64 / n),
                num(t.fork_regret / n)
            ),
        );
    }
    if DIAGNOSE.load(Ordering::Relaxed) {
        let end = o.rfind("\n}").expect("the report ends with its brace");
        o.replace_range(
            end..,
            &format!(
                ",\n \"deep\": {{\"lost_blinds_replayed\": {}, \"beaten\": {}, \"samples_x\": {}}}\n}}\n",
                t.deep,
                t.deep_won,
                num(DEEP_X.load(Ordering::Relaxed) as f64 / 100.0)
            ),
        );
    }
    if ENDLESS.load(Ordering::Relaxed) {
        // How strong the wins were: blinds beaten after the win, on average and by number.
        let end = o.rfind("\n}").expect("the report ends with its brace");
        let mean = if t.endless_wins > 0 {
            num(t.endless_blinds as f64 / t.endless_wins as f64)
        } else {
            "null".into()
        };
        o.replace_range(
            end..,
            &format!(
                ",\n \"endless\": true,\n \"endless_last_ante\": {ENDLESS_LAST_ANTE},\n \"endless_blinds_per_win\": {mean},\n \
                 \"endless_hist\": {:?}\n}}\n",
                t.endless_hist
            ),
        );
    }
    o
}

fn sample_json(s: &RunSummary, record: &[(u8, u64)], pick: &str, run: u64, bot: &str) -> String {
    let moves: Vec<String> = record.iter().map(|(k, a)| format!("[{k}, {a}]")).collect();
    format!(
        "{{\"seed\": {}, \"stake\": {}, \"bot\": {}, \"source\": \"baseline\", \"pick\": {}, \"run\": {run}, \
         \"label\": {}, \"result\": {}, \"won\": {}, \"ante\": {}, \"stage\": {}, \"closeness\": {}, \
         \"best_hand\": {}, \"moves\": [{}]}}\n",
        esc(&s.seed),
        STAKE.load(Ordering::Relaxed),
        esc(bot),
        esc(pick),
        esc(&if pick == "best" { format!("one of the {bot}'s best runs") } else { format!("a typical run by the {bot}") }),
        esc(&s.result()),
        s.won,
        s.end,
        s.stage,
        num(s.closeness),
        num(s.best_hand),
        moves.join(", ")
    )
}

/// The best runs so far, best first, and the sample files they were written to.
struct Best {
    runs: Vec<(u64, RunSummary)>,
    written: HashMap<u64, PathBuf>,
    gone: Vec<PathBuf>,
}

impl Best {
    fn new() -> Best {
        Best {
            runs: Vec::new(),
            written: HashMap::new(),
            gone: Vec::new(),
        }
    }

    /// Would this run make the list?
    fn wants(&self, s: &RunSummary) -> bool {
        self.runs.len() < KEEP_BEST || self.runs.last().is_some_and(|(_, w)| s.better_than(w))
    }

    fn add(&mut self, i: u64, s: RunSummary) {
        let at = self
            .runs
            .iter()
            .position(|(_, o)| s.better_than(o))
            .unwrap_or(self.runs.len());
        self.runs.insert(at, (i, s));
        while self.runs.len() > KEEP_BEST {
            let (gone, _) = self.runs.pop().expect("non-empty");
            if let Some(f) = self.written.remove(&gone) {
                self.gone.push(f);
            }
        }
    }

    fn rows(&self, top: usize) -> String {
        let rows: Vec<String> = self
            .runs
            .iter()
            .take(top)
            .enumerate()
            .map(|(k, (i, s))| {
                format!(
                    "[{}, {}, {}, {}, {}, {}, {}]",
                    k + 1,
                    esc(&s.seed),
                    esc(&s.result()),
                    num(s.best_hand),
                    s.won,
                    i,
                    esc(&format!("best-{i:010}.json"))
                )
            })
            .collect();
        format!("[{}]", rows.join(", "))
    }
}

fn write_atomic(path: &Path, text: &str) {
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, path); // the viewer is a nicety: never fail the job over it
    }
}

/// The sample files whose names start with `prefix`, oldest name first.
fn sample_files(dir: &Path, prefix: &str) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension().is_some_and(|x| x == "json")
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(prefix))
        })
        .collect();
    files.sort();
    files
}

fn prune_typical(dir: &Path) {
    let files = sample_files(dir, "typical-");
    let extra = files.len().saturating_sub(KEEP_TYPICAL);
    for f in files.into_iter().take(extra) {
        let _ = std::fs::remove_file(f);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let opt = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    // `--replay DIR`: the runs of a recording made with `--record`, each played again by its own moves instead of
    // by a bot, so `--record` writes their positions as this code encodes them (a new input, no new search).
    // `--offsets-into DIR`: DIR's runs played again by their moves, for their positions' offsets alone.
    let offsets_into = opt("--offsets-into");
    if offsets_into.is_some()
        && [opt("--replay"), opt("--record"), opt("--record-moves")]
            .iter()
            .any(Option::is_some)
    {
        panic!("--offsets-into DIR plays DIR's runs again and writes beside them: no --replay or --record with it");
    }
    if let (Some(d), Some(nets)) = (&offsets_into, opt("--value-net")) {
        if offsets_there(Path::new(d), &nets) {
            println!("{d} has these networks' offsets already (offsets.json): nothing to do.");
            return;
        }
    }
    OFFSETS.store(
        offsets_into.is_some() || args.iter().any(|a| a == "--offsets"),
        Ordering::Relaxed,
    );
    let replay = opt("--replay")
        .or_else(|| offsets_into.clone())
        .map(|d| load_replay(Path::new(&d)));
    // `--stake N` (1 White ... 8 Gold); a replay plays at its recording's.
    let stake: u8 = match &replay {
        Some((_, _, s)) => *s,
        None => opt("--stake").map_or(1, |s| {
            s.parse().expect("--stake takes 1 (White) to 8 (Gold)")
        }),
    };
    assert!(
        (1..=8).contains(&stake),
        "--stake takes 1 (White) to 8 (Gold), not {stake}"
    );
    STAKE.store(stake, Ordering::Relaxed);
    if args.iter().any(|a| a == "--endless") {
        assert!(
            replay.is_none(),
            "--endless: a replay plays on as its recording did"
        );
        ENDLESS.store(true, Ordering::Relaxed);
    }
    let runs_wanted: Option<u64> = opt("--runs").and_then(|s| s.replace('_', "").parse().ok());
    let target: u64 = match &replay {
        Some((runs, _, _)) => runs_wanted.map_or(runs.len() as u64, |n| n.min(runs.len() as u64)),
        None => runs_wanted.unwrap_or(1_000_000),
    };
    let seconds: Option<f64> = opt("--seconds").and_then(|s| s.parse().ok());
    let base: u64 = opt("--seed").and_then(|s| s.parse().ok()).unwrap_or(1);
    let threads: usize = match opt("--threads").and_then(|s| s.parse().ok()) {
        Some(t) if t > 0 => t,
        _ => std::thread::available_parallelism().map_or(1, |n| n.get()),
    };
    let out = opt("--out");
    // A table of every run (`--runs-csv`), and experimental teacher options (`--teacher-options a,b`) for
    // A/B tests on the same seeds.
    let runs_csv = opt("--runs-csv");
    DETAIL.store(runs_csv.is_some(), Ordering::Relaxed);
    // `--diagnose OUT.csv`: a row about every run, the lost blind played again with `--deep` times the samples for
    // a share `--deep-share` of the lost runs.
    let diagnose = opt("--diagnose");
    DIAGNOSE.store(diagnose.is_some(), Ordering::Relaxed);
    if let Some(x) = opt("--deep") {
        let x: f64 = x
            .parse()
            .expect("--deep takes a number (times the usual samples)");
        DEEP_X.store((x * 100.0).round() as u64, Ordering::Relaxed);
    }
    if let Some(x) = opt("--deep-share") {
        let x: f64 = x.parse().expect("--deep-share takes a share, 0 to 1");
        DEEP_SHARE_PPM.store((x.clamp(0.0, 1.0) * 1e6).round() as u64, Ordering::Relaxed);
    }
    // `--forks DIR`: decisions forked as the runs go (forks.rs), each option played out by the teacher.
    let forks_dir = opt("--forks");
    FORKS.store(forks_dir.is_some(), Ordering::Relaxed);
    if let Some(x) = opt("--fork-chance") {
        let x: f64 = x.parse().expect("--fork-chance takes a share, 0 to 1");
        FORK_CHANCE_PPM.store((x.clamp(0.0, 1.0) * 1e6).round() as u64, Ordering::Relaxed);
    }
    if let Some(a) = opt("--fork-antes") {
        let (lo, hi) = a.split_once('-').expect("--fork-antes takes LO-HI, as 2-6");
        FORK_ANTE_LO.store(lo.parse().expect("an ante"), Ordering::Relaxed);
        FORK_ANTE_HI.store(hi.parse().expect("an ante"), Ordering::Relaxed);
    }
    if let Some(n) = opt("--fork-best") {
        FORK_BEST.store(
            n.parse().expect("--fork-best takes a count"),
            Ordering::Relaxed,
        );
    }
    if let Some(n) = opt("--fork-copies") {
        let n: u64 = n.parse().expect("--fork-copies takes a count");
        assert!(n >= 1, "--fork-copies: at least one copy");
        FORK_COPIES.store(n, Ordering::Relaxed);
    }
    if let Some(n) = opt("--fork-salt") {
        let n: u64 = n.parse().expect("--fork-salt takes a number");
        assert!(n < 1 << 20, "--fork-salt: under a million");
        FORK_SALT.store(n, Ordering::Relaxed);
    }
    if let Some(o) = opt("--teacher-options") {
        let names: Vec<String> = o
            .split(',')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
        if let Err(e) = bm_core::teacher::set_options(&names) {
            panic!("{e}");
        }
        println!("Teacher options: {}", names.join(", "));
    }
    // `--fork-play-options a,b`: options the forks' play-outs add to those (lighter play-outs, say).
    if let Some(o) = opt("--fork-play-options") {
        let names: Vec<String> = o
            .split(',')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
        if let Err(e) = bm_core::forks::set_play_options(&names) {
            panic!("{e}");
        }
        println!("Fork play-outs add: {}", names.join(", "));
    }
    let details: Mutex<Vec<String>> = Mutex::new(Vec::new());
    // `--replay`: runs that went another way than recorded.
    let diverged: Mutex<Vec<String>> = Mutex::new(Vec::new());
    // The learned value (`--value-net`), and how much it counts (`--value-weight`).
    let value_net = opt("--value-net");
    if let Some(p) = &value_net {
        if let Err(e) = learned::load(p) {
            panic!("{e}");
        }
        println!("Value network: {p}");
    }
    if let Some(w) = opt("--value-weight") {
        let w: f64 = w.parse().expect("--value-weight takes a number");
        learned::set_weight(w);
        println!("Value network weight: {w}");
    }
    // `--stake-net NETS`: the value networks' chance corrected by the stake (`--stake-mode residual|delta`).
    let stake_net = opt("--stake-net");
    let stake_mode =
        learned::StakeMode::parse(&opt("--stake-mode").unwrap_or_else(|| "residual".into()))
            .unwrap_or_else(|e| panic!("{e}"));
    if let Some(p) = &stake_net {
        assert!(
            value_net.is_some(),
            "--stake-net corrects the chance of --value-net's networks: give both"
        );
        let reach = bm_core::teacher::options().iter().any(|o| {
            o.strip_prefix("reach=")
                .and_then(|k| k.parse::<f64>().ok())
                .is_some_and(|k| k >= 1.0)
        });
        assert!(
            !(reach && stake_mode == learned::StakeMode::Residual),
            "a residual stake correction corrects the win chance; teacher option reach rates by another"
        );
        if let Err(e) = learned::load_stake(p, stake_mode) {
            panic!("{e}");
        }
        println!("Stake correction ({}): {p}", stake_mode.name());
    }
    assert!(
        !OFFSETS.load(Ordering::Relaxed) || value_net.is_some(),
        "offsets are the value networks' log-odds: give --value-net"
    );
    let bot_label = opt("--bot").unwrap_or_else(|| "greedy".into());
    let (bot, bot_name): (Bot, &str) = match (bot_label.as_str(), value_net.is_some()) {
        ("greedy", _) => (greedy_move, "greedy bot"),
        ("teacher", false) => (teacher_move, "teacher bot"),
        ("teacher", true) => (teacher_move, "teacher bot with a learned value"),
        (other, _) => panic!("unknown bot {other:?}: greedy or teacher"),
    };
    assert!(
        !(FORKS.load(Ordering::Relaxed) || diagnose.is_some()) || bot_label == "teacher",
        "--forks and --diagnose play options out with the teacher: give --bot teacher"
    );
    let bot_name = if replay.is_some() {
        "replayed runs"
    } else {
        bot_name
    };
    // How each run ended, in seed order (for paired comparisons), when there aren't too many runs.
    let stages: Option<Mutex<Vec<u8>>> =
        (target <= 50_000).then(|| Mutex::new(vec![b'?'; target as usize]));
    // `--endless`: how far each win got after it, in seed order (A for no blind further ... M for all twelve; a
    // dash for a loss), for paired comparisons of how strong the wins are.
    let endless_marks: Option<Mutex<Vec<u8>>> = (target <= 50_000
        && ENDLESS.load(Ordering::Relaxed))
    .then(|| Mutex::new(vec![b'?'; target as usize]));
    let (record_dir, mode) = match (opt("--record"), opt("--record-moves"), &offsets_into) {
        (Some(d), None, None) => (Some(d), 1),
        (None, Some(d), None) => (Some(d), 2),
        (None, None, Some(d)) => (Some(d.clone()), 1),
        (None, None, None) => (None, 0),
        _ => panic!("--record or --record-moves, not both"),
    };
    assert!(
        !OFFSETS.load(Ordering::Relaxed) || mode == 1,
        "--offsets keeps the value networks' log-odds of the recorded positions: give --record DIR"
    );
    RECORD_MODE.store(mode, Ordering::Relaxed);
    RECORD.store(record_dir.is_some(), Ordering::Relaxed);
    let recorder = record_dir.map(|d| {
        let d = PathBuf::from(d);
        Mutex::new(if offsets_into.is_some() {
            Recorder::offsets_into(d, target)
        } else {
            Recorder::new(d, target)
        })
    });
    let fork_recorder = forks_dir.map(|d| {
        let mut r = Recorder::new(PathBuf::from(d), target);
        r.forks = true;
        Mutex::new(r)
    });
    let diagnoses: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let watch = std::env::var("BM_WATCH_DIR")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    if let Some(w) = &watch {
        let dir = w.join("samples");
        let _ = std::fs::create_dir_all(&dir);
        for f in sample_files(&dir, "best-")
            .into_iter()
            .chain(sample_files(&dir, "typical-"))
        {
            let _ = std::fs::remove_file(f); // a rerun of the job replaces the last run's samples
        }
    }

    let next = AtomicU64::new(0);
    let finished = AtomicBool::new(false);
    let totals = Mutex::new(Totals::new());
    let offer: Mutex<Option<(u64, RunSummary)>> = Mutex::new(None);
    let best = Mutex::new(Best::new());
    let start = Instant::now();
    let deadline = seconds.map(Duration::from_secs_f64);
    println!(
        "Baseline: the {bot_name} on {threads} threads, {target} runs (Red Deck, {}).",
        STAKES[usize::from(stake) - 1]
    );

    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut local = Totals::new();
                    let mut merged = Instant::now();
                    loop {
                        let over = deadline.is_some_and(|d| start.elapsed() >= d);
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= target || over {
                            break;
                        }
                        let mut other_way = false;
                        let mut s = match &replay {
                            None => play(run_seed(base, i), i, true, bot, None),
                            Some((runs, _, _)) => {
                                let r = &runs[i as usize];
                                let s = play(r.seed.clone(), i, true, bot, Some(&r.moves));
                                if s.refused
                                    || s.unfinished
                                    || s.moves != r.moves.len() as u64
                                    || s.won != r.won
                                    || s.stage != r.end_stage
                                    || s.recorded.as_ref().is_some_and(|rec| rec.at.len() != r.rows)
                                {
                                    other_way = true;
                                    diverged.lock().unwrap().push(format!(
                                        "run {i} (seed {}): recorded {} moves, {}; replayed {} moves, {}{}",
                                        r.seed,
                                        r.moves.len(),
                                        if r.won { "won".into() } else { format!("ended at stage {}", r.end_stage) },
                                        s.moves,
                                        if s.won { "won".into() } else { format!("ended at stage {}", s.stage) },
                                        if s.refused { " (a move refused)" } else if s.unfinished { " (not over)" } else { "" }
                                    ));
                                }
                                s
                            }
                        };
                        if let Some(d) = s.detail.take() {
                            details.lock().unwrap().push(d);
                        }
                        // A run that went another way on today's engine is left out of a diagnosis: its blinds
                        // are not the ones the recording played.
                        if let (Some(d), false) = (s.diagnosis.clone(), other_way) {
                            diagnoses.lock().unwrap().push(d);
                        }
                        if let Some(r) = &fork_recorder {
                            r.lock().unwrap().add(RecordedRun {
                                run: i,
                                seed: s.seed.clone(),
                                won: s.won,
                                end_stage: s.stage,
                                end_ante: s.end,
                                endless: -1.0,
                                rec: fork_rows(i, &s),
                                moves: Vec::new(),
                            });
                        }
                        if let Some(st) = &stages {
                            st.lock().unwrap()[i as usize] = b'A' + s.stage.min(24) as u8;
                        }
                        if let Some(em) = &endless_marks {
                            em.lock().unwrap()[i as usize] = s.endless.map_or(b'-', |d| b'A' + d as u8);
                        }
                        if let (Some(rec), Some(r)) = (s.recorded.take(), &recorder) {
                            r.lock().unwrap().add(RecordedRun {
                                run: i,
                                seed: s.seed.clone(),
                                won: s.won,
                                end_stage: s.stage,
                                end_ante: s.end,
                                endless: s.endless.map_or(-1.0, |d| d as f32),
                                rec,
                                moves: s.record.clone().unwrap_or_default(),
                            });
                        }
                        local.add(&s);
                        let mut b = best.lock().unwrap();
                        if b.wants(&s) {
                            b.add(i, s);
                        } else if i.is_multiple_of(17) {
                            drop(b);
                            *offer.lock().unwrap() = Some((i, s));
                        }
                        // Into the shared totals every 64 runs, or every half second for a slow bot.
                        if local.runs >= 64 || merged.elapsed() >= Duration::from_millis(500) {
                            totals.lock().unwrap().merge(&local);
                            local = Totals::new();
                            merged = Instant::now();
                        }
                    }
                    totals.lock().unwrap().merge(&local);
                })
            })
            .collect();

        // The reporter: progress about once a second, a sample every few seconds, a line in the log every ten.
        let reporter = scope.spawn(|| {
            let mut history: Vec<(f64, u64, f64, f64)> = Vec::new();
            let mut last_sample = Instant::now() - SAMPLE_EVERY;
            let mut last_line = Instant::now();
            let mut window: Vec<(f64, u64)> = Vec::new(); // (time, runs) for the recent rate
            let mut prev_hist = (0u64, 0.0f64); // runs and ante sum at the last history point
            loop {
                let done = finished.load(Ordering::Relaxed);
                let el = start.elapsed().as_secs_f64();
                let snapshot = {
                    let t = totals.lock().unwrap();
                    let mut c = Totals::new();
                    c.merge(&t);
                    c
                };
                window.push((el, snapshot.runs));
                window.retain(|(t, _)| el - t <= 5.0);
                let rate = match (window.first(), window.last()) {
                    (Some(a), Some(b)) if b.0 > a.0 => (b.1 - a.1) as f64 / (b.0 - a.0),
                    _ => 0.0,
                };
                if history.last().is_none_or(|h| el - h.0 >= 5.0) || done {
                    let ante_sum = snapshot.mean_ante() * snapshot.runs as f64;
                    let n = snapshot.runs - prev_hist.0;
                    let recent = if n > 0 {
                        (ante_sum - prev_hist.1) / n as f64
                    } else {
                        snapshot.mean_ante()
                    };
                    prev_hist = (snapshot.runs, ante_sum);
                    history.push((
                        el,
                        snapshot.runs,
                        recent,
                        snapshot.wins as f64 / snapshot.runs.max(1) as f64,
                    ));
                }
                if let Some(w) = &watch {
                    let dir = w.join("samples");
                    // The best runs: write the newcomers, remove the ones pushed off the list.
                    let rows = {
                        let mut b = best.lock().unwrap();
                        let Best {
                            runs,
                            written,
                            gone,
                        } = &mut *b;
                        for (i, s) in runs.iter() {
                            if !written.contains_key(i) {
                                if let Some(rec) = &s.record {
                                    let f = dir.join(format!("best-{i:010}.json"));
                                    write_atomic(&f, &sample_json(s, rec, "best", *i, bot_name));
                                    written.insert(*i, f);
                                }
                            }
                        }
                        for f in gone.drain(..) {
                            let _ = std::fs::remove_file(f);
                        }
                        b.rows(10)
                    };
                    let r = Report {
                        t: &snapshot,
                        elapsed: el,
                        rate: if done {
                            snapshot.runs as f64 / el.max(1e-9)
                        } else {
                            rate
                        },
                        target,
                        threads,
                        history: &history,
                        best: rows,
                        bot: bot_name,
                        done,
                    };
                    write_atomic(&w.join("progress.json"), &report_json(&r));
                    if last_sample.elapsed() >= SAMPLE_EVERY {
                        if let Some((i, s)) = offer.lock().unwrap().take() {
                            if let Some(rec) = &s.record {
                                let ms = SystemTime::now()
                                    .duration_since(UNIX_EPOCH)
                                    .map_or(0, |d| d.as_millis());
                                write_atomic(
                                    &dir.join(format!("typical-{ms:015}.json")),
                                    &sample_json(&s, rec, "typical", i, bot_name),
                                );
                                prune_typical(&dir);
                                last_sample = Instant::now();
                            }
                        }
                    }
                }
                if last_line.elapsed() >= Duration::from_secs(10) || done {
                    last_line = Instant::now();
                    if FORKS.load(Ordering::Relaxed) {
                        // Forking: the forks made and their pace, and the time cap when there is one.
                        let cap = deadline.map_or(String::new(), |d| {
                            format!(
                                ", {:.0} of {:.0} minutes",
                                el / 60.0,
                                d.as_secs_f64() / 60.0
                            )
                        });
                        println!(
                            "{} runs, {} forks ({} play-outs, {:.1} s a fork){cap}",
                            snapshot.runs,
                            snapshot.forks,
                            snapshot.fork_playouts,
                            el / snapshot.forks.max(1) as f64
                        );
                    }
                    println!(
                        "{} runs ({:.0} a second): mean final ante {:.2}, {} wins ({:.3}%)",
                        snapshot.runs,
                        rate,
                        snapshot.mean_ante(),
                        snapshot.wins,
                        100.0 * snapshot.wins as f64 / snapshot.runs.max(1) as f64
                    );
                }
                if done {
                    break;
                }
                std::thread::sleep(Duration::from_millis(1000));
            }
        });

        // Every run played: the reporter writes its last report and stops.
        for w in workers {
            w.join().expect("a worker thread failed");
        }
        finished.store(true, Ordering::Relaxed);
        reporter.join().expect("the reporter thread failed");
    });

    let t = totals.into_inner().unwrap();
    let el = start.elapsed().as_secs_f64();
    // A replayed run that went another way means the engine no longer plays what was recorded: its positions and
    // labels would be wrong, so the new recording is left without a manifest (the trainer refuses it). A diagnosis
    // alone (nothing recorded) leaves those runs out and carries on, unless more than one in a hundred went another
    // way (then the engine has changed too much for the recording to stand for it).
    let diverged = diverged.into_inner().unwrap();
    if !diverged.is_empty() {
        eprintln!(
            "{} of {} runs replayed differently from how they were recorded (the engine's rules changed since?):",
            diverged.len(),
            t.runs
        );
        for d in diverged.iter().take(10) {
            eprintln!("  {d}");
        }
        let recording = recorder.is_some() || fork_recorder.is_some();
        if recording || diverged.len() as u64 * 100 > t.runs.max(1) {
            std::process::exit(1);
        }
        eprintln!("Those runs are left out of the diagnosis; the rest go on.");
    }
    if let Some(r) = recorder {
        let mut r = r.into_inner().unwrap();
        r.finish();
        let opts = bm_core::teacher::options();
        let about = match &replay {
            // A replay keeps how the runs were played; only the encoding is this code's.
            Some((_, about, _)) => about.clone(),
            None => format!(
                " \"bot\": {},\n \"seed_base\": {base},\n \"value_net\": {},\n \"value_weight\": {},\n \
                 \"teacher_options\": [{}],\n \"stake_net\": {},\n \"stake_mode\": {},\n \"wins\": {},\n \
                 \"seconds\": {}",
                esc(bot_name),
                value_net.as_deref().map_or("null".into(), esc),
                num(learned::weight()),
                opts.iter().map(|o| esc(o)).collect::<Vec<_>>().join(", "),
                stake_net.as_deref().map_or("null".into(), esc),
                learned::stake_mode().map_or("null".into(), |m| esc(m.name())),
                t.wins,
                num(el)
            ),
        };
        r.manifest(
            &about,
            value_net
                .as_deref()
                .filter(|_| OFFSETS.load(Ordering::Relaxed)),
        );
        println!(
            "Recorded {} positions from {} runs in {} chunks: {}",
            r.rows,
            r.runs,
            r.chunks,
            r.dir.display()
        );
    }
    if let Some(r) = fork_recorder {
        let mut r = r.into_inner().unwrap();
        r.finish();
        let opts = bm_core::teacher::options();
        let about = format!(
            " \"bot\": {},\n \"seed_base\": {base},\n \"value_net\": {},\n \"value_weight\": {},\n \
             \"teacher_options\": [{}],\n \"replayed_from\": {},\n \"forks\": {},\n \"chosen_best\": {},\n \
             \"made_best\": {},\n \"mean_regret\": {},\n \"seconds\": {}",
            esc(bot_name),
            value_net.as_deref().map_or("null".into(), esc),
            num(learned::weight()),
            opts.iter().map(|o| esc(o)).collect::<Vec<_>>().join(", "),
            opt("--replay").map_or("null".into(), |d| esc(&d)),
            t.forks,
            num(t.fork_agree as f64 / t.forks.max(1) as f64),
            num(t.fork_made_agree as f64 / t.forks.max(1) as f64),
            num(t.fork_regret / t.forks.max(1) as f64),
            num(el)
        );
        r.manifest(&about, None);
        println!(
            "Forked {} decisions ({} play-outs) from {} runs: {} rows in {} chunks: {}. The teacher's own choice went \
             best at {:.1}%, giving away {:.3} in win rate on average.",
            t.forks,
            t.fork_playouts,
            r.runs,
            r.rows,
            r.chunks,
            r.dir.display(),
            100.0 * t.fork_agree as f64 / t.forks.max(1) as f64,
            t.fork_regret / t.forks.max(1) as f64
        );
    }
    let history = vec![(
        el,
        t.runs,
        t.mean_ante(),
        t.wins as f64 / t.runs.max(1) as f64,
    )];
    let r = Report {
        t: &t,
        elapsed: el,
        rate: t.runs as f64 / el.max(1e-9),
        target,
        threads,
        history: &history,
        best: best.lock().unwrap().rows(KEEP_BEST),
        bot: bot_name,
        done: true,
    };
    let mut text = report_json(&r);
    // What the runs were played with, so two reports can be checked to pair seed by seed
    // (scripts/compare_runs.py), and each run's ending in seed order.
    let opts = bm_core::teacher::options();
    let mut tail = format!(
        ",\n \"seed_base\": {base},\n \"value_net\": {},\n \"value_weight\": {},\n \"teacher_options\": [{}],\n \
         \"stake_net\": {},\n \"stake_mode\": {}",
        value_net.as_deref().map_or("null".into(), esc),
        if value_net.is_some() {
            num(learned::weight())
        } else {
            "null".into()
        },
        opts.iter().map(|o| esc(o)).collect::<Vec<_>>().join(", "),
        stake_net.as_deref().map_or("null".into(), esc),
        learned::stake_mode().map_or("null".into(), |m| esc(m.name())),
    );
    if let Some(st) = stages {
        let st = String::from_utf8(st.into_inner().unwrap()).expect("letters");
        tail += &format!(",\n \"run_stages\": {}", esc(&st));
    }
    if let Some(em) = endless_marks {
        let em = String::from_utf8(em.into_inner().unwrap()).expect("letters");
        tail += &format!(",\n \"run_endless\": {}", esc(&em));
    }
    let end = text.rfind("\n}").expect("the report ends with its brace");
    text.replace_range(end.., &format!("{tail}\n}}\n"));
    if let Some(p) = out {
        let p = Path::new(&p);
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).expect("make the results folder");
        }
        std::fs::write(p, &text).expect("write the results");
        println!("Results: {}", p.display());
    }
    if let Some(p) = runs_csv {
        let mut rows = details.into_inner().unwrap();
        rows.sort();
        let mut text = String::from(DETAIL_HEADER);
        for r in rows {
            text.push('\n');
            text.push_str(&r);
        }
        text.push('\n');
        std::fs::write(&p, text).expect("write the runs table");
        println!("Runs table: {p}");
    }
    if let Some(p) = diagnose {
        let mut rows = diagnoses.into_inner().unwrap();
        rows.sort_by_key(|r| {
            r.split(',')
                .nth(1)
                .and_then(|n| n.parse::<u64>().ok())
                .unwrap_or(0)
        });
        let mut text = String::from(DIAGNOSIS_HEADER);
        for r in rows {
            text.push('\n');
            text.push_str(&r);
        }
        text.push('\n');
        if let Some(dir) = Path::new(&p).parent() {
            std::fs::create_dir_all(dir).expect("make the results folder");
        }
        std::fs::write(&p, text).expect("write the diagnosis");
        println!(
            "Diagnosis: {p} ({} lost blinds played again with deeper search, {} beaten)",
            t.deep, t.deep_won
        );
    }
    assert_eq!(t.refused, 0, "the bot made a move the engine refused");
}
