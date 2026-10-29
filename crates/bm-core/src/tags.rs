//! Skip tags: the pool, its availability rules and the per-ante draw
//! (`rules/blinds.md` sections 3-5).

use bm_rng::{pick_index, Streams};

/// Every skip tag, in the pool's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum TagKind {
    Uncommon,
    Rare,
    Negative,
    Foil,
    Holographic,
    Polychrome,
    Investment,
    Voucher,
    Boss,
    Standard,
    Charm,
    Meteor,
    Buffoon,
    Handy,
    Garbage,
    Ethereal,
    Coupon,
    Double,
    Juggle,
    D6,
    TopUp,
    Skip,
    Orbital,
    Economy,
}

pub const N_TAGS: usize = 24;

/// What a tag's availability depends on in the player's profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TagRequires {
    Blueprint,
    Negative,
    Foil,
    Holographic,
    Polychrome,
}

impl TagKind {
    pub const ALL: [TagKind; N_TAGS] = [
        TagKind::Uncommon,
        TagKind::Rare,
        TagKind::Negative,
        TagKind::Foil,
        TagKind::Holographic,
        TagKind::Polychrome,
        TagKind::Investment,
        TagKind::Voucher,
        TagKind::Boss,
        TagKind::Standard,
        TagKind::Charm,
        TagKind::Meteor,
        TagKind::Buffoon,
        TagKind::Handy,
        TagKind::Garbage,
        TagKind::Ethereal,
        TagKind::Coupon,
        TagKind::Double,
        TagKind::Juggle,
        TagKind::D6,
        TagKind::TopUp,
        TagKind::Skip,
        TagKind::Orbital,
        TagKind::Economy,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn key(self) -> &'static str {
        match self {
            TagKind::Uncommon => "tag_uncommon",
            TagKind::Rare => "tag_rare",
            TagKind::Negative => "tag_negative",
            TagKind::Foil => "tag_foil",
            TagKind::Holographic => "tag_holo",
            TagKind::Polychrome => "tag_polychrome",
            TagKind::Investment => "tag_investment",
            TagKind::Voucher => "tag_voucher",
            TagKind::Boss => "tag_boss",
            TagKind::Standard => "tag_standard",
            TagKind::Charm => "tag_charm",
            TagKind::Meteor => "tag_meteor",
            TagKind::Buffoon => "tag_buffoon",
            TagKind::Handy => "tag_handy",
            TagKind::Garbage => "tag_garbage",
            TagKind::Ethereal => "tag_ethereal",
            TagKind::Coupon => "tag_coupon",
            TagKind::Double => "tag_double",
            TagKind::Juggle => "tag_juggle",
            TagKind::D6 => "tag_d_six",
            TagKind::TopUp => "tag_top_up",
            TagKind::Skip => "tag_skip",
            TagKind::Orbital => "tag_orbital",
            TagKind::Economy => "tag_economy",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            TagKind::Uncommon => "Uncommon Tag",
            TagKind::Rare => "Rare Tag",
            TagKind::Negative => "Negative Tag",
            TagKind::Foil => "Foil Tag",
            TagKind::Holographic => "Holographic Tag",
            TagKind::Polychrome => "Polychrome Tag",
            TagKind::Investment => "Investment Tag",
            TagKind::Voucher => "Voucher Tag",
            TagKind::Boss => "Boss Tag",
            TagKind::Standard => "Standard Tag",
            TagKind::Charm => "Charm Tag",
            TagKind::Meteor => "Meteor Tag",
            TagKind::Buffoon => "Buffoon Tag",
            TagKind::Handy => "Handy Tag",
            TagKind::Garbage => "Garbage Tag",
            TagKind::Ethereal => "Ethereal Tag",
            TagKind::Coupon => "Coupon Tag",
            TagKind::Double => "Double Tag",
            TagKind::Juggle => "Juggle Tag",
            TagKind::D6 => "D6 Tag",
            TagKind::TopUp => "Top-up Tag",
            TagKind::Skip => "Skip Tag",
            TagKind::Orbital => "Orbital Tag",
            TagKind::Economy => "Economy Tag",
        }
    }

