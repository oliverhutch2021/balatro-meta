//! A learned value: a network trained on how the teacher's runs actually ended (`scripts/train_value.py`),
//! read from the screen alone (`observe::encode_value`), so it sees exactly what a player sees. It gives the
//! chance of winning the run from a position. When one is loaded, the value model (`value.rs`) adds the log
//! of that chance, so the teacher's search weighs what wins at ante 8, which a hand-made formula can't see.
//!
//! The teacher's own positions are its training data: `baseline --record DIR` keeps every position it
//! values between rounds (the blind choice, the shop, open packs), each labelled with how its run ended
//! ([`Rows`], [`npy_bytes`]). Train on them, play with the network, record again: each round of this
//! learns from a better player.
//!
//! The weights file is our own format (little-endian):
//! - `b"BMVN"`, format version (u32, 1), input layout version (u32, `VALUE_INPUT_VERSION`), input length
//!   (u32, `VALUE_LEN`), layer count (u32); any layout of `observe::VALUE_LAYOUTS` loads: an older layout's
//!   floats are the start of this engine's input, so its first layer reads just those;
//! - per layer: inputs (u32), outputs (u32), activation (u32: 0 none, 1 ReLU), then the weights as f32 in
//!   input-major order (all of input 0's outputs, then input 1's...) and the biases (f32 per output).
//!
//! The first output is the logit of winning; the trainer folds the input scaling into the first layer.
//! Input-major weights let the first layer skip the inputs that are zero (most of them: one-hots).

use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use crate::blinds::SlotState;
use crate::observe::{
    encode_value, screen, STAKE_PART, VALUE_INPUT_VERSION, VALUE_LAYOUTS, VALUE_LEN,
};
use crate::state::{Phase, Run};

/// One dense layer.
#[derive(Clone, Debug)]
pub struct Layer {
    inputs: usize,
    outputs: usize,
    relu: bool,
    /// `inputs * outputs`, input-major.
    weights: Vec<f32>,
    bias: Vec<f32>,
}

/// A small multilayer network.
#[derive(Clone, Debug)]
pub struct Net {
    layers: Vec<Layer>,
}

fn take<'a>(b: &mut &'a [u8], n: usize) -> Result<&'a [u8], String> {
    if b.len() < n {
        return Err("the weights file is cut short".into());
    }
    let (head, rest) = b.split_at(n);
    *b = rest;
    Ok(head)
}

