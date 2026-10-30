//! Python bindings: `balatro_meta._engine`.
//!
//! The RNG (for checks), and `VecEnv`: many runs stepped in parallel on all
//! threads, with the bot's input written straight into numpy arrays.
//! `python/balatro_meta/env.py` wraps it.

use bm_core::blinds::{BlindSlot, SlotState};
use bm_core::card::{Edition, Enhancement, Seal};
use bm_core::content::BOOSTERS;
use bm_core::env::{apply, legal_kinds, random_move, Env, MoveKind, XorShift, N_MOVE_KINDS};
use bm_core::observe::{
    encode, encode_value, obs_layout, screen, OBS_LEN, VALUE_INPUT_VERSION, VALUE_LEN,
};
use bm_core::packs::PackCard;
use bm_core::shop::RowItem;
use bm_core::state::{Phase, Run, RunConfig};
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

/// A value network's outputs (a BMVN file's bytes, `bm_core::learned`) for one input of `VALUE_LEN`
/// floats: the engine's own forward pass, to check the trainer's export against.
#[pyfunction]
fn value_net_forward(data: &[u8], x: Vec<f32>) -> PyResult<Vec<f32>> {
    let net = bm_core::learned::Net::from_bytes(data).map_err(PyValueError::new_err)?;
    if x.len() != VALUE_LEN {
        return Err(PyValueError::new_err(format!(
            "the input has {} floats, not {VALUE_LEN}",
            x.len()
        )));
    }
    Ok(net.forward(&x))
}

/// The seed of run `i` of a job with seed base `base` (the baseline job's runs).
#[pyfunction]
fn run_seed(base: u64, i: u64) -> String {
    bm_core::env::run_seed(base, i)
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

    /// Write every run's input into `out` (float32, n x OBS_LEN, C-contiguous,
    /// writable). The runs are encoded in parallel straight into the array;
    /// don't touch the array from another thread during the call.
    fn obs_into(&mut self, py: Python<'_>, out: &Bound<'_, PyAny>) -> PyResult<()> {
        let n = self.envs.len();
        let buf = buffer::<f32>(out, n * OBS_LEN, "out")?;
        if buf.readonly() {
            return Err(PyValueError::new_err("out: the array must be writable"));
        }
        let ptr = buf.buf_ptr() as usize;
        let (envs, pool) = (&self.envs, &self.pool);
        py.detach(|| {
            pool.install(|| {
                // SAFETY: `buf` keeps the array alive and checked its size, layout and
                // writability; each run writes only its own chunk.
                let dst = unsafe { std::slice::from_raw_parts_mut(ptr as *mut f32, n * OBS_LEN) };
                dst.par_chunks_mut(OBS_LEN)
                    .zip(envs.par_iter())
                    .for_each_init(
                        || Vec::with_capacity(OBS_LEN),
                        |v, (o, e)| {
                            encode(&screen(&e.run), v);
                            o.copy_from_slice(v);
                        },
                    )
            })
        });
        drop(buf);
        Ok(())
    }

    /// The same through an intermediate copy (a slower reference for tests).
    fn obs_into_copy(&mut self, py: Python<'_>, out: &Bound<'_, PyAny>) -> PyResult<()> {
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
    /// reward (float32), done, won and refused (uint8) and the ante (int32),
    /// and, if given, whether the move beat a blind (`cleared`, uint8).
    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature = (kinds, args, reward, done, won, refused, ante, cleared = None))]
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
        cleared: Option<&Bound<'_, PyAny>>,
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
        if let Some(c) = cleared {
            buffer::<u8>(c, n, "cleared")?.copy_from_slice(
                py,
                &infos.iter().map(|x| x.cleared as u8).collect::<Vec<_>>(),
            )?;
        }
        Ok(())
    }

    /// Start run `i` again on this seed (to play chosen seeds, such as another bot's).
    fn set_seed(&mut self, i: usize, seed: &str) -> PyResult<()> {
        let s = bm_rng::normalize_seed(seed)
            .ok_or_else(|| PyValueError::new_err("not a valid seed"))?;
        self.envs
            .get_mut(i)
            .ok_or_else(|| PyValueError::new_err("no such run"))?
            .start(&s);
        Ok(())
    }

    /// Each run's seed (to replay a run with `Game`).
    fn seeds(&self) -> Vec<String> {
        self.envs.iter().map(|e| e.run.cfg.seed.clone()).collect()
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
        let (envs, rngs, pool) = (&self.envs, &mut self.rngs, &self.pool);
        let moves: Vec<(MoveKind, u64)> = py.detach(|| {
            pool.install(|| {
                envs.par_iter()
                    .zip(rngs.par_iter_mut())
                    .map(|(e, r)| random_move(&e.run, r))
                    .collect()
            })
        });
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

fn enh_name(e: Enhancement) -> Option<&'static str> {
    Some(match e {
        Enhancement::None => return None,
        Enhancement::Bonus => "BONUS",
        Enhancement::Mult => "MULT",
        Enhancement::Wild => "WILD",
        Enhancement::Glass => "GLASS",
        Enhancement::Steel => "STEEL",
        Enhancement::Stone => "STONE",
        Enhancement::Gold => "GOLD",
        Enhancement::Lucky => "LUCKY",
    })
}