    /// The first ante the tag can be drawn at.
    pub fn min_ante(self) -> i32 {
        match self {
            TagKind::Negative
            | TagKind::Standard
            | TagKind::Meteor
            | TagKind::Buffoon
            | TagKind::Handy
            | TagKind::Garbage
            | TagKind::Ethereal
            | TagKind::TopUp
            | TagKind::Orbital => 2,
            _ => i32::MIN,
        }
    }

    pub fn requires(self) -> Option<TagRequires> {
        match self {
            TagKind::Rare => Some(TagRequires::Blueprint),
            TagKind::Negative => Some(TagRequires::Negative),
            TagKind::Foil => Some(TagRequires::Foil),
            TagKind::Holographic => Some(TagRequires::Holographic),
            TagKind::Polychrome => Some(TagRequires::Polychrome),
            _ => None,
        }
    }
}

/// The parts of the player's profile that tag pools read. A seeded run never
/// discovers anything new, so this is fixed for the run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TagProfile {
    pub blueprint_discovered: bool,
    pub negative_discovered: bool,
    pub foil_discovered: bool,
    pub holo_discovered: bool,
    pub polychrome_discovered: bool,
}

impl Default for TagProfile {
    /// Everything discovered.
    fn default() -> Self {
        TagProfile {
            blueprint_discovered: true,
            negative_discovered: true,
            foil_discovered: true,
            holo_discovered: true,
            polychrome_discovered: true,
        }
    }
}

impl TagProfile {
    fn has(&self, r: TagRequires) -> bool {
        match r {
            TagRequires::Blueprint => self.blueprint_discovered,
            TagRequires::Negative => self.negative_discovered,
            TagRequires::Foil => self.foil_discovered,
            TagRequires::Holographic => self.holo_discovered,
            TagRequires::Polychrome => self.polychrome_discovered,
        }
    }
}

/// Is `tag` in the pool at `ante` (else its slot holds the unavailable
/// placeholder)?
pub fn tag_available(
    tag: TagKind,
    ante: i32,
    profile: &TagProfile,
    banned: &[bool; N_TAGS],
) -> bool {
    tag.requires().is_none_or(|r| profile.has(r)) && tag.min_ante() <= ante && !banned[tag.index()]
}

/// Draw one skip tag for `ante` on stream `Tag<ante>`, resampling on
/// `Tag<ante>_resample<k>` while the pick lands on an unavailable slot.
pub fn draw_tag(
    rng: &mut Streams,
    ante: i32,
    profile: &TagProfile,
    banned: &[bool; N_TAGS],
) -> TagKind {
    let avail: Vec<bool> = TagKind::ALL
        .iter()
        .map(|&t| tag_available(t, ante, profile, banned))
        .collect();
    if !avail.iter().any(|&a| a) {
        return TagKind::Handy;
    }
    let key = format!("Tag{ante}");
    let mut g = rng.generator(&key);
    let mut i = pick_index(N_TAGS, &mut g);
    let mut it = 1;
    while !avail[i] {
        it += 1;
        let mut g = rng.generator(&format!("{key}_resample{it}"));
        i = pick_index(N_TAGS, &mut g);
    }
    TagKind::ALL[i]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ante_one_pool_has_fifteen_tags() {
        let p = TagProfile::default();
        let banned = [false; N_TAGS];
        let n1 = TagKind::ALL
            .iter()
            .filter(|&&t| tag_available(t, 1, &p, &banned))
            .count();
        let n2 = TagKind::ALL
            .iter()
            .filter(|&&t| tag_available(t, 2, &p, &banned))
            .count();
        assert_eq!((n1, n2), (15, 24));
    }

    #[test]
    fn draws_only_available_tags() {
        let p = TagProfile::default();
        let banned = [false; N_TAGS];
        for seed in ["AAAAAAAA", "12345678", "ZZZZZZZZ", "K9JQ2B7X"] {
            let mut rng = Streams::new(seed);
            for _ in 0..40 {
                let t = draw_tag(&mut rng, 1, &p, &banned);
                assert!(tag_available(t, 1, &p, &banned), "{t:?} drawn at ante 1");
            }
        }
    }
}
