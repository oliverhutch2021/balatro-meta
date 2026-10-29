//! Python bindings: `balatro_meta._engine`.
//!
//! The RNG (for checks), and `VecEnv`: many runs stepped in parallel on all
//! threads, with the bot's input written straight into numpy arrays.
//! `python/balatro_meta/env.py` wraps it.

use bm_core::env::{legal_kinds, random_move, Env, MoveKind, XorShift, N_MOVE_KINDS};
use bm_core::observe::{encode, screen, OBS_LEN};
use pyo3::buffer::PyBuffer;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use rayon::prelude::*;

/// The game's string hash.
#[pyfunction]
fn pseudohash(s: &str) -> f64 {
    bm_rng::pseudohash(s)
}

/// LuaJIT's `math.randomseed(d)` followed by `n` calls to `math.random()`.
#[pyfunction]
fn lua_random(d: f64, n: usize) -> Vec<f64> {
    let mut r = bm_rng::LuaRandom::new(d);
    (0..n).map(|_| r.random()).collect()
}

/// A run's named random streams.
#[pyclass(module = "balatro_meta._engine")]
struct Streams {
    inner: bm_rng::Streams,
}

#[pymethods]
impl Streams {
    #[new]
    fn new(seed: &str) -> Self {
        Streams {
            inner: bm_rng::Streams::new(seed),
        }
    }

    #[getter]
    fn seed(&self) -> String {
        self.inner.seed().to_string()
    }

    fn pseudoseed(&mut self, key: &str) -> f64 {
        self.inner.pseudoseed(key)
    }

    fn random(&mut self, key: &str) -> f64 {
        self.inner.random(key)
    }

    fn random_range(&mut self, key: &str, lo: i64, hi: i64) -> i64 {
        self.inner.random_range(key, lo, hi)
    }
}

fn buffer<T: pyo3::buffer::Element>(
    obj: &Bound<'_, PyAny>,
    n: usize,
    what: &str,
) -> PyResult<PyBuffer<T>> {
    let b = PyBuffer::<T>::get(obj)?;
    if b.item_count() != n || !b.is_c_contiguous() {
        return Err(PyValueError::new_err(format!(
            "{what}: expected a contiguous array of {n} items"
        )));
    }
    Ok(b)
}

/// Many runs at once. Every run gets fresh seeds from `seed` and restarts by
/// itself when it ends.
#[pyclass(module = "balatro_meta._engine")]
struct VecEnv {
    envs: Vec<Env>,
    rngs: Vec<XorShift>,
    obs: Vec<f32>,
    pool: rayon::ThreadPool,
}

#[pymethods]
impl VecEnv {
    #[new]
    #[pyo3(signature = (n, seed = 0, stake = 1, threads = 0))]
    fn new(n: usize, seed: u64, stake: u8, threads: usize) -> PyResult<Self> {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        let envs = (0..n)
            .map(|i| Env::new(seed.wrapping_mul(1_000_003).wrapping_add(i as u64), stake))
            .collect();
        let rngs = (0..n)
            .map(|i| XorShift(0x9E37_79B9_7F4A_7C15 ^ (seed << 20) ^ (i as u64 + 1)))
            .collect();
        Ok(VecEnv {
            envs,
            rngs,
            obs: vec![0.0; n * OBS_LEN],
            pool,
        })
    }

    #[getter]
    fn n(&self) -> usize {
        self.envs.len()
    }

    /// Write every run's input into `out` (float32, n x OBS_LEN).
    fn obs_into(&mut self, py: Python<'_>, out: &Bound<'_, PyAny>) -> PyResult<()> {
        let buf = buffer::<f32>(out, self.envs.len() * OBS_LEN, "out")?;
        let (envs, obs, pool) = (&self.envs, &mut self.obs, &self.pool);
        py.detach(|| {
            pool.install(|| {
                obs.par_chunks_mut(OBS_LEN)
                    .zip(envs.par_iter())
                    .for_each(|(o, e)| {
                        let mut v = Vec::with_capacity(OBS_LEN);
                        encode(&screen(&e.run), &mut v);
                        o.copy_from_slice(&v);
                    })
            })
        });
        buf.copy_from_slice(py, &self.obs)
    }

