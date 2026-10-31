//! The teacher's view of a player's decisions, for `make compare-me` (`scripts/compare_me.py` writes the input
//! and reads the output): each recorded run is replayed from its seed with the moves `scripts/replay_me.py`
//! found, and at each decision of the player's the teacher says what it would have done there and what it
//! thinks each choice is worth (`teacher::judge_option`: in a round the chance of beating the blind, between
//! rounds the value of the position). The teacher sees only the screen and fair copies, as when it plays.
//!
//!     cargo run --release -p bm-core --example judge -- --runs FILE --out FILE
//!         [--value-net a.bin,b.bin] [--value-weight W] [--teacher-options precise] [--threads N]
//!
//! The input has one run per line, tab-separated: its name, its seed and its steps; steps are separated by `|`,
//! each a comma-separated list of `kind:arg` moves (the player's reorders first). The output has one line per
//! decision (JSON).

use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use bm_core::content::BOOSTERS;
use bm_core::env::{apply, mask_positions, MoveKind};
use bm_core::learned;
use bm_core::packs::PackCard;
use bm_core::shop::RowItem;
use bm_core::state::{Phase, Run, RunConfig};
use bm_core::teacher::{self, Measure};

struct Decision {
    run: usize,
    step: usize,
    salt: u64,
    before: Run,
    player: Vec<(MoveKind, u64)>,
}

fn parse_moves(s: &str) -> Vec<(MoveKind, u64)> {
    s.split(',')
        .filter(|m| !m.is_empty())
        .map(|m| {
            let (k, a) = m.split_once(':').expect("a move is kind:arg");
            let k =
                MoveKind::from_index(k.parse().expect("a move kind")).expect("a known move kind");
            (k, a.parse().expect("a move argument"))
        })
        .collect()
}