fn edition_name(e: Edition) -> Option<&'static str> {
    Some(match e {
        Edition::None => return None,
        Edition::Foil => "FOIL",
        Edition::Holographic => "HOLO",
        Edition::Polychrome => "POLYCHROME",
        Edition::Negative => "NEGATIVE",
    })
}

fn seal_name(s: Seal) -> Option<&'static str> {
    Some(match s {
        Seal::None => return None,
        Seal::Gold => "GOLD",
        Seal::Red => "RED",
        Seal::Blue => "BLUE",
        Seal::Purple => "PURPLE",
    })
}

fn state_name(p: Phase) -> &'static str {
    match p {
        Phase::BlindSelect => "BLIND_SELECT",
        Phase::Round => "SELECTING_HAND",
        Phase::RoundEval => "ROUND_EVAL",
        Phase::Shop => "SHOP",
        Phase::Pack => "SMODS_BOOSTER_OPENED",
        Phase::GameOver => "GAME_OVER",
    }
}

fn slot_name(s: SlotState) -> &'static str {
    match s {
        SlotState::Upcoming => "UPCOMING",
        SlotState::Select => "SELECT",
        SlotState::Current => "CURRENT",
        SlotState::Defeated => "DEFEATED",
        SlotState::Skipped => "SKIPPED",
    }
}

/// One run, for replays and viewers: every move is recorded, and `view()`
/// gives the full state in the same terms the real game reports.
#[pyclass(module = "balatro_meta._engine")]
struct Game {
    run: Run,
    moves: Vec<(i64, u64)>,
}

#[pymethods]
impl Game {
    #[new]
    #[pyo3(signature = (seed, stake = 1))]
    fn new(seed: &str, stake: u8) -> PyResult<Self> {
        let s = bm_rng::normalize_seed(seed)
            .ok_or_else(|| PyValueError::new_err("not a valid seed"))?;
        let mut cfg = RunConfig::new(&s);
        cfg.stake = stake;
        Ok(Game {
            run: Run::new(cfg),
            moves: Vec::new(),
        })
    }

    #[getter]
    fn seed(&self) -> String {
        self.run.cfg.seed.clone()
    }

    #[getter]
    fn moves(&self) -> Vec<(i64, u64)> {
        self.moves.clone()
    }

    #[getter]
    fn over(&self) -> bool {
        self.run.phase == Phase::GameOver || self.run.won
    }

    /// Apply a move; False if the game refuses it (nothing changes).
    fn step(&mut self, kind: i64, arg: u64) -> bool {
        let Some(k) = MoveKind::from_index(kind) else {
            return false;
        };
        let ok = apply(&mut self.run, k, arg).is_ok();
        if ok {
            self.moves.push((kind, arg));
        }
        ok
    }

    /// The greedy bot's move (decided on a fair copy).
    fn greedy_move(&self) -> (i64, u64) {
        let (k, a) = bm_core::bots::greedy_move(&self.run, self.moves.len() as u64);
        (k as i64, a)
    }

    /// The teacher bot's move (rule-based, stronger; decided from the screen and fair copies).
    fn teacher_move(&self) -> (i64, u64) {
        let (k, a) = bm_core::teacher::teacher_move(&self.run, self.moves.len() as u64);
        (k as i64, a)
    }