    /// Apply one move per run: `kinds` (int64) and `args` (uint64). Writes
    /// reward (float32), done, won and refused (uint8) and the ante (int32).
    #[allow(clippy::too_many_arguments)]
    fn step(
        &mut self,
        py: Python<'_>,
        kinds: &Bound<'_, PyAny>,
        args: &Bound<'_, PyAny>,
        reward: &Bound<'_, PyAny>,
        done: &Bound<'_, PyAny>,
        won: &Bound<'_, PyAny>,
        refused: &Bound<'_, PyAny>,
        ante: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let n = self.envs.len();
        let k: Vec<i64> = buffer::<i64>(kinds, n, "kinds")?.to_vec(py)?;
        let a: Vec<u64> = buffer::<u64>(args, n, "args")?.to_vec(py)?;
        let (envs, pool) = (&mut self.envs, &self.pool);
        let infos: Vec<_> = py.detach(|| {
            pool.install(|| {
                envs.par_iter_mut()
                    .enumerate()
                    .map(|(i, e)| e.step(k[i], a[i]))
                    .collect()
            })
        });
        buffer::<f32>(reward, n, "reward")?
            .copy_from_slice(py, &infos.iter().map(|x| x.reward).collect::<Vec<_>>())?;
        buffer::<u8>(done, n, "done")?
            .copy_from_slice(py, &infos.iter().map(|x| x.done as u8).collect::<Vec<_>>())?;
        buffer::<u8>(won, n, "won")?
            .copy_from_slice(py, &infos.iter().map(|x| x.won as u8).collect::<Vec<_>>())?;
        buffer::<u8>(refused, n, "refused")?.copy_from_slice(
            py,
            &infos.iter().map(|x| x.refused as u8).collect::<Vec<_>>(),
        )?;
        buffer::<i32>(ante, n, "ante")?
            .copy_from_slice(py, &infos.iter().map(|x| x.ante).collect::<Vec<_>>())?;
        Ok(())
    }

    /// Which move kinds each run allows now (uint8, n x N_MOVE_KINDS).
    fn legal_kinds_into(&self, py: Python<'_>, out: &Bound<'_, PyAny>) -> PyResult<()> {
        let n = self.envs.len();
        let buf = buffer::<u8>(out, n * N_MOVE_KINDS, "out")?;
        let v: Vec<u8> = self
            .envs
            .iter()
            .flat_map(|e| legal_kinds(&e.run).map(u8::from))
            .collect();
        buf.copy_from_slice(py, &v)
    }

    /// A random legal move for every run (for baselines and benchmarks).
    fn random_moves_into(
        &mut self,
        py: Python<'_>,
        kinds: &Bound<'_, PyAny>,
        args: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let n = self.envs.len();
        let moves: Vec<(MoveKind, u64)> = self
            .envs
            .iter()
            .zip(self.rngs.iter_mut())
            .map(|(e, r)| random_move(&e.run, r))
            .collect();
        buffer::<i64>(kinds, n, "kinds")?
            .copy_from_slice(py, &moves.iter().map(|m| m.0 as i64).collect::<Vec<_>>())?;
        buffer::<u64>(args, n, "args")?
            .copy_from_slice(py, &moves.iter().map(|m| m.1).collect::<Vec<_>>())
    }

    /// Hand sizes now (int32, n).
    fn hand_len_into(&self, py: Python<'_>, out: &Bound<'_, PyAny>) -> PyResult<()> {
        let n = self.envs.len();
        let v: Vec<i32> = self.envs.iter().map(|e| e.run.hand.len() as i32).collect();
        buffer::<i32>(out, n, "out")?.copy_from_slice(py, &v)
    }

    /// What run `i` shows on screen, as a dict (for viewers and debugging).
    fn screen<'py>(&self, py: Python<'py>, i: usize) -> PyResult<Bound<'py, PyDict>> {
        let e = self
            .envs
            .get(i)
            .ok_or_else(|| PyValueError::new_err("no such run"))?;
        let s = screen(&e.run);
        let d = PyDict::new(py);
        d.set_item("phase", format!("{:?}", s.phase))?;
        d.set_item("ante", s.ante)?;
        d.set_item("round", s.round)?;
        d.set_item("money", s.money)?;
        d.set_item("hands_left", s.hands_left)?;
        d.set_item("discards_left", s.discards_left)?;
        d.set_item("round_chips", s.round_chips)?;
        d.set_item("blind", s.blind.map(|b| (format!("{:?}", b.0), b.1)))?;
        d.set_item("boss", s.boss.name())?;
        d.set_item("tags_offered", s.tags_offered.map(|t| t.name()).to_vec())?;
        let hand: Vec<String> = e
            .run
            .hand
            .iter()
            .map(|&id| {
                let c = e.run.card(id);
                if c.face_down {
                    "??".to_string()
                } else {
                    c.to_string()
                }
            })
            .collect();
        d.set_item("hand", hand)?;
        let jokers: Vec<String> = s
            .jokers
            .iter()
            .map(|j| j.kind.map_or("??", |k| k.name()).to_string())
            .collect();
        d.set_item("jokers", jokers)?;
        d.set_item("deck_left", s.deck_left)?;
        Ok(d)
    }
}

#[pymodule]
fn _engine(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add("OBS_LEN", OBS_LEN)?;
    m.add("N_MOVE_KINDS", N_MOVE_KINDS)?;
    m.add_function(wrap_pyfunction!(pseudohash, m)?)?;
    m.add_function(wrap_pyfunction!(lua_random, m)?)?;
    m.add_class::<Streams>()?;
    m.add_class::<VecEnv>()?;
    Ok(())
}