fn phase_name(p: Phase) -> &'static str {
    match p {
        Phase::BlindSelect => "BLIND_SELECT",
        Phase::Round => "SELECTING_HAND",
        Phase::RoundEval => "ROUND_EVAL",
        Phase::Shop => "SHOP",
        Phase::Pack => "SMODS_BOOSTER_OPENED",
        Phase::GameOver => "GAME_OVER",
    }
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn cards(run: &Run, mask: u64) -> String {
    mask_positions(mask)
        .into_iter()
        .filter_map(|p| run.hand.get(p).map(|&id| run.card(id).key()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A move in words, and what it was done to (for telling two choices apart: card ids, item keys).
fn describe(run: &Run, kind: MoveKind, arg: u64) -> (String, String) {
    let i = (arg & 0xff) as usize;
    let targets = arg >> 8;
    let on = |t: u64| {
        if t == 0 {
            String::new()
        } else {
            format!(" on {}", cards(run, t))
        }
    };
    let ids = |mask: u64| {
        let mut v: Vec<u32> = mask_positions(mask)
            .into_iter()
            .filter_map(|p| run.hand.get(p).copied())
            .collect();
        v.sort_unstable();
        format!("{v:?}")
    };
    let row = |i: usize| -> String {
        run.shop
            .as_ref()
            .and_then(|s| s.row.get(i))
            .map(|r| match &r.item {
                RowItem::Joker(j) => j.kind.name().to_string(),
                RowItem::Consumable(c) => c.kind.name().to_string(),
                RowItem::Card(c) => c.key(),
            })
            .unwrap_or_default()
    };
    match kind {
        MoveKind::SelectBlind => ("Play the blind".into(), String::new()),
        MoveKind::SkipBlind => ("Skip the blind".into(), String::new()),
        MoveKind::CashOut => ("Cash out".into(), String::new()),
        MoveKind::LeaveShop => ("Leave the shop".into(), String::new()),
        MoveKind::RerollBoss => ("Reroll the boss".into(), String::new()),
        MoveKind::Reroll => ("Reroll the shop".into(), String::new()),
        MoveKind::Play => (format!("Play {}", cards(run, arg)), ids(arg)),
        MoveKind::Discard => (format!("Discard {}", cards(run, arg)), ids(arg)),
        MoveKind::Buy => (format!("Buy {}", row(i)), format!("row {i}")),
        MoveKind::BuyAndUse => (format!("Buy and use {}", row(i)), format!("row {i}")),
        MoveKind::RedeemVoucher => {
            let v = run
                .shop
                .as_ref()
                .and_then(|s| s.vouchers.get(i))
                .map(|v| v.voucher.name())
                .unwrap_or_default();
            (format!("Redeem {v}"), format!("voucher {i}"))
        }
        MoveKind::OpenBooster => {
            let b = run
                .shop
                .as_ref()
                .and_then(|s| s.boosters.get(i).and_then(|b| b.as_ref()))
                .map(|b| BOOSTERS[b.booster as usize].key)
                .unwrap_or_default();
            (format!("Open {b}"), format!("pack {i}"))
        }
        MoveKind::SellJoker => {
            let j = run.jokers.get(i);
            (
                format!("Sell {}", j.map(|j| j.kind.name()).unwrap_or_default()),
                format!("joker {}", j.map_or(0, |j| j.sort_id)),
            )
        }
        MoveKind::SellConsumable | MoveKind::UseConsumable => {
            let c = run.consumables.get(i);
            let verb = if kind == MoveKind::SellConsumable {
                "Sell"
            } else {
                "Use"
            };
            (
                format!(
                    "{verb} {}{}",
                    c.map(|c| c.kind.name()).unwrap_or_default(),
                    on(targets)
                ),
                format!("{verb} {} {}", c.map_or(0, |c| c.sort_id), ids(targets)),
            )
        }
        MoveKind::PackChoose => {
            let name = run
                .pack
                .as_ref()
                .and_then(|p| p.cards.get(i))
                .map(|c| match c {
                    PackCard::Joker(j) => j.kind.name().to_string(),
                    PackCard::Consumable(c) => c.kind.name().to_string(),
                    PackCard::Card(c) => c.key(),
                })
                .unwrap_or_default();
            (
                format!("Take {name}{}", on(targets)),
                format!("card {i} {}", ids(targets)),
            )
        }
        MoveKind::PackSkip => ("Skip the pack".into(), String::new()),
        MoveKind::MoveCard | MoveKind::MoveJoker | MoveKind::SortRank | MoveKind::SortSuit => {
            ("Reorder".into(), String::new())
        }
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
    let input = opt("--runs").expect("--runs FILE");
    let out = opt("--out").expect("--out FILE");
    let threads: usize = match opt("--threads").and_then(|s| s.parse().ok()) {
        Some(t) if t > 0 => t,
        _ => std::thread::available_parallelism().map_or(1, |n| n.get()),
    };
    if let Some(o) = opt("--teacher-options") {
        let names: Vec<String> = o
            .split(',')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
        teacher::set_options(&names).unwrap_or_else(|e| panic!("{e}"));
    }
    if let Some(p) = opt("--value-net") {
        learned::load(&p).unwrap_or_else(|e| panic!("{e}"));
    }
    if let Some(w) = opt("--value-weight") {
        learned::set_weight(w.parse().expect("--value-weight takes a number"));
    }

    // Replay each run, keeping the position before each of the player's decisions.
    let text = std::fs::read_to_string(&input).expect("the runs file");
    let mut names = Vec::new();
    let mut decisions = Vec::new();
    for (r, line) in text.lines().filter(|l| !l.trim().is_empty()).enumerate() {
        let mut parts = line.split('\t');
        let name = parts.next().unwrap_or("").to_string();
        let seed = parts.next().expect("a seed").to_string();
        let mut run = Run::new(RunConfig::new(&seed));
        let mut done = 0u64;
        for (step, s) in parts.next().unwrap_or("").split('|').enumerate() {
            let moves = parse_moves(s);
            if moves.iter().any(|m| !teacher::reorders(m.0)) && run.phase != Phase::GameOver {
                decisions.push(Decision {
                    run: r,
                    step,
                    salt: done,
                    before: run.clone(),
                    player: moves.clone(),
                });
            }
            for (k, a) in moves {
                if apply(&mut run, k, a).is_err() {
                    eprintln!("{name}: the engine refused a recorded move at step {step}");
                    break;
                }
                done += 1;
            }
        }
        names.push((name, seed));
    }
    println!(
        "{} decisions in {} runs, on {threads} threads",
        decisions.len(),
        names.len()
    );

    let next = AtomicUsize::new(0);
    let lines: Mutex<Vec<(usize, String)>> = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(d) = decisions.get(i) else { break };
                teacher::new_run();
                // The player's decision: its reorders made, then the move (or the first of two).
                let mut p_run = d.before.clone();
                let mut p_move = None;
                for &(k, a) in &d.player {
                    if teacher::reorders(k) && p_move.is_none() {
                        let _ = apply(&mut p_run, k, a);
                    } else if p_move.is_none() {
                        p_move = Some((k, a));
                    }
                }
                let p_move = p_move.expect("a decision has a move");
                let (b_run, _, b_move) = teacher::decision(&d.before, d.salt);
                let (p_text, p_id) = describe(&p_run, p_move.0, p_move.1);
                let (b_text, b_id) = describe(&b_run, b_move.0, b_move.1);
                let agree = p_move.0 == b_move.0 && p_id == b_id;
                teacher::new_run();
                let (measure, pv) = teacher::judge_option(&p_run, d.salt, p_move);
                let bv = if agree {
                    pv
                } else {
                    teacher::new_run();
                    teacher::judge_option(&b_run, d.salt, b_move).1
                };
                let (name, seed) = &names[d.run];
                let r = &d.before;
                let num = |x: f64| {
                    if x.is_finite() {
                        format!("{x:.6}")
                    } else {
                        "null".into()
                    }
                };
                let line = format!(
                    "{{\"run\":\"{}\",\"seed\":\"{}\",\"step\":{},\"ante\":{},\"round\":{},\"phase\":\"{}\",\
                     \"money\":{},\"player\":\"{}\",\"player_kind\":{},\"bot\":\"{}\",\"bot_kind\":{},\
                     \"agree\":{},\"measure\":\"{}\",\"player_value\":{},\"bot_value\":{}}}",
                    esc(name),
                    esc(seed),
                    d.step,
                    r.ante,
                    r.round,
                    phase_name(r.phase),
                    r.money,
                    esc(&p_text),
                    p_move.0 as u8,
                    esc(&b_text),
                    b_move.0 as u8,
                    agree,
                    if measure == Measure::BeatChance {
                        "beat"
                    } else {
                        "value"
                    },
                    num(pv),
                    num(bv),
                );
                lines.lock().unwrap().push((i, line));
            });
        }
    });
    let mut lines = lines.into_inner().unwrap();
    lines.sort();
    let mut f = std::fs::File::create(&out).expect("the output file");
    for (_, l) in &lines {
        writeln!(f, "{l}").unwrap();
    }
    println!("{} decisions judged: {out}", lines.len());
}