    /// How the run is going: up to `n` quick greedy playouts from fair copies
    /// of this position (no more than `budget_ms` once 8 are done). Runs
    /// without holding the GIL.
    #[pyo3(signature = (n = 48, budget_ms = 400, salt = 0))]
    fn projection<'py>(
        &self,
        py: Python<'py>,
        n: usize,
        budget_ms: u64,
        salt: u64,
    ) -> PyResult<Bound<'py, PyDict>> {
        let run = self.run.clone();
        let p = py.detach(move || {
            bm_core::bots::projection(&run, n, std::time::Duration::from_millis(budget_ms), salt)
        });
        let d = PyDict::new(py);
        d.set_item("playouts", p.playouts)?;
        d.set_item("mean_ante", p.mean_ante())?;
        d.set_item("p_win", p.p_win())?;
        d.set_item("p_clear", p.p_clear())?;
        d.set_item("final_ante", p.final_ante.to_vec())?;
        Ok(d)
    }

    /// The teacher's view of this position (its value model, `bm_core::value`): the chance of beating each
    /// blind ahead, the hand that scores most, a typical round's total, and the value. None once the run
    /// is over. For the viewer; the bot itself never sees anything beyond the screen.
    fn value_view<'py>(&self, py: Python<'py>, salt: u64) -> PyResult<Option<Bound<'py, PyDict>>> {
        let run = self.run.clone();
        let b = py.detach(move || {
            if run.phase == bm_core::Phase::GameOver || run.won {
                None
            } else {
                bm_core::value::breakdown(&run, salt)
                    .map(|b| (bm_core::explain::plan_from(&run, salt, &b), b))
            }
        });
        let Some((plan, b)) = b else {
            return Ok(None);
        };
        let d = PyDict::new(py);
        let chances: Vec<(f64, f64)> = b.blinds.iter().map(|&(t, p, _)| (t, p)).collect();
        d.set_item("chances", chances)?;
        d.set_item("main", b.main.name())?;
        d.set_item("total", b.next_total)?;
        d.set_item("value", b.total)?;
        // The plan in words (from the same model), for the viewer.
        d.set_item("plan", plan)?;
        Ok(Some(d))
    }

    /// What the value network reads here (`observe::encode_value`: the screen alone), `VALUE_LEN` floats.
    fn value_input(&self) -> Vec<f32> {
        let mut x = Vec::with_capacity(VALUE_LEN);
        encode_value(&screen(&self.run), &mut x);
        x
    }

    /// The full state, in the real game's terms (for replay checks).
    fn view<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let r = &self.run;
        let d = PyDict::new(py);
        d.set_item("state", state_name(r.phase))?;
        d.set_item("ante", r.ante)?;
        d.set_item("round", r.round)?;
        d.set_item("money", r.money)?;
        d.set_item("hands_left", r.hands_left)?;
        d.set_item("discards_left", r.discards_left)?;
        d.set_item("chips", r.round_chips)?;
        d.set_item("won", r.won)?;
        let hand = pyo3::types::PyList::empty(py);
        for &id in &r.hand {
            let c = r.card(id);
            let h = PyDict::new(py);
            h.set_item("key", c.key())?;
            h.set_item("enhancement", enh_name(c.enhancement))?;
            h.set_item("edition", edition_name(c.edition))?;
            h.set_item("seal", seal_name(c.seal))?;
            h.set_item("debuff", c.debuffed)?;
            h.set_item("face_down", c.face_down)?;
            hand.append(h)?;
        }
        d.set_item("hand", hand)?;
        let jokers = pyo3::types::PyList::empty(py);
        // What each joker's card shows (the same values the bot sees).
        let shown = bm_core::observe::screen(r);
        for (j, v) in r.jokers.iter().zip(shown.jokers.iter()) {
            let h = PyDict::new(py);
            h.set_item("key", j.kind.key())?;
            h.set_item("name", j.kind.name())?;
            h.set_item("rarity", j.kind.rarity())?;
            h.set_item("edition", edition_name(j.edition))?;
            h.set_item("debuff", j.debuffed)?;
            h.set_item("face_down", j.face_down)?;
            h.set_item("eternal", j.eternal)?;
            h.set_item("perishable", v.perishable)?;
            h.set_item("rental", j.rental)?;
            // The true sell value, also for a face-down joker (the bot's screen hides it).
            h.set_item("sell", bm_core::run::joker_sell_value(j))?;
            h.set_item("mult", v.mult)?;
            h.set_item("x_mult", v.x_mult)?;
            h.set_item("chips", v.chips)?;
            h.set_item("counter", v.counter)?;
            h.set_item("hand", v.hand.map(|t| t.name()))?;
            jokers.append(h)?;
        }
        d.set_item("jokers", jokers)?;
        d.set_item(
            "vouchers",
            bm_core::content::VoucherKind::all()
                .filter(|v| r.vouchers[v.index()])
                .map(|v| v.name())
                .collect::<Vec<_>>(),
        )?;
        let levels = pyo3::types::PyList::empty(py);
        for t in bm_core::poker::HandType::ALL {
            let i = r.hand_levels.get(t);
            if i.visible && (i.played > 0 || i.level > 1) {
                let h = PyDict::new(py);
                h.set_item("name", t.name())?;
                h.set_item("level", i.level)?;
                h.set_item("chips", i.chips)?;
                h.set_item("mult", i.mult)?;
                h.set_item("played", i.played)?;
                levels.append(h)?;
            }
        }
        d.set_item("hand_levels", levels)?;
        d.set_item("last_hand", r.last_hand.map(|t| t.name()))?;
        d.set_item("hands_played_run", r.hands_played_run)?;
        d.set_item("deck_left", r.deck.len())?;
        d.set_item("deck_total", r.playing_cards.len())?;
        d.set_item("joker_slots", r.joker_slots)?;
        d.set_item("consumable_slots", r.consumable_slots)?;
        d.set_item("hand_size", r.hand_size())?;
        if let Some(c) = r.cash_out.as_ref() {
            d.set_item("cash_out", c.total)?;
        }
        if let Some(b) = r.blind.as_ref() {
            d.set_item(
                "blind_name",
                match b.kind {
                    bm_core::blinds::BlindKind::Small => "Small Blind",
                    bm_core::blinds::BlindKind::Big => "Big Blind",
                    bm_core::blinds::BlindKind::Boss(x) => x.name(),
                },
            )?;
            d.set_item("blind_reward", b.dollars)?;
            d.set_item("blind_disabled", b.disabled)?;
        }
        let blinds = PyDict::new(py);
        for s in BlindSlot::ALL {
            let b = PyDict::new(py);
            let kind = r.blind_in_slot(s);
            b.set_item(
                "name",
                match kind {
                    bm_core::blinds::BlindKind::Small => "Small Blind",
                    bm_core::blinds::BlindKind::Big => "Big Blind",
                    bm_core::blinds::BlindKind::Boss(x) => x.name(),
                },
            )?;
            b.set_item("status", slot_name(r.slots[s.index()]))?;
            b.set_item(
                "score",
                bm_core::blinds::blind_target(
                    r.ante,
                    bm_core::blinds::scaling_for_stake(r.cfg.stake),
                    kind,
                    1.0,
                ),
            )?;
            b.set_item("reward", kind.reward(r.cfg.stake))?;
            if s != BlindSlot::Boss {
                b.set_item("tag_name", r.tags_offered[s.index()].name())?;
            }
            blinds.set_item(["small", "big", "boss"][s.index()], b)?;
        }
        d.set_item("blinds", blinds)?;
        if let Some(b) = r.blind.as_ref() {
            d.set_item("blind_score", b.chips)?;
        }
        d.set_item(
            "tags",
            r.tags.iter().map(|t| t.kind.name()).collect::<Vec<_>>(),
        )?;
        d.set_item("reroll_cost", r.reroll_cost)?;
        let consumables = pyo3::types::PyList::empty(py);
        for c in &r.consumables {
            let h = consumable_item(py, c)?;
            h.set_item("cost", c.cost)?;
            h.set_item("sell", c.sell_cost)?;
            consumables.append(h)?;
        }
        d.set_item("consumables", consumables)?;
        // The shop: the joker row, the vouchers, and the two booster slots
        // (None once opened).
        let row = pyo3::types::PyList::empty(py);
        let vouchers = pyo3::types::PyList::empty(py);
        let packs = pyo3::types::PyList::empty(py);
        if let Some(shop) = r.shop.as_ref() {
            for slot in &shop.row {
                let h = match &slot.item {
                    RowItem::Joker(j) => joker_item(py, j)?,
                    RowItem::Consumable(c) => consumable_item(py, c)?,
                    RowItem::Card(c) => playing_card(py, c)?,
                };
                h.set_item("cost", slot.cost)?;
                row.append(h)?;
            }
            for v in &shop.vouchers {
                let h = item(py, v.voucher.key().to_string(), Edition::None)?;
                h.set_item("cost", v.cost)?;
                vouchers.append(h)?;
            }
            for b in &shop.boosters {
                match b {
                    Some(b) => {
                        let h = item(
                            py,
                            BOOSTERS[b.booster as usize].key.to_string(),
                            Edition::None,
                        )?;
                        h.set_item("cost", b.cost)?;
                        packs.append(h)?;
                    }
                    None => packs.append(py.None())?,
                }
            }
        }
        d.set_item("shop", row)?;
        d.set_item("shop_vouchers", vouchers)?;
        d.set_item("shop_packs", packs)?;
        // An open pack's cards.
        let pack = pyo3::types::PyList::empty(py);
        if let Some(p) = r.pack.as_ref() {
            for c in &p.cards {
                let h = match c {
                    PackCard::Joker(j) => joker_item(py, j)?,
                    PackCard::Consumable(c) => consumable_item(py, c)?,
                    PackCard::Card(c) => playing_card(py, c)?,
                };
                pack.append(h)?;
            }
            d.set_item("pack_key", BOOSTERS[p.booster as usize].key)?;
            d.set_item("pack_choices_left", p.choices_left)?;
        }
        d.set_item("pack", pack)?;
        Ok(d)
    }
}

