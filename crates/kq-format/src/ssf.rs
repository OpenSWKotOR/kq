//! SSF — a creature's 28 fixed sound-event slots, each a StrRef into
//! `dialog.tlk`.
//!
//! Layout (V1.1): `"SSF "`, `"V1.1"`, u32 table_offset (conventionally 12),
//! then 28 little-endian i32 StrRefs in a fixed event order. `0xFFFFFFFF`
//! means "no sound for this event", represented here as `-1`.

use std::path::Path;

use crate::error::Result;
use crate::reader::Reader;

/// The 28 sound-event slots, in on-disk order. Matches PyKotor's `SSFSound`.
pub const EVENTS: [&str; 28] = [
    "battle_cry_1",
    "battle_cry_2",
    "battle_cry_3",
    "battle_cry_4",
    "battle_cry_5",
    "battle_cry_6",
    "select_1",
    "select_2",
    "select_3",
    "attack_grunt_1",
    "attack_grunt_2",
    "attack_grunt_3",
    "pain_grunt_1",
    "pain_grunt_2",
    "low_health",
    "dead",
    "critical_hit",
    "target_immune",
    "lay_mine",
    "disarm_mine",
    "begin_stealth",
    "begin_search",
    "begin_unlock",
    "unlock_failed",
    "unlock_success",
    "separated_from_party",
    "rejoined_party",
    "poisoned",
];

#[derive(Clone, Debug)]
pub struct Ssf {
    /// One StrRef per [`EVENTS`] slot, `-1` for "no sound".
    pub sounds: [i64; 28],
}

pub fn sniff(data: &[u8]) -> bool {
    data.starts_with(b"SSF ")
}

pub fn read(data: &[u8], path: &Path) -> Result<Ssf> {
    let mut r = Reader::new(data, path);
    r.expect_signature("SSF V1.1")?;
    r.seek(8)?;
    let table_offset = r.u32()? as usize;
    r.seek(table_offset)?;

    let mut sounds = [-1i64; 28];
    for slot in &mut sounds {
        let raw = r.u32()?;
        *slot = if raw == 0xFFFF_FFFF { -1 } else { raw as i64 };
    }
    Ok(Ssf { sounds })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn reads_twenty_eight_slots() {
        let mut data = b"SSF V1.1".to_vec();
        data.extend_from_slice(&12u32.to_le_bytes());
        for i in 0..28u32 {
            data.extend_from_slice(&i.to_le_bytes());
        }
        let s = read(&data, Path::new("c_human.ssf")).unwrap();
        assert_eq!(s.sounds[0], 0);
        assert_eq!(s.sounds[27], 27);
    }

    #[test]
    fn missing_sound_is_minus_one() {
        let mut data = b"SSF V1.1".to_vec();
        data.extend_from_slice(&12u32.to_le_bytes());
        data.extend_from_slice(&[0xFF; 28 * 4]);
        let s = read(&data, Path::new("empty.ssf")).unwrap();
        assert!(s.sounds.iter().all(|&n| n == -1));
    }
}
