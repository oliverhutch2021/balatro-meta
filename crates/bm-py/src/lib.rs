//! Python bindings: `balatro_meta._engine`.
//!
//! For now this exposes the RNG, so the build and the PC setup can be checked
//! end to end. The engine and the batched environment for training are added
//! here as they land.

use pyo3::prelude::*;

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

#[pymodule]
fn _engine(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add_function(wrap_pyfunction!(pseudohash, m)?)?;
    m.add_function(wrap_pyfunction!(lua_random, m)?)?;
    m.add_class::<Streams>()?;
    Ok(())
}