/// A shop or pack item: its key, its name and its edition.
fn item<'py>(py: Python<'py>, key: String, e: Edition) -> PyResult<Bound<'py, PyDict>> {
    let h = PyDict::new(py);
    h.set_item("name", item_name(&key))?;
    h.set_item("key", key)?;
    h.set_item("edition", edition_name(e))?;
    Ok(h)
}

/// A joker offered in the shop or a pack, with its rarity (1 common to 4 legendary).
fn joker_item<'py>(py: Python<'py>, j: &bm_core::jokers::Joker) -> PyResult<Bound<'py, PyDict>> {
    let h = item(py, j.kind.key().to_string(), j.edition)?;
    h.set_item("rarity", j.kind.rarity())?;
    Ok(h)
}

/// A consumable, with its set (Tarot, Planet or Spectral).
fn consumable_item<'py>(
    py: Python<'py>,
    c: &bm_core::create::Consumable,
) -> PyResult<Bound<'py, PyDict>> {
    let h = item(py, c.kind.key().to_string(), c.edition)?;
    h.set_item(
        "set",
        match c.kind.set() {
            bm_core::content::ConsumableSet::Tarot => "Tarot",
            bm_core::content::ConsumableSet::Planet => "Planet",
            bm_core::content::ConsumableSet::Spectral => "Spectral",
        },
    )?;
    Ok(h)
}

