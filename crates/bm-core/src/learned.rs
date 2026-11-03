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
//!   (u32, `VALUE_LEN`), layer count (u32);
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

use crate::observe::{encode_value, screen, VALUE_INPUT_VERSION, VALUE_LEN};
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
        if layout != VALUE_INPUT_VERSION || inputs != VALUE_LEN {
            return Err(format!(
                "the network reads input layout {layout} ({inputs} floats), this engine's is \
                 {VALUE_INPUT_VERSION} ({VALUE_LEN}): retrain it on data recorded by this engine"
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
        let mut out = last.bias[0];
        if hidden.is_empty() {
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
    let mut nets = Vec::new();
    for p in path.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let bytes = std::fs::read(p).map_err(|e| format!("{p}: {e}"))?;
        nets.push(Net::from_bytes(&bytes).map_err(|e| format!("{p}: {e}"))?);
    }
    if nets.is_empty() {
        return Err(format!("no value network in {path:?}"));
    }
    NET.set(nets)
        .map_err(|_| "a value network is already loaded".to_string())
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
    let nets = NET.get()?;
    Some(INPUT.with(|x| {
        let mut x = x.borrow_mut();
        encode_value(&screen(run), &mut x);
        let key = input_key(&x);
        if let Some(p) = SEEN.with(|s| s.borrow().get(&key).copied()) {
            return p;
        }
        let p = mean_chance(nets, &x);
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

/// The chance of winning by a given network.
pub fn chance_with(net: &Net, run: &Run) -> f64 {
    INPUT.with(|x| {
        let mut x = x.borrow_mut();
        encode_value(&screen(run), &mut x);
        chance_of(net, &x)
    })
}

/// What the loaded network adds to a position's value: the weighted log of its chance of winning (None if
/// no network is loaded, or the position is not one it learned).
pub fn term(run: &Run) -> Option<f64> {
    if !values_here(run) {
        return None;
    }
    let p = win_chance(run)?;
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
        let mut b = b"BMVN".to_vec();
        for v in [
            1u32,
            VALUE_INPUT_VERSION,
            VALUE_LEN as u32,
            layers.len() as u32,
        ] {
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
