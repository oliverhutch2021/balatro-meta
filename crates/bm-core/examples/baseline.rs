//! The baseline: the greedy bot plays fresh-seed runs on every thread and reports as it goes.
//!
//!     cargo run --release -p bm-core --example baseline -- [--runs N] [--seconds S] [--threads T]
//!         [--seed BASE] [--out results/baseline/greedy.json]
//!
//! With `BM_WATCH_DIR` set (the job runner sets it), it writes there for the live viewer:
//! `progress.json` about once a second (runs, runs a second, win rate, final antes, what ended the
//! runs, the jokers bought and held, the hands played, the best runs so far), and runs to replay in
//! `samples/` (a seed and every move; the viewer replays them move by move with the same engine):
//! the best runs so far (`best-*.json`, kept up to date as better runs turn up) and every few seconds a
//! typical one picked at random (`typical-*.json`). Best means: won first, and among wins the biggest
//! single hand; among losses, the furthest blind reached, then how close the run came to its target.
//! The final numbers go to `--out`.
//!
//! Seeds come from a fixed sampler (`--seed`), so a rerun plays the same runs. The bot sees only what
//! the screen shows (bots.rs works on fair copies).

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bm_core::blinds::BlindKind;
use bm_core::bm_rng::SeedSampler;
use bm_core::bots::greedy_move;
use bm_core::env::{apply, MoveKind};
use bm_core::joker_data::{JokerKind, N_JOKERS};
use bm_core::packs::PackCard;
use bm_core::poker::HandType;
use bm_core::shop::RowItem;
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

fn run_seed(base: u64, i: u64) -> String {
    SeedSampler::new(base.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ i).next_seed()
}

/// One run by the greedy bot, start to end.
fn play(seed: String, salt: u64, keep: bool) -> RunSummary {
    let mut run = Run::new(RunConfig::new(&seed));
    let mut s = RunSummary {
        record: keep.then(Vec::new),
        ..RunSummary::default()
    };
    let mut step = 0u64;
    while run.phase != Phase::GameOver && !run.won && step < MAX_STEPS {
        let (k, a) = greedy_move(&run, salt << 20 | step);
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
            MoveKind::CashOut => s.blinds_beaten += 1,
            _ => {}
        }
        let chips_before = run.round_chips;
        if apply(&mut run, k, a).is_err() {
            s.refused = true; // never expected: the bot only makes legal moves (tested)
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
        }
        if let Some(r) = s.record.as_mut() {
            r.push((k as u8, a));
        }
        step += 1;
    }
    s.seed = seed;
    s.won = run.won;
    s.moves = step;
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
    s
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
    let mut o = String::new();
    let _ = write!(
        o,
        "{{\n \"kind\": \"baseline\",\n \"bot\": \"greedy bot\",\n \"deck\": \"Red Deck\",\n \"stake\": \"White Stake\",\n \
         \"done\": {},\n \"updated\": {},\n \"elapsed\": {},\n \"threads\": {},\n \"target\": {},\n \"runs\": {},\n \
         \"rate\": {},\n \"moves\": {},\n \"wins\": {},\n \"win_rate\": {},\n \"win_ci\": [{}, {}],\n \"mean_ante\": {},\n \
         \"ante_hist\": {:?},\n \"blinds_beaten_per_run\": {},\n \"refused_moves\": {},\n \"lost_to\": [{}],\n \
         \"hands_played\": [{}],\n \"jokers_bought\": {},\n \"jokers_held_at_end\": {},\n \"jokers_in_wins\": {},\n \
         \"per_run\": {{\"moves\": {}, \"jokers_bought\": {}, \"rerolls\": {}, \"skips\": {}, \"packs\": {}, \
         \"vouchers\": {}, \"money_at_end\": {}}},\n \"best_runs\": {},\n \"history\": [{}]\n}}\n",
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
    o
}

fn sample_json(s: &RunSummary, record: &[(u8, u64)], pick: &str, run: u64) -> String {
    let moves: Vec<String> = record.iter().map(|(k, a)| format!("[{k}, {a}]")).collect();
    format!(
        "{{\"seed\": {}, \"bot\": \"greedy bot\", \"source\": \"baseline\", \"pick\": {}, \"run\": {run}, \
         \"label\": {}, \"result\": {}, \"won\": {}, \"ante\": {}, \"stage\": {}, \"closeness\": {}, \
         \"best_hand\": {}, \"moves\": [{}]}}\n",
        esc(&s.seed),
        esc(pick),
        esc(if pick == "best" { "one of the baseline's best runs (greedy bot)" } else { "a typical baseline run (greedy bot)" }),
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
    let target: u64 = opt("--runs")
        .and_then(|s| s.replace('_', "").parse().ok())
        .unwrap_or(1_000_000);
    let seconds: Option<f64> = opt("--seconds").and_then(|s| s.parse().ok());
    let base: u64 = opt("--seed").and_then(|s| s.parse().ok()).unwrap_or(1);
    let threads: usize = match opt("--threads").and_then(|s| s.parse().ok()) {
        Some(t) if t > 0 => t,
        _ => std::thread::available_parallelism().map_or(1, |n| n.get()),
    };
    let out = opt("--out");
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
        "Baseline: the greedy bot on {threads} threads, {target} runs (Red Deck, White Stake)."
    );

    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut local = Totals::new();
                    loop {
                        let over = deadline.is_some_and(|d| start.elapsed() >= d);
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= target || over {
                            break;
                        }
                        let s = play(run_seed(base, i), i, true);
                        local.add(&s);
                        let mut b = best.lock().unwrap();
                        if b.wants(&s) {
                            b.add(i, s);
                        } else if i.is_multiple_of(17) {
                            drop(b);
                            *offer.lock().unwrap() = Some((i, s));
                        }
                        if local.runs >= 64 {
                            totals.lock().unwrap().merge(&local);
                            local = Totals::new();
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
                                    write_atomic(&f, &sample_json(s, rec, "best", *i));
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
                                    &sample_json(&s, rec, "typical", i),
                                );
                                prune_typical(&dir);
                                last_sample = Instant::now();
                            }
                        }
                    }
                }
                if last_line.elapsed() >= Duration::from_secs(10) || done {
                    last_line = Instant::now();
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
        done: true,
    };
    let text = report_json(&r);
    if let Some(p) = out {
        let p = Path::new(&p);
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).expect("make the results folder");
        }
        std::fs::write(p, &text).expect("write the results");
        println!("Results: {}", p.display());
    }
    assert_eq!(
        t.refused, 0,
        "the greedy bot made a move the engine refused"
    );
}