/// A readable name for a joker, consumable, voucher, pack or playing card key.
fn item_name(key: &str) -> String {
    use bm_core::content::{ConsumableKind, VoucherKind};
    use bm_core::joker_data::JOKERS;
    if let Some(j) = JOKERS.iter().find(|j| j.key == key) {
        return j.name.to_string();
    }
    if let Some(c) = ConsumableKind::ALL.iter().find(|c| c.key() == key) {
        return c.name().to_string();
    }
    if let Some(v) = VoucherKind::all().find(|v| v.key() == key) {
        return v.name().to_string();
    }
    if let Some(rest) = key.strip_prefix("p_") {
        let mut parts = rest.split('_');
        let kind = parts.next().unwrap_or("");
        let size = parts.next().unwrap_or("");
        let mut kind = kind.to_string();
        if let Some(f) = kind.get_mut(0..1) {
            f.make_ascii_uppercase();
        }
        return match size {
            "jumbo" => format!("Jumbo {kind} Pack"),
            "mega" => format!("Mega {kind} Pack"),
            _ => format!("{kind} Pack"),
        };
    }
    key.to_string()
}

/// A playing card offered in the shop or a pack.
fn playing_card<'py>(py: Python<'py>, c: &bm_core::card::Card) -> PyResult<Bound<'py, PyDict>> {
    let h = item(py, c.key(), c.edition)?;
    h.set_item("enhancement", enh_name(c.enhancement))?;
    h.set_item("seal", seal_name(c.seal))?;
    Ok(h)
}

#[pymodule]
fn _engine(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add("OBS_LEN", OBS_LEN)?;
    m.add("N_MOVE_KINDS", N_MOVE_KINDS)?;
    m.add("VALUE_LEN", VALUE_LEN)?;
    m.add("OBS_LAYOUT", obs_layout())?;
    m.add_function(wrap_pyfunction!(run_seed, m)?)?;
    m.add("VALUE_INPUT_VERSION", VALUE_INPUT_VERSION)?;
    m.add_function(wrap_pyfunction!(pseudohash, m)?)?;
    m.add_function(wrap_pyfunction!(value_net_forward, m)?)?;
    m.add_function(wrap_pyfunction!(lua_random, m)?)?;
    m.add_class::<Streams>()?;
    m.add_class::<VecEnv>()?;
    m.add_class::<Game>()?;
    Ok(())
}
