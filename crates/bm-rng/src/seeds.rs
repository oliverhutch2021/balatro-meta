//! Seed strings: normalising typed seeds and drawing fresh ones for training.

/// Characters a seed may contain: 1-9 and A-Z. A typed 0 becomes O.
pub const SEED_ALPHABET: &[u8; 35] = b"123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";

/// Longest seed the game accepts.
pub const SEED_LEN: usize = 8;

/// Normalise a typed seed the way the seed box does: uppercase, 0 → O, only
/// [1-9A-Z], 1 to 8 characters. Returns `None` for anything else.
pub fn normalize_seed(s: &str) -> Option<String> {
    let out: String = s
        .trim()
        .chars()
        .map(|c| c.to_ascii_uppercase())
        .map(|c| if c == '0' { 'O' } else { c })
        .collect();
    if out.is_empty() || out.len() > SEED_LEN {
        return None;
    }
    if !out.bytes().all(|b| SEED_ALPHABET.contains(&b)) {
        return None;
    }
    Some(out)
}

/// A small fair generator (SplitMix64) for choosing fresh seeds. It is
/// deliberately unrelated to the game's own generator, so nothing about the
/// game's streams can leak into which seeds a bot is trained on.
#[derive(Clone, Debug)]
pub struct SeedSampler {
    state: u64,
}

impl SeedSampler {
    pub fn new(seed: u64) -> Self {
        SeedSampler { state: seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A uniform integer in [0, n), without modulo bias.
    pub fn below(&mut self, n: u64) -> u64 {
        assert!(n > 0);
        let zone = u64::MAX - (u64::MAX % n);
        loop {
            let x = self.next_u64();
            if x < zone {
                return x % n;
            }
        }
    }

    /// A fresh 8-character seed, uniform over all 35^8 (about 2.25 trillion).
    pub fn next_seed(&mut self) -> String {
        (0..SEED_LEN)
            .map(|_| SEED_ALPHABET[self.below(SEED_ALPHABET.len() as u64) as usize] as char)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_like_the_seed_box() {
        assert_eq!(normalize_seed("abc0def1").as_deref(), Some("ABCODEF1"));
        assert_eq!(normalize_seed("  7lb2wvpk ").as_deref(), Some("7LB2WVPK"));
        assert_eq!(normalize_seed(""), None);
        assert_eq!(normalize_seed("TOOLONGSEED"), None);
        assert_eq!(normalize_seed("AB-CD"), None);
    }

    #[test]
    fn fresh_seeds_are_valid_and_vary() {
        let mut s = SeedSampler::new(1);
        let a = s.next_seed();
        let b = s.next_seed();
        assert_ne!(a, b);
        assert_eq!(normalize_seed(&a).as_deref(), Some(a.as_str()));
        assert_eq!(a.len(), 8);
    }
}