fn read_u32(b: &mut &[u8]) -> Result<u32, String> {
    let s = take(b, 4)?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn read_f32s(b: &mut &[u8], n: usize) -> Result<Vec<f32>, String> {
    let s = take(b, n * 4)?;
    Ok(s.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect())
}

impl Net {
    /// A network from the bytes of a weights file.
    pub fn from_bytes(mut b: &[u8]) -> Result<Net, String> {
        if take(&mut b, 4)? != b"BMVN" {
            return Err("not a value network file".into());
        }
        let version = read_u32(&mut b)?;
        if version != 1 {
            return Err(format!("unknown value network format {version}"));
        }
        let layout = read_u32(&mut b)?;
        let inputs = read_u32(&mut b)? as usize;
        // An older layout's floats are the start of this engine's input (`VALUE_LAYOUTS`): the network reads
        // just those, as it was trained to.
        if !VALUE_LAYOUTS.contains(&(layout, inputs)) {
            return Err(format!(
                "the network reads input layout {layout} ({inputs} floats), this engine's is \
                 {VALUE_INPUT_VERSION} ({VALUE_LEN}) and it reads layouts {}: retrain it on data recorded by \
                 this engine",
                VALUE_LAYOUTS
                    .iter()
                    .map(|(v, n)| format!("{v} ({n})"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        let n = read_u32(&mut b)? as usize;
        let mut layers = Vec::with_capacity(n);
        let mut width = inputs;
        for _ in 0..n {
            let i = read_u32(&mut b)? as usize;
            let o = read_u32(&mut b)? as usize;
            let act = read_u32(&mut b)?;
            if i != width || o == 0 || act > 1 {
                return Err("the layers don't fit together".into());
            }
            let weights = read_f32s(&mut b, i * o)?;
            let bias = read_f32s(&mut b, o)?;
            layers.push(Layer {
                inputs: i,
                outputs: o,
                relu: act == 1,
                weights,
                bias,
            });
            width = o;
        }
        if !b.is_empty() {
            return Err("trailing bytes after the last layer".into());
        }
        if layers.is_empty() {
            return Err("no layers".into());
        }
        Ok(Net { layers })
    }

    /// How many outputs it has (the win logit, the ending stage's, and maybe the depth outputs).
    pub fn outputs(&self) -> usize {
        self.layers.last().map_or(0, |l| l.outputs)
    }

    /// How many floats of the input the network reads (its layout's: `VALUE_LAYOUTS`).
    pub fn inputs(&self) -> usize {
        self.layers[0].inputs
    }

    /// The network's outputs for an input of `VALUE_LEN` floats.
    pub fn forward(&self, x: &[f32]) -> Vec<f32> {
        let mut h: Vec<f32> = x.to_vec();
        for l in &self.layers {
            let mut out = l.bias.clone();
            for (i, &v) in h.iter().enumerate().take(l.inputs) {
                if v == 0.0 {
                    continue;
                }
                let row = &l.weights[i * l.outputs..(i + 1) * l.outputs];
                for (o, w) in out.iter_mut().zip(row) {
                    *o += v * w;
                }
            }
            if l.relu {
                for o in &mut out {
                    *o = o.max(0.0);
                }
            }
            h = out;
        }
        h
    }

    /// The first output, using the inference thread's reusable buffers. The teacher only uses the win
    /// logit; the other outputs predict the ending stage for training. Keep `forward` for full-output
    /// exports and parity checks. Accumulation order and f32 arithmetic match it exactly.
    pub fn first_output(&self, x: &[f32]) -> f32 {
        INFERENCE.with(|w| {
            let mut w = w.borrow_mut();
            w.read(x);
            self.first_with(&mut w)
        })
    }

    fn first_with(&self, w: &mut Inference) -> f32 {
        let last = self.hidden_with(w);
        let mut out = last.bias[0];
        if self.layers.len() == 1 {
            for &(i, v) in &w.input {
                if i >= last.inputs {
                    break;
                }
                out += v * last.weights[i * last.outputs];
            }
        } else {
            for (i, &v) in w.current.iter().enumerate().take(last.inputs) {
                if v != 0.0 {
                    out += v * last.weights[i * last.outputs];
                }
            }
        }
        if last.relu {
            out.max(0.0)
        } else {
            out
        }
    }

    /// Every output (the win logit, then the ending stage's logits) into `out`, with the inference buffers, in
    /// `forward`'s order of arithmetic.
    fn all_with(&self, w: &mut Inference, out: &mut Vec<f32>) {
        let last = self.hidden_with(w);
        out.clone_from(&last.bias);
        let mut add = |i: usize, v: f32| {
            for (o, weight) in out
                .iter_mut()
                .zip(&last.weights[i * last.outputs..][..last.outputs])
            {
                *o += v * weight;
            }
        };
        if self.layers.len() == 1 {
            for &(i, v) in &w.input {
                if i >= last.inputs {
                    break;
                }
                add(i, v);
            }
        } else {
            for (i, &v) in w.current.iter().enumerate().take(last.inputs) {
                if v != 0.0 {
                    add(i, v);
                }
            }
        }
        if last.relu {
            for o in out.iter_mut() {
                *o = o.max(0.0);
            }
        }
    }

    /// The hidden layers on the input `w` holds, their last activations left in `w.current`; the last layer,
    /// for the caller to apply.
    fn hidden_with(&self, w: &mut Inference) -> &Layer {
        let (last, hidden) = self
            .layers
            .split_last()
            .expect("a validated network has layers");
        for (layer_index, layer) in hidden.iter().enumerate() {
            w.next.clone_from(&layer.bias);
            if layer_index == 0 {
                for &(i, v) in &w.input {
                    if i >= layer.inputs {
                        break;
                    }
                    for (o, weight) in w
                        .next
                        .iter_mut()
                        .zip(&layer.weights[i * layer.outputs..][..layer.outputs])
                    {
                        *o += v * weight;
                    }
                }
            } else {
                for (i, &v) in w.current.iter().enumerate().take(layer.inputs) {
                    if v == 0.0 {
                        continue;
                    }
                    for (o, weight) in w
                        .next
                        .iter_mut()
                        .zip(&layer.weights[i * layer.outputs..][..layer.outputs])
                    {
                        *o += v * weight;
                    }
                }
            }
            if layer.relu {
                for o in &mut w.next {
                    *o = o.max(0.0);
                }
            }
            std::mem::swap(&mut w.current, &mut w.next);
        }
        last
    }
}

/// Reused across all networks in an ensemble and across decisions on one worker. Input nonzeros are
/// collected once per ensemble, in their original order; no full input copy or auxiliary output vector.
#[derive(Default)]
struct Inference {
    input: Vec<(usize, f32)>,
    current: Vec<f32>,
    next: Vec<f32>,
}

impl Inference {
    fn read(&mut self, x: &[f32]) {
        self.input.clear();
        self.input
            .extend(x.iter().copied().enumerate().filter(|&(_, v)| v != 0.0));
    }
}

thread_local! {
    static INFERENCE: RefCell<Inference> = RefCell::new(Inference::default());
}

static NET: OnceLock<Vec<Net>> = OnceLock::new();

/// Load the value network for this process (the teacher uses it from then on). Several paths joined by commas
/// load an ensemble: the chance of winning is then the mean of the networks' chances.
pub fn load(path: &str) -> Result<(), String> {
    NET.set(read_nets(path)?)
        .map_err(|_| "a value network is already loaded".to_string())
}

/// The networks of a comma-separated list of weights files.
fn read_nets(path: &str) -> Result<Vec<Net>, String> {
    let mut nets = Vec::new();
    for p in path.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let bytes = std::fs::read(p).map_err(|e| format!("{p}: {e}"))?;
        nets.push(Net::from_bytes(&bytes).map_err(|e| format!("{p}: {e}"))?);
    }
    if nets.is_empty() {
        return Err(format!("no value network in {path:?}"));
    }
    Ok(nets)
}

/// Whether a value network is loaded.
pub fn loaded() -> bool {
    NET.get().is_some()
}

/// The weight of the network's log chance in the value (f64 bits; 1 unless set).
static WEIGHT: AtomicU64 = AtomicU64::new(0x3FF0_0000_0000_0000);

/// Set the weight of the network's log chance in the value (`baseline --value-weight`).
pub fn set_weight(w: f64) {
    WEIGHT.store(w.to_bits(), Ordering::Relaxed);
}

pub fn weight() -> f64 {
    f64::from_bits(WEIGHT.load(Ordering::Relaxed))
}

/// Chances are floored this much before their log: a network can't tell 0.1% from 0.3% (a handful of wins
/// in its data), and the log would make that noise loud.
pub const FLOOR: f64 = 0.01;

/// The positions the teacher values, and so the ones the network learns: the blind choice, the shop and
/// open packs. (A position in a round is valued as if the round began again, at its blind choice.)
pub fn values_here(run: &Run) -> bool {
    matches!(run.phase, Phase::BlindSelect | Phase::Shop | Phase::Pack)
}

thread_local! {
    static INPUT: RefCell<Vec<f32>> = RefCell::new(Vec::with_capacity(VALUE_LEN));
    /// The loaded network's chance for inputs seen lately, by a hash of the input (searches value the same
    /// positions many times).
    static SEEN: RefCell<HashMap<u64, f64>> = RefCell::new(HashMap::new());
}

/// A hash of an input's nonzero floats.
fn input_key(x: &[f32]) -> u64 {
    let mut h = DefaultHasher::new();
    for (i, v) in x.iter().enumerate() {
        if *v != 0.0 {
            (i as u32, v.to_bits()).hash(&mut h);
        }
    }
    h.finish()
}

/// The chance of winning the run from this position, by the loaded network (None if none is loaded).
pub fn win_chance(run: &Run) -> Option<f64> {
    chance(run, None)
}

/// The loaded networks' chance from this position of winning (`reach` None), or of getting at least `reach`
/// blinds further than it is (`reach_chance`), remembered by the input for the searches' repeats.
fn chance(run: &Run, reach: Option<usize>) -> Option<f64> {
    let nets = NET.get()?;
    Some(INPUT.with(|x| {
        let mut x = x.borrow_mut();
        encode_value(&screen(run), &mut x);
        let key = input_key(&x)
            ^ reach.map_or(0, |k| (k as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15))
            ^ disagreement_lean().to_bits().rotate_left(17)
            ^ margin().map_or(0, |k| (k as u64 + 7).wrapping_mul(0xC2B2_AE3D_27D4_EB4F));
        if let Some(p) = SEEN.with(|s| s.borrow().get(&key).copied()) {
            return p;
        }
        let from = reach.map(|k| blinds_behind(run) + k);
        let lean = disagreement_lean();
        let mut p = match from {
            None if lean != 0.0 && nets.len() > 1 => {
                let (mean, spread) = mean_and_spread(nets, &x);
                (mean + lean * spread).clamp(0.0, 1.0)
            }
            None => mean_chance(nets, &x),
            Some(f) => mean_reach(nets, &x, f),
        };
        if let (None, Some(k)) = (from, margin()) {
            if let Some(q) = mean_depth_tail(nets, &x, k) {
                p *= q;
            }
        }
        if let Some((stake_nets, mode)) = STAKE_NET.get() {
            // A residual correction learned White's runs too; "correct-stakes-only" keeps White's play the value
            // networks' own. A delta is nothing at White anyway.
            if shows_a_stake(&x) || !crate::teacher::on("correct-stakes-only") {
                p = corrected(p, correction(stake_nets, *mode, &x, from), stake_scale());
            }
        }
        SEEN.with(|s| {
            let mut s = s.borrow_mut();
            if s.len() > 8192 {
                s.clear();
            }
            s.insert(key, p);
        });
        p
    }))
}

fn chance_of(net: &Net, x: &[f32]) -> f64 {
    1.0 / (1.0 + (-(net.first_output(x) as f64)).exp())
}

/// An ensemble's chance: the mean of its networks' chances.
fn mean_chance(nets: &[Net], x: &[f32]) -> f64 {
    INFERENCE.with(|w| {
        let mut w = w.borrow_mut();
        w.read(x);
        nets.iter()
            .map(|n| 1.0 / (1.0 + (-(n.first_with(&mut w) as f64)).exp()))
            .sum::<f64>()
            / nets.len().max(1) as f64
    })
}

/// An ensemble's mean chance and how far its networks disagree about it (the standard deviation of their
/// chances).
fn mean_and_spread(nets: &[Net], x: &[f32]) -> (f64, f64) {
    INFERENCE.with(|w| {
        let mut w = w.borrow_mut();
        w.read(x);
        let ps: Vec<f64> = nets
            .iter()
            .map(|n| 1.0 / (1.0 + (-(n.first_with(&mut w) as f64)).exp()))
            .collect();
        let n = ps.len().max(1) as f64;
        let mean = ps.iter().sum::<f64>() / n;
        let var = ps.iter().map(|p| (p - mean) * (p - mean)).sum::<f64>() / n;
        (mean, var.sqrt())
    })
}

/// Teacher options `doubt=k` and `hope=k`: how many standard deviations of the networks' chances the
/// ensemble's chance of winning moves, down for doubt, up for hope (0 unless set: the plain mean).
fn disagreement_lean() -> f64 {
    crate::teacher::param("hope").unwrap_or(0.0) - crate::teacher::param("doubt").unwrap_or(0.0)
}

/// Blinds behind the run: 3 for each ante before this one, and this ante's beaten or skipped (0 to 23), as the
/// recordings' `stage` counts them (`baseline --record`).
pub fn blinds_behind(run: &Run) -> usize {
    let done = run
        .slots
        .iter()
        .filter(|s| matches!(s, SlotState::Defeated | SlotState::Skipped))
        .count();
    (run.ante.clamp(1, 8) as usize - 1) * 3 + done
}

/// The chance a network's ending-stage outputs give of the run ending at stage `from` or later (0 to 23: lost at
/// that blind, so every blind before it beaten; 24: won): the softmax's share from `from` on. A network without
/// them (one output) gives its win chance.
fn reach_of(out: &[f32], from: usize) -> f64 {
    if out.len() < 2 {
        return 1.0 / (1.0 + (-(out[0] as f64)).exp());
    }
    let logits = &out[1..out.len().min(1 + STAGES)];
    let top = logits.iter().fold(f32::NEG_INFINITY, |m, &v| m.max(v)) as f64;
    let (mut all, mut later) = (0.0, 0.0);
    for (stage, &v) in logits.iter().enumerate() {
        let e = (v as f64 - top).exp();
        all += e;
        if stage >= from.min(logits.len() - 1) {
            later += e;
        }
    }
    later / all
}

/// The ending stage's outputs after the win logit: lost at blind 0 to 23, or won.
const STAGES: usize = 25;

/// A network's chance, if it has the depth outputs (trained on recordings that played their wins on into Endless,
/// `baseline --endless`: the logits of how many blinds a win beats after it, after the ending stage's), that a win
/// from here beats at least `k` blinds after it. None for a network without them.
fn depth_tail(out: &[f32], k: usize) -> Option<f64> {
    let depth = out.get(1 + STAGES..).filter(|d| !d.is_empty())?;
    let top = depth.iter().fold(f32::NEG_INFINITY, |m, &v| m.max(v)) as f64;
    let (mut all, mut deep) = (0.0, 0.0);
    for (d, &v) in depth.iter().enumerate() {
        let e = (v as f64 - top).exp();
        all += e;
        if d >= k {
            deep += e;
        }
    }
    Some(deep / all)
}

/// The mean over the networks that have the depth outputs of their chance that a win goes at least `k` blinds on
/// (None if none has them).
fn mean_depth_tail(nets: &[Net], x: &[f32], k: usize) -> Option<f64> {
    INFERENCE.with(|w| {
        let mut w = w.borrow_mut();
        w.read(x);
        let mut out = Vec::new();
        let tails: Vec<f64> = nets
            .iter()
            .filter(|n| n.outputs() > 1 + STAGES)
            .filter_map(|n| {
                n.all_with(&mut w, &mut out);
                depth_tail(&out, k)
            })
            .collect();
        (!tails.is_empty()).then(|| tails.iter().sum::<f64>() / tails.len() as f64)
    })
}

/// Teacher option `margin=K` (K of 1 or more): the learned term rates a position by the chance of winning with K
/// blinds to spare: the networks' chance of winning times their chance (those with the depth outputs) that the win
/// then beats K more blinds. Two thirds of White's runs end in the same "won", so a barely-won run and a strong
/// one look alike to the win chance alone; the spare blinds are the headroom that beats the big last bosses.
fn margin() -> Option<usize> {
    crate::teacher::param("margin")
        .filter(|&k| k >= 1.0)
        .map(|k| k.round() as usize)
}

/// An ensemble's chance of ending at stage `from` or later: the mean of its networks'.
fn mean_reach(nets: &[Net], x: &[f32], from: usize) -> f64 {
    INFERENCE.with(|w| {
        let mut w = w.borrow_mut();
        w.read(x);
        let mut out = Vec::new();
        nets.iter()
            .map(|n| {
                n.all_with(&mut w, &mut out);
                reach_of(&out, from)
            })
            .sum::<f64>()
            / nets.len().max(1) as f64
    })
}

/// Teacher option `reach=K` (K of 1 or more): the learned term rates a position by the networks' chance of getting
/// at least K blinds further, not of winning. From Purple Stake up a network that knows the stake rates nearly
/// every position under `FLOOR`'s 1% to win, so the log of its win chance is the same everywhere and tells the
/// choices apart by nothing; the chance of beating the next few blinds stays far from 0 and 1 at every stake.
pub fn reach_chance(run: &Run) -> Option<f64> {
    chance(run, reach())
}

fn reach() -> Option<usize> {
    crate::teacher::param("reach")
        .filter(|&k| k >= 1.0)
        .map(|k| k.round() as usize)
}

// ---- The stake correction ----

/// How a second set of networks (`baseline --stake-net`: networks of input layout 3, which shows the stake)
/// corrects the value networks' chance, in log-odds. The value networks keep the judgement they learned from
/// far more runs (White's, so far); the correction only says where the runs show it wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StakeMode {
    /// Networks that learned the gap between the value networks' log-odds of winning and how runs ended
    /// (`train_value.py --offset`, on recordings that keep the value networks' log-odds of each position,
    /// `baseline --offsets`): their first output, the gap, is added. Starting from no gap, they move away from it
    /// only as far as the runs they judge on bear out, at every stake (White too, unless teacher option
    /// "correct-stakes-only").
    Residual,
    /// Networks that learned the chance of winning at every stake: what the stake changes by their reckoning,
    /// their log-odds here less their log-odds for the same position at White (the stake's floats off), is added.
    /// Their own judgement of the position cancels out; nothing at White.
    Delta,
}

impl StakeMode {
    pub fn parse(s: &str) -> Result<StakeMode, String> {
        match s {
            "residual" => Ok(StakeMode::Residual),
            "delta" => Ok(StakeMode::Delta),
            other => Err(format!(
                "unknown stake correction {other:?}: residual or delta"
            )),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            StakeMode::Residual => "residual",
            StakeMode::Delta => "delta",
        }
    }
}

static STAKE_NET: OnceLock<(Vec<Net>, StakeMode)> = OnceLock::new();

/// Load the stake correction's networks for this process (several joined by commas: their corrections' mean).
pub fn load_stake(path: &str, mode: StakeMode) -> Result<(), String> {
    let nets = read_nets(path)?;
    check_stake_nets(&nets).map_err(|e| format!("{path}: {e}"))?;
    STAKE_NET
        .set((nets, mode))
        .map_err(|_| "a stake correction is already loaded".to_string())
}

/// The stake correction loaded, if one is.
pub fn stake_mode() -> Option<StakeMode> {
    STAKE_NET.get().map(|(_, m)| *m)
}

/// A stake correction reads the whole input: a network of an older layout can't see the stake.
fn check_stake_nets(nets: &[Net]) -> Result<(), String> {
    match nets.iter().find(|n| n.inputs() != VALUE_LEN) {
        Some(n) => Err(format!(
            "a stake correction must read this engine's whole input ({VALUE_LEN} floats, layout \
             {VALUE_INPUT_VERSION}: the stake), not {} floats",
            n.inputs()
        )),
        None => Ok(()),
    }
}

/// Whether an input shows a stake above White (layout 3's rule floats, the last `STAKE_PART`, any on).
fn shows_a_stake(x: &[f32]) -> bool {
    x.len() == VALUE_LEN && x[VALUE_LEN - STAKE_PART..].iter().any(|&v| v != 0.0)
}

/// The same input as at White Stake: the stake's rule floats off, everything else as it is.
fn at_white(x: &[f32]) -> Vec<f32> {
    let mut w = x.to_vec();
    let n = w.len();
    w[n.saturating_sub(STAKE_PART)..].fill(0.0);
    w
}

fn logit(p: f64) -> f64 {
    let p = p.clamp(1e-9, 1.0 - 1e-9);
    (p / (1.0 - p)).ln()
}

/// The correction to add to the value networks' log-odds for this input. `from`: with teacher option `reach`,
/// the stage to get to (a delta then compares the chances of getting there; a residual corrects the win chance
/// alone, so it adds nothing, and `baseline` refuses the two together).
fn correction(nets: &[Net], mode: StakeMode, x: &[f32], from: Option<usize>) -> f64 {
    match (mode, from) {
        (StakeMode::Residual, Some(_)) => 0.0,
        (StakeMode::Residual, None) => INFERENCE.with(|w| {
            let mut w = w.borrow_mut();
            w.read(x);
            nets.iter()
                .map(|n| n.first_with(&mut w) as f64)
                .sum::<f64>()
                / nets.len().max(1) as f64
        }),
        (StakeMode::Delta, _) if !shows_a_stake(x) => 0.0,
        (StakeMode::Delta, from) => {
            let chance = |x: &[f32]| match from {
                None => mean_chance(nets, x),
                Some(f) => mean_reach(nets, x, f),
            };
            logit(chance(x)) - logit(chance(&at_white(x)))
        }
    }
}

/// A chance moved by a correction in log-odds, `scale` times; exactly as it was for none.
fn corrected(p: f64, c: f64, scale: f64) -> f64 {
    if c == 0.0 || scale == 0.0 {
        return p;
    }
    1.0 / (1.0 + (-(logit(p) + scale * c)).exp())
}

/// Teacher option `stake-scale=S`: the stake correction counts S times (1 unless set).
fn stake_scale() -> f64 {
    crate::teacher::param("stake-scale").unwrap_or(1.0)
}

/// The loaded value networks' log-odds of winning for an input, as the teacher uses them (the ensemble's mean
/// chance), without a stake correction: what a residual correction learns on top of (`baseline --offsets`).
pub fn base_logit(x: &[f32]) -> Option<f32> {
    let nets = NET.get()?;
    Some(logit(mean_chance(nets, x)) as f32)
}

/// The chance of winning by a given network.
pub fn chance_with(net: &Net, run: &Run) -> f64 {
    INPUT.with(|x| {
        let mut x = x.borrow_mut();
        encode_value(&screen(run), &mut x);
        chance_of(net, &x)
    })
}

/// What the loaded network adds to a position's value: the weighted log of its chance of winning (with teacher
/// option `reach=K`, of getting K blinds further) (None if no network is loaded, or the position is not one it
/// learned).
pub fn term(run: &Run) -> Option<f64> {
    // Positions past ante 8 (a won run playing on, `baseline --endless`) are not ones the networks learned.
    if !values_here(run) || run.ante > 8 {
        return None;
    }
    let p = chance(run, reach())?;
    Some(weight() * (p + FLOOR).ln())
}

// ---- Training data ----

/// IEEE half precision, rounded to nearest (ties to even): how the recorded inputs are stored.
pub fn f16_bits(x: f32) -> u16 {
    let b = x.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xFF) as i32;
    let man = b & 0x7F_FFFF;
    if exp == 0xFF {
        return sign | 0x7C00 | if man != 0 { 0x200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 0x1F {
        return sign | 0x7C00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        let m = man | 0x80_0000;
        let shift = (14 - e) as u32;
        let rounded = (m + (1 << (shift - 1)) - 1 + ((m >> shift) & 1)) >> shift;
        return sign | rounded as u16;
    }
    let h = ((e as u32) << 10) | (man >> 13);
    let rest = man & 0x1FFF;
    let up = rest > 0x1000 || (rest == 0x1000 && h & 1 == 1);
    sign | (h + u32::from(up)) as u16
}

/// The float a half-precision value stands for.
pub fn f16_value(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = i32::from((h >> 10) & 0x1F);
    let man = f32::from(h & 0x3FF);
    sign * match exp {
        0 => man * 2f32.powi(-24),
        0x1F if man == 0.0 => f32::INFINITY,
        0x1F => f32::NAN,
        _ => (1.0 + man / 1024.0) * 2f32.powi(exp - 15),
    }
}

/// Rows of value inputs, kept sparse: each row's nonzero floats as (index, half-precision value).
#[derive(Clone, Debug)]
pub struct Rows {
    pub idx: Vec<u16>,
    pub val: Vec<u16>,
    /// Where each row starts in `idx`/`val`, then the end: one more entry than rows.
    pub ptr: Vec<i64>,
}

impl Default for Rows {
    fn default() -> Rows {
        Rows {
            idx: Vec::new(),
            val: Vec::new(),
            ptr: vec![0],
        }
    }
}

impl Rows {
    pub fn len(&self) -> usize {
        self.ptr.len() - 1
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Add a row (its zeros, and values too small for half precision, are left out).
    pub fn push(&mut self, x: &[f32]) {
        for (i, &v) in x.iter().enumerate() {
            if v != 0.0 {
                let h = f16_bits(v);
                if h & 0x7FFF != 0 {
                    self.idx.push(i as u16);
                    self.val.push(h);
                }
            }
        }
        self.ptr.push(self.idx.len() as i64);
    }

    /// Add another set's rows after these.
    pub fn extend(&mut self, o: &Rows) {
        let base = *self.ptr.last().unwrap_or(&0);
        self.idx.extend_from_slice(&o.idx);
        self.val.extend_from_slice(&o.val);
        self.ptr.extend(o.ptr[1..].iter().map(|p| p + base));
    }

    /// Row `r` as dense floats.
    pub fn dense(&self, r: usize, len: usize) -> Vec<f32> {
        let mut x = vec![0.0; len];
        for k in self.ptr[r] as usize..self.ptr[r + 1] as usize {
            x[self.idx[k] as usize] = f16_value(self.val[k]);
        }
        x
    }
}

/// A `.npy` file (format 1.0, read by `numpy.load`): `descr` is numpy's type string (`<u2`, `<f2`, `<i8`,
/// `<f4`), `shape` the array's shape (C order), `data` its little-endian bytes.
pub fn npy_bytes(descr: &str, shape: &[usize], data: &[u8]) -> Vec<u8> {
    let dims: Vec<String> = shape.iter().map(|d| d.to_string()).collect();
    let shape_text = if dims.len() == 1 {
        format!("({},)", dims[0])
    } else {
        format!("({})", dims.join(", "))
    };
    let mut header =
        format!("{{'descr': '{descr}', 'fortran_order': False, 'shape': {shape_text}, }}");
    // Magic, version and length (10 bytes), the header and its newline: padded to a multiple of 64.
    let pad = (64 - (10 + header.len() + 1) % 64) % 64;
    header.push_str(&" ".repeat(pad));
    header.push('\n');
    let mut out = Vec::with_capacity(10 + header.len() + data.len());
    out.extend_from_slice(b"\x93NUMPY\x01\x00");
    out.extend_from_slice(&(header.len() as u16).to_le_bytes());
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(data);
    out
}

/// Little-endian bytes of a slice of numbers.
pub fn le_bytes<T: Copy, const N: usize>(v: &[T], f: impl Fn(T) -> [u8; N]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * N);
    for &x in v {
        out.extend_from_slice(&f(x));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A layer for a test file: inputs, outputs, ReLU, weights (input-major), biases.
    type Spec = (usize, usize, bool, Vec<f32>, Vec<f32>);

    /// A weights file for the given layers.
    fn file(layers: &[Spec]) -> Vec<u8> {
        file_as(VALUE_INPUT_VERSION, VALUE_LEN, layers)
    }

    /// A weights file that says it reads input layout `layout` (`inputs` floats).
    fn file_as(layout: u32, inputs: usize, layers: &[Spec]) -> Vec<u8> {
        let mut b = b"BMVN".to_vec();
        for v in [1u32, layout, inputs as u32, layers.len() as u32] {
            b.extend(v.to_le_bytes());
        }
        for (i, o, relu, w, bias) in layers {
            for v in [*i as u32, *o as u32, u32::from(*relu)] {
                b.extend(v.to_le_bytes());
            }
            for f in w.iter().chain(bias) {
                b.extend(f.to_le_bytes());
            }
        }
        b
    }

    #[test]
    fn a_network_reads_its_file_and_computes() {
        // Input 0 and input 5 feed two hidden units; the output adds them.
        let n = VALUE_LEN;
        let mut w1 = vec![0.0f32; n * 2];
        w1[0] = 1.0; // input 0 -> hidden 0
        w1[5 * 2 + 1] = -2.0; // input 5 -> hidden 1
        let net = Net::from_bytes(&file(&[
            (n, 2, true, w1, vec![0.5, 0.0]),
            (2, 1, false, vec![1.0, 1.0], vec![0.25]),
        ]))
        .unwrap();
        let mut x = vec![0.0f32; n];
        x[0] = 2.0;
        x[5] = -1.0;
        // hidden = relu(2 + 0.5, 2) = (2.5, 2); output = 4.5 + 0.25
        assert_eq!(net.forward(&x), vec![4.75]);
        x[5] = 1.0; // hidden 1 = relu(-2) = 0
        assert_eq!(net.forward(&x), vec![2.75]);
    }

    #[test]
    fn a_wrong_file_is_refused() {
        assert!(Net::from_bytes(b"NOPE").is_err());
        let n = VALUE_LEN;
        let good = file(&[(n, 1, false, vec![0.0; n], vec![0.0])]);
        assert!(Net::from_bytes(&good).is_ok());
        let mut short = good.clone();
        short.pop();
        assert!(Net::from_bytes(&short).is_err());
        // Another input layout.
        let mut other = good.clone();
        other[8..12].copy_from_slice(&(VALUE_INPUT_VERSION + 1).to_le_bytes());
        let e = Net::from_bytes(&other).unwrap_err();
        assert!(e.contains("retrain"), "{e}");
        // Layers that don't fit together.
        assert!(Net::from_bytes(&file(&[
            (n, 2, true, vec![0.0; n * 2], vec![0.0; 2]),
            (3, 1, false, vec![0.0; 3], vec![0.0])
        ]))
        .is_err());
    }

    #[test]
    fn a_network_of_an_older_layout_reads_the_start_of_the_input_as_it_was_trained() {
        use crate::observe::VALUE_LEN_1;
        // Weights on every input it has (the last of layout 1's included): fed this engine's longer input, it
        // gives what it gives on layout 1's floats alone, whatever the floats after them hold.
        let n1 = VALUE_LEN_1;
        let w: Vec<f32> = (0..n1 * 3)
            .map(|j| ((j * 37) % 11) as f32 / 10.0 - 0.5)
            .collect();
        let layers = [
            (n1, 3, true, w, vec![0.1, -0.2, 0.3]),
            (
                3,
                2,
                false,
                vec![1.0, 0.5, -1.0, 0.25, 2.0, -0.75],
                vec![0.05, 0.0],
            ),
        ];
        let old = Net::from_bytes(&file_as(1, n1, &layers)).unwrap();
        let mut x: Vec<f32> = (0..VALUE_LEN)
            .map(|i| {
                if i % 9 == 0 {
                    (i % 7) as f32 - 3.0
                } else {
                    0.0
                }
            })
            .collect();
        x[n1 - 1] = 1.5;
        let alone = old.forward(&x[..n1]);
        assert_eq!(old.forward(&x), alone);
        x[n1..].iter_mut().for_each(|v| *v = 4.0);
        assert_eq!(
            old.forward(&x),
            alone,
            "the floats after its layout's are not read"
        );
        assert_eq!(old.first_output(&x).to_bits(), alone[0].to_bits());
        // In an ensemble with a network of this engine's layout: each reads its own, the mean as usual.
        let new = Net::from_bytes(&file(&[(
            VALUE_LEN,
            1,
            false,
            vec![0.01; VALUE_LEN],
            vec![0.0],
        )]))
        .unwrap();
        let sig = |z: f32| 1.0 / (1.0 + (-(z as f64)).exp());
        let want = (sig(alone[0]) + sig(new.forward(&x)[0])) / 2.0;
        assert_eq!(mean_chance(&[old, new], &x).to_bits(), want.to_bits());
        // A length that is no layout's, or a layout with another length, is refused.
        let one = |inputs: usize| vec![(inputs, 1, false, vec![0.0; inputs], vec![0.0])];
        assert!(Net::from_bytes(&file_as(1, VALUE_LEN, &one(VALUE_LEN))).is_err());
        assert!(Net::from_bytes(&file_as(VALUE_INPUT_VERSION, n1, &one(n1))).is_err());
        assert!(Net::from_bytes(&file_as(1, n1 + 1, &one(n1 + 1))).is_err());
    }

    #[test]
    fn an_ensemble_gives_the_mean_of_its_networks_chances() {
        let n = VALUE_LEN;
        let bias =
            |b: f32| Net::from_bytes(&file(&[(n, 1, false, vec![0.0; n], vec![b])])).unwrap();
        let x = vec![0.0f32; n];
        let (a, b) = (bias(0.0), bias(2.0f32.ln()));
        // Chances 1/2 and 2/3.
        assert!((mean_chance(&[a.clone(), b.clone()], &x) - (0.5 + 2.0 / 3.0) / 2.0).abs() < 1e-6);
        assert!((mean_chance(&[b], &x) - 2.0 / 3.0).abs() < 1e-6);
        assert!((mean_chance(&[a], &x) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn the_depth_outputs_say_how_far_a_win_goes_on_and_leave_the_stages_alone() {
        let n = VALUE_LEN;
        // One layer: the win logit 0, the 25 stage logits all 0 but "won" (24) at ln 3, then 13 depth logits: depths
        // 0 and 1 at ln 2, the rest at 0 (so depth 0: 2/15, 1: 2/15, each other: 1/15).
        let mut bias = vec![0.0f32; 1 + STAGES + 13];
        bias[1 + 24] = 3f32.ln();
        bias[1 + STAGES] = 2f32.ln();
        bias[1 + STAGES + 1] = 2f32.ln();
        let outs = bias.len();
        let net = Net::from_bytes(&file(&[(n, outs, false, vec![0.0; n * outs], bias)])).unwrap();
        assert_eq!(net.outputs(), 1 + STAGES + 13);
        let x = vec![0.0f32; n];
        let out = net.forward(&x);
        // Stages: 24 logits at 0 and "won" at ln 3 → reaching the win: 3/27; the depth logits don't count.
        assert!((reach_of(&out, 24) - 3.0 / 27.0).abs() < 1e-6);
        assert!((depth_tail(&out, 0).unwrap() - 1.0).abs() < 1e-9);
        assert!((depth_tail(&out, 1).unwrap() - 13.0 / 15.0).abs() < 1e-6);
        assert!((depth_tail(&out, 2).unwrap() - 11.0 / 15.0).abs() < 1e-6);
        // A network without them: none.
        let old =
            Net::from_bytes(&file(&[(n, 26, false, vec![0.0; n * 26], vec![0.0; 26])])).unwrap();
        assert_eq!(depth_tail(&old.forward(&x), 1), None);
        assert_eq!(mean_depth_tail(std::slice::from_ref(&old), &x, 1), None);
        let mixed = mean_depth_tail(&[old, net], &x, 2).unwrap();
        assert!(
            (mixed - 11.0 / 15.0).abs() < 1e-6,
            "only the networks that have them"
        );
        assert_eq!(margin(), None, "off unless asked");
        assert_eq!(crate::teacher::with_options(&["margin=3"], margin), Some(3));
    }

    #[test]
    fn doubt_and_hope_move_the_chance_by_the_networks_disagreement() {
        let n = VALUE_LEN;
        let bias =
            |b: f32| Net::from_bytes(&file(&[(n, 1, false, vec![0.0; n], vec![b])])).unwrap();
        let x = vec![0.0f32; n];
        // Chances 1/2 and 2/3: mean 7/12, spread 1/12.
        let nets = [bias(0.0), bias(2.0f32.ln())];
        let (mean, spread) = mean_and_spread(&nets, &x);
        assert!((mean - 7.0 / 12.0).abs() < 1e-6 && (spread - 1.0 / 12.0).abs() < 1e-6);
        assert_eq!(
            mean.to_bits(),
            mean_chance(&nets, &x).to_bits(),
            "the same mean as before"
        );
        assert_eq!(disagreement_lean(), 0.0, "off unless asked");
        let lean = |opts: &[&str]| crate::teacher::with_options(opts, disagreement_lean);
        assert_eq!(lean(&["doubt=1"]), -1.0);
        assert_eq!(lean(&["hope=0.5"]), 0.5);
        assert_eq!(lean(&["doubt=2", "hope=0.5"]), -1.5);
        let (m, s) = mean_and_spread(&nets[..1], &x);
        assert!(
            (m - 0.5).abs() < 1e-12 && s == 0.0,
            "one network: no disagreement"
        );
    }

    /// A one-layer network of this engine's layout: weights on chosen inputs, the rest 0, and a bias (one
    /// output: the win logit).
    fn linear(on: &[(usize, f32)], bias: f32) -> Net {
        let mut w = vec![0.0; VALUE_LEN];
        for &(i, v) in on {
            w[i] = v;
        }
        Net::from_bytes(&file(&[(VALUE_LEN, 1, false, w, vec![bias])])).unwrap()
    }

    /// An input at a stake (its rule floats as `encode_value` sets them) with a few other floats on.
    fn input_at(stake: u8) -> Vec<f32> {
        let mut x = vec![0.0; VALUE_LEN];
        x[3] = 1.0;
        x[40] = 0.5;
        for rule in 2..=8u8 {
            x[VALUE_LEN - STAKE_PART + usize::from(rule) - 2] = f32::from(stake >= rule);
        }
        x
    }

    #[test]
    fn a_residual_correction_adds_its_networks_log_odds() {
        let x = input_at(3);
        let (a, b) = (linear(&[(3, 0.4)], -0.1), linear(&[(40, 1.0)], 0.2));
        // Outputs 0.3 and 0.7: the mean, 0.5, is added to the value networks' log-odds.
        let c = correction(&[a.clone(), b], StakeMode::Residual, &x, None);
        assert!((c - 0.5).abs() < 1e-6, "{c}");
        let p = 0.25f64;
        let want = 1.0 / (1.0 + (-((p / (1.0 - p)).ln() + 0.5)).exp());
        assert!((corrected(p, c, 1.0) - want).abs() < 1e-6);
        // Twice the scale, twice the shift; none, no change at all.
        let twice = 1.0 / (1.0 + (-((p / (1.0 - p)).ln() + 1.0)).exp());
        assert!((corrected(p, c, 2.0) - twice).abs() < 1e-6);
        assert_eq!(corrected(p, c, 0.0).to_bits(), p.to_bits());
        // A residual network of output 0 leaves the chance bit for bit as it was.
        let zero = linear(&[], 0.0);
        let c0 = correction(&[zero], StakeMode::Residual, &x, None);
        assert_eq!(c0, 0.0);
        assert_eq!(corrected(0.3712, c0, 1.0).to_bits(), 0.3712f64.to_bits());
        // It corrects the win chance only: with teacher option reach it adds nothing.
        assert_eq!(correction(&[a], StakeMode::Residual, &x, Some(9)), 0.0);
    }

    #[test]
    fn a_delta_correction_is_what_the_stake_changes_and_nothing_at_white() {
        let first = VALUE_LEN - STAKE_PART;
        // The network's log-odds: 0.8 for the position, -0.6 for Red's rule, -1.1 for Green's.
        let net = linear(&[(3, 0.8), (first, -0.6), (first + 1, -1.1)], 0.1);
        let green = input_at(3);
        let c = correction(std::slice::from_ref(&net), StakeMode::Delta, &green, None);
        assert!((c - (-1.7)).abs() < 1e-5, "{c}");
        // Its judgement of the position (0.8, the bias) cancels: the same delta for another position.
        let mut other = green.clone();
        other[3] = 0.0;
        let c2 = correction(std::slice::from_ref(&net), StakeMode::Delta, &other, None);
        assert!((c2 - c).abs() < 1e-5);
        // At White: nothing, exactly.
        let white = input_at(1);
        assert!(!shows_a_stake(&white) && shows_a_stake(&green));
        assert_eq!(correction(&[net], StakeMode::Delta, &white, None), 0.0);
        // The White view turns the stake's floats off and nothing else.
        let w = at_white(&green);
        assert_eq!(w[..first], green[..first]);
        assert!(w[first..].iter().all(|&v| v == 0.0));
    }

    #[test]
    fn a_stake_correction_must_see_the_stake() {
        use crate::observe::VALUE_LEN_1;
        let old = Net::from_bytes(&file_as(
            1,
            VALUE_LEN_1,
            &[(VALUE_LEN_1, 1, false, vec![0.0; VALUE_LEN_1], vec![0.0])],
        ))
        .unwrap();
        assert!(check_stake_nets(std::slice::from_ref(&old)).is_err());
        assert!(check_stake_nets(&[linear(&[], 0.0)]).is_ok());
        assert_eq!(StakeMode::parse("residual"), Ok(StakeMode::Residual));
        assert_eq!(
            StakeMode::parse(StakeMode::Delta.name()),
            Ok(StakeMode::Delta)
        );
        assert!(StakeMode::parse("both").is_err());
    }

    #[test]
    fn the_reach_chance_is_the_share_of_endings_from_a_stage_on() {
        // 25 equal ending logits: each ending 1 in 25.
        let out = vec![0.3f32; 26];
        assert!((reach_of(&out, 0) - 1.0).abs() < 1e-12);
        assert!((reach_of(&out, 20) - 5.0 / 25.0).abs() < 1e-12);
        // Further than the last blind: a win.
        assert!((reach_of(&out, 40) - 1.0 / 25.0).abs() < 1e-12);
        // A network with the win logit alone gives its win chance.
        assert!((reach_of(&[0.0], 5) - 0.5).abs() < 1e-12);
        let mut skewed = vec![0.0f32; 26];
        skewed[1 + 24] = 3.0f32.ln();
        assert!((reach_of(&skewed, 24) - 3.0 / 27.0).abs() < 1e-6);
    }

    #[test]
    fn win_only_inference_is_bit_identical_across_shapes_and_reused_buffers() {
        let mut nets = Vec::new();
        for widths in [vec![26], vec![64, 26], vec![128, 64, 26], vec![3, 7, 1]] {
            for last_relu in [false, true] {
                let mut inputs = VALUE_LEN;
                let mut specs = Vec::new();
                for (i, &outputs) in widths.iter().enumerate() {
                    let weights = (0..inputs * outputs)
                        .map(|j| ((j * 37 + i * 17) % 101) as f32 / 500.0 - 0.1)
                        .collect();
                    let bias = (0..outputs).map(|j| j as f32 / 100.0 - 0.03).collect();
                    specs.push((
                        inputs,
                        outputs,
                        i + 1 < widths.len() || last_relu,
                        weights,
                        bias,
                    ));
                    inputs = outputs;
                }
                nets.push(Net::from_bytes(&file(&specs)).unwrap());
            }
        }
        for stride in [1, 7, 31, VALUE_LEN + 1] {
            let x: Vec<f32> = (0..VALUE_LEN)
                .map(|i| {
                    if i % stride == 0 {
                        (i % 23) as f32 / 7.0 - 1.0
                    } else {
                        0.0
                    }
                })
                .collect();
            for net in &nets {
                assert_eq!(net.first_output(&x).to_bits(), net.forward(&x)[0].to_bits());
                let mut w = Inference::default();
                w.read(&x);
                let mut out = Vec::new();
                net.all_with(&mut w, &mut out);
                let bits = |v: &[f32]| v.iter().map(|o| o.to_bits()).collect::<Vec<_>>();
                assert_eq!(bits(&out), bits(&net.forward(&x)));
            }
            let full_mean = nets
                .iter()
                .map(|n| 1.0 / (1.0 + (-(n.forward(&x)[0] as f64)).exp()))
                .sum::<f64>()
                / nets.len() as f64;
            assert_eq!(mean_chance(&nets, &x).to_bits(), full_mean.to_bits());
        }
        let zero = vec![0.0; VALUE_LEN];
        for net in nets.iter().rev() {
            assert_eq!(
                net.first_output(&zero).to_bits(),
                net.forward(&zero)[0].to_bits()
            );
        }
    }

    #[test]
    fn the_chance_comes_from_the_screen() {
        // A network whose output is a bias alone: the chance is its sigmoid, for any position.
        let n = VALUE_LEN;
        let net = Net::from_bytes(&file(&[(n, 1, false, vec![0.0; n], vec![0.0])])).unwrap();
        let run = Run::new(crate::RunConfig::new("LEARNED1"));
        assert!((chance_with(&net, &run) - 0.5).abs() < 1e-12);
        assert!(values_here(&run), "the blind choice is valued");
    }

    #[test]
    fn half_precision_rounds_like_numpy() {
        // Values and bits from numpy's float16.
        for (x, h) in [
            (0.0f32, 0x0000u16),
            (1.0, 0x3C00),
            (-2.0, 0xC000),
            (0.5, 0x3800),
            (0.1, 0x2E66),
            (1.0 / 3.0, 0x3555),
            (65504.0, 0x7BFF),
            (70000.0, 0x7C00),
            (1e-5, 0x00A8),
            (6.0e-8, 0x0001),
            (2.0e-8, 0x0000),
            (2049.0, 0x6800), // a tie, to even
            (2051.0, 0x6802), // a tie, to even (up)
            (46.0517, 0x51C2),
        ] {
            assert_eq!(f16_bits(x), h, "{x}");
        }
        for h in [0x3C00u16, 0x2E66, 0x00A8, 0x7BFF, 0xC000, 0x51C2] {
            assert_eq!(f16_bits(f16_value(h)), h);
        }
    }

    #[test]
    fn rows_keep_the_nonzero_floats() {
        let mut r = Rows::default();
        r.push(&[0.0, 1.0, 0.0, 0.25]);
        r.push(&[0.0; 4]);
        r.push(&[3.0, 0.0, 0.0, 0.0]);
        assert_eq!(r.len(), 3);
        assert_eq!(r.idx, vec![1, 3, 0]);
        assert_eq!(r.ptr, vec![0, 2, 2, 3]);
        assert_eq!(r.dense(0, 4), vec![0.0, 1.0, 0.0, 0.25]);
        assert_eq!(r.dense(2, 4), vec![3.0, 0.0, 0.0, 0.0]);
        let mut all = Rows::default();
        all.extend(&r);
        all.extend(&r);
        assert_eq!(all.len(), 6);
        assert_eq!(all.ptr, vec![0, 2, 2, 3, 5, 5, 6]);
        assert_eq!(all.dense(5, 4), vec![3.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn npy_files_are_what_numpy_writes() {
        // np.save(np.array([1, 2, 3], dtype='<u2')) writes these 134 bytes: magic, version 1.0, the
        // header's length, the header (the dictionary numpy reads, padded with spaces and a newline to a
        // multiple of 64 bytes), then the data.
        let b = npy_bytes("<u2", &[3], &le_bytes(&[1u16, 2, 3], u16::to_le_bytes));
        assert_eq!(&b[..8], b"\x93NUMPY\x01\x00");
        assert_eq!(u16::from_le_bytes([b[8], b[9]]), 118);
        assert_eq!(b.len(), 134);
        let header = std::str::from_utf8(&b[10..128]).unwrap();
        assert!(header.starts_with("{'descr': '<u2', 'fortran_order': False, 'shape': (3,), }"));
        assert!(header.ends_with(" \n"));
        assert_eq!(&b[128..], &[1, 0, 2, 0, 3, 0]);
        let m = npy_bytes("<f4", &[2, 5], &[0; 40]);
        assert_eq!(m.len(), 128 + 40);
        assert!(std::str::from_utf8(&m[10..128])
            .unwrap()
            .contains("'shape': (2, 5), }"));
    }
}
