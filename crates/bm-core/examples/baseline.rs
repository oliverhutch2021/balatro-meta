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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bm_core::blinds::{BlindKind, SlotState};
use bm_core::bots::greedy_move;
use bm_core::env::{apply, legal_kinds, run_seed, MoveKind};
use bm_core::joker_data::{JokerKind, N_JOKERS};
use bm_core::learned::{self, le_bytes, npy_bytes, Rows};
use bm_core::observe::{encode, encode_value, screen, OBS_LEN, VALUE_INPUT_VERSION, VALUE_LEN};
use bm_core::packs::PackCard;
use bm_core::poker::HandType;
use bm_core::shop::RowItem;
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
    let mut run = Run::new(RunConfig::new(&seed));
    let mut s = RunSummary {
        record: keep.then(Vec::new),
        recorded: RECORD.load(Ordering::Relaxed).then(Recorded::default),
        ..RunSummary::default()
    };
    let mut input = Vec::with_capacity(VALUE_LEN);
    let moves = RECORD_MODE.load(Ordering::Relaxed) == 2;
    let mut step = 0u64;
    while run.phase != Phase::GameOver && !run.won && step < MAX_STEPS {
        let keep_this = match s.recorded.as_mut() {
            Some(r) if moves || learned::values_here(&run) => {
                if moves {
                    encode(&screen(&run), &mut input);
                } else {
                    encode_value(&screen(&run), &mut input);
                }
                r.rows.push(&input);
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
            MoveKind::CashOut => s.blinds_beaten += 1,
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
    if DETAIL.load(Ordering::Relaxed) {
        s.detail = Some(detail_row(&run, &s));
    }
    s
}

/// Whether runs keep a `--runs-csv` row.
static DETAIL: AtomicBool = AtomicBool::new(false);

/// Whether runs keep their valued positions (`--record`).
static RECORD: AtomicBool = AtomicBool::new(false);

/// Runs per chunk of recorded positions.
const RUNS_PER_CHUNK: u64 = 250;

/// The columns of a chunk's `lab.npy` (f32), one row per position.
const LABELS: [&str; 8] = [
    "run",
    "step",
    "phase",
    "ante",
    "stage",
    "won",
    "end_stage",
    "end_ante",
];

/// A run's valued positions: the network's input, and where in the run each was (step, phase, ante,
/// stage).
#[derive(Default)]
struct Recorded {
    rows: Rows,
    at: Vec<[f32; 4]>,
    /// With `--record-moves`: each position's move (kind, argument) and the kinds it allowed (bits).
    moves: Vec<[i64; 3]>,
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
    let done = run
        .slots
        .iter()
        .filter(|s| matches!(s, SlotState::Defeated | SlotState::Skipped))
        .count();
    ((run.ante.clamp(1, 8) - 1) * 3) as f32 + done as f32
}

/// One finished run's recording, waiting for the rest of its chunk.
struct RecordedRun {
    run: u64,
    seed: String,
    won: bool,
    end_stage: u32,
    end_ante: usize,
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
        let name = |part: &str| self.dir.join(format!("chunk-{chunk:05}-{part}"));
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
                npy_bytes("<f4", &[n, LABELS.len()], &le_bytes(&lab, f32::to_le_bytes)),
            ),
            (
                "runs.json",
                format!("[\n{}\n]\n", list.join(",\n")).into_bytes(),
            ),
        ];
        let mut files = Vec::from(files);
        if RECORD_MODE.load(Ordering::Relaxed) == 2 {
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
    }

    fn manifest(&self, about: &str) {
        let labels: Vec<String> = LABELS.iter().map(|l| esc(l)).collect();
        let (input, inputs) = if RECORD_MODE.load(Ordering::Relaxed) == 2 {
            ("screen", OBS_LEN)
        } else {
            ("value", VALUE_LEN)
        };
        let moves_kept = RECORD_MODE.load(Ordering::Relaxed) == 1;
        let text = format!(
            "{{\n \"format\": 1,\n \"input\": \"{input}\",\n \"input_version\": {VALUE_INPUT_VERSION},\n \
             \"inputs\": {inputs},\n \"runs\": {},\n \"rows\": {},\n \"chunks\": {},\n \
             \"runs_per_chunk\": {RUNS_PER_CHUNK},\n \"labels\": [{}],\n \"moves_kept\": {moves_kept},\n{about}\n}}\n",
            self.runs,
            self.rows,
            self.chunks,
            labels.join(", "),
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
fn load_replay(dir: &Path) -> (Vec<Replayed>, String) {
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
            });
        }
        assert_eq!(at, kinds.len(), "{stem}: every move belongs to a run");
    }
    let from = dir
        .file_name()
        .map_or(String::new(), |n| n.to_string_lossy().into_owned());
    let about = format!("{},\n \"replayed_from\": {}", about.join(",\n"), esc(&from));
    (runs, about)
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
    let mut o = String::new();
    let _ = write!(
        o,
        "{{\n \"kind\": \"baseline\",\n \"bot\": {},\n \"deck\": \"Red Deck\",\n \"stake\": \"White Stake\",\n \
         \"done\": {},\n \"updated\": {},\n \"elapsed\": {},\n \"threads\": {},\n \"target\": {},\n \"runs\": {},\n \
         \"rate\": {},\n \"moves\": {},\n \"wins\": {},\n \"win_rate\": {},\n \"win_ci\": [{}, {}],\n \"mean_ante\": {},\n \
         \"ante_hist\": {:?},\n \"blinds_beaten_per_run\": {},\n \"refused_moves\": {},\n \"lost_to\": [{}],\n \
         \"hands_played\": [{}],\n \"jokers_bought\": {},\n \"jokers_held_at_end\": {},\n \"jokers_in_wins\": {},\n \
         \"per_run\": {{\"moves\": {}, \"jokers_bought\": {}, \"rerolls\": {}, \"skips\": {}, \"packs\": {}, \
         \"vouchers\": {}, \"money_at_end\": {}}},\n \"best_runs\": {},\n \"history\": [{}]\n}}\n",
        esc(r.bot),
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

fn sample_json(s: &RunSummary, record: &[(u8, u64)], pick: &str, run: u64, bot: &str) -> String {
    let moves: Vec<String> = record.iter().map(|(k, a)| format!("[{k}, {a}]")).collect();
    format!(
        "{{\"seed\": {}, \"bot\": {}, \"source\": \"baseline\", \"pick\": {}, \"run\": {run}, \
         \"label\": {}, \"result\": {}, \"won\": {}, \"ante\": {}, \"stage\": {}, \"closeness\": {}, \
         \"best_hand\": {}, \"moves\": [{}]}}\n",
        esc(&s.seed),
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
    let replay = opt("--replay").map(|d| load_replay(Path::new(&d)));
    let runs_wanted: Option<u64> = opt("--runs").and_then(|s| s.replace('_', "").parse().ok());
    let target: u64 = match &replay {
        Some((runs, _)) => runs_wanted.map_or(runs.len() as u64, |n| n.min(runs.len() as u64)),
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
    let bot_label = opt("--bot").unwrap_or_else(|| "greedy".into());
    let (bot, bot_name): (Bot, &str) = match (bot_label.as_str(), value_net.is_some()) {
        ("greedy", _) => (greedy_move, "greedy bot"),
        ("teacher", false) => (teacher_move, "teacher bot"),
        ("teacher", true) => (teacher_move, "teacher bot with a learned value"),
        (other, _) => panic!("unknown bot {other:?}: greedy or teacher"),
    };
    let bot_name = if replay.is_some() {
        "replayed runs"
    } else {
        bot_name
    };
    // How each run ended, in seed order (for paired comparisons), when there aren't too many runs.
    let stages: Option<Mutex<Vec<u8>>> =
        (target <= 50_000).then(|| Mutex::new(vec![b'?'; target as usize]));
    let (record_dir, mode) = match (opt("--record"), opt("--record-moves")) {
        (Some(d), None) => (Some(d), 1),
        (None, Some(d)) => (Some(d), 2),
        (None, None) => (None, 0),
        _ => panic!("--record or --record-moves, not both"),
    };
    RECORD_MODE.store(mode, Ordering::Relaxed);
    RECORD.store(record_dir.is_some(), Ordering::Relaxed);
    let recorder = record_dir.map(|d| Mutex::new(Recorder::new(PathBuf::from(d), target)));
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
        "Baseline: the {bot_name} on {threads} threads, {target} runs (Red Deck, White Stake)."
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
                        let mut s = match &replay {
                            None => play(run_seed(base, i), i, true, bot, None),
                            Some((runs, _)) => {
                                let r = &runs[i as usize];
                                let s = play(r.seed.clone(), i, true, bot, Some(&r.moves));
                                if s.refused
                                    || s.unfinished
                                    || s.moves != r.moves.len() as u64
                                    || s.won != r.won
                                    || s.stage != r.end_stage
                                {
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
                        if let Some(st) = &stages {
                            st.lock().unwrap()[i as usize] = b'A' + s.stage.min(24) as u8;
                        }
                        if let (Some(rec), Some(r)) = (s.recorded.take(), &recorder) {
                            r.lock().unwrap().add(RecordedRun {
                                run: i,
                                seed: s.seed.clone(),
                                won: s.won,
                                end_stage: s.stage,
                                end_ante: s.end,
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
    // labels would be wrong, so the new recording is left without a manifest (the trainer refuses it).
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
        std::process::exit(1);
    }
    if let Some(r) = recorder {
        let mut r = r.into_inner().unwrap();
        r.finish();
        let opts = bm_core::teacher::options();
        let about = match &replay {
            // A replay keeps how the runs were played; only the encoding is this code's.
            Some((_, about)) => about.clone(),
            None => format!(
                " \"bot\": {},\n \"seed_base\": {base},\n \"value_net\": {},\n \"value_weight\": {},\n \
                 \"teacher_options\": [{}],\n \"wins\": {},\n \"seconds\": {}",
                esc(bot_name),
                value_net.as_deref().map_or("null".into(), esc),
                num(learned::weight()),
                opts.iter().map(|o| esc(o)).collect::<Vec<_>>().join(", "),
                t.wins,
                num(el)
            ),
        };
        r.manifest(&about);
        println!(
            "Recorded {} positions from {} runs in {} chunks: {}",
            r.rows,
            r.runs,
            r.chunks,
            r.dir.display()
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
        ",\n \"seed_base\": {base},\n \"value_net\": {},\n \"value_weight\": {},\n \"teacher_options\": [{}]",
        value_net.as_deref().map_or("null".into(), esc),
        if value_net.is_some() {
            num(learned::weight())
        } else {
            "null".into()
        },
        opts.iter().map(|o| esc(o)).collect::<Vec<_>>().join(", ")
    );
    if let Some(st) = stages {
        let st = String::from_utf8(st.into_inner().unwrap()).expect("letters");
        tail += &format!(",\n \"run_stages\": {}", esc(&st));
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
    assert_eq!(t.refused, 0, "the bot made a move the engine refused");
}
