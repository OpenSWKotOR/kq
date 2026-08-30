//! Work out the `[SSFList]` instructions that turn one soundset into another.
//!
//! A soundset is a fixed array of talk-table references, one per slot, so the
//! only edit there is to express is "this slot now points somewhere else".
//! Nothing can be added or removed.
//!
//! Slot names come from the shared crate's canonical list, which carries all
//! forty the original patcher defines — the last twelve are unnamed in the
//! games and appear as `Unknown(29)` upward. Later tools know only the first
//! twenty-eight; writing the canonical names keeps the output readable by the
//! patcher this targets.

use kotor_formats::ssf::{SsfFile, ENTRY_LABELS};

use super::{ChangesIni, Section};

/// The talk-table reference a slot carries when nothing is assigned.
const UNASSIGNED: u32 = u32::MAX;

impl ChangesIni {
    /// Add the instructions that turn `base` into `modified`.
    ///
    /// `filename` is the soundset as it will be named in `[SSFList]`, for
    /// example `my_creature.ssf`.
    pub fn add_ssf(&mut self, filename: &str, base: &SsfFile, modified: &SsfFile) {
        let mut section = Section::new(filename);

        for (slot, label) in ENTRY_LABELS.iter().enumerate() {
            let before = base.entries()[slot];
            let after = modified.entries()[slot];
            if before == after {
                continue;
            }
            // An unassigned slot reads back as -1 rather than a huge number.
            let value = if after == UNASSIGNED {
                "-1".to_string()
            } else {
                after.to_string()
            };
            self.changed_entries
                .push((filename.to_string(), (*label).to_string()));
            section.set(*label, value);
        }

        if section.entries.is_empty() {
            return;
        }

        if !self.ssf_files.iter().any(|f| f == filename) {
            self.ssf_files.push(filename.to_string());
        }
        self.push(section);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn soundset(pairs: &[(&str, u32)]) -> SsfFile {
        let mut ssf = SsfFile::new();
        for (label, value) in pairs {
            ssf.set_entry(label, *value).unwrap();
        }
        ssf
    }

    #[test]
    fn an_unchanged_soundset_produces_nothing() {
        let base = soundset(&[("Battlecry 1", 100)]);
        let modified = soundset(&[("Battlecry 1", 100)]);

        let mut ini = ChangesIni::new();
        ini.add_ssf("my_creature.ssf", &base, &modified);

        assert!(ini.is_empty());
        assert!(!ini.render().contains("[SSFList]"));
    }

    #[test]
    fn a_changed_slot_is_listed_by_its_canonical_name() {
        let base = soundset(&[("Battlecry 1", 100), ("Death", 200)]);
        let modified = soundset(&[("Battlecry 1", 100), ("Death", 999)]);

        let mut ini = ChangesIni::new();
        ini.add_ssf("my_creature.ssf", &base, &modified);
        let out = ini.render();

        assert!(out.contains("[SSFList]\nFile0=my_creature.ssf\n"));
        assert!(out.contains("[my_creature.ssf]\nDeath=999\n"));
        // The slot that did not move is left out.
        assert!(!out.contains("Battlecry 1="));
    }

    #[test]
    fn clearing_a_slot_is_written_as_minus_one() {
        let base = soundset(&[("Search", 42)]);
        let modified = soundset(&[("Search", u32::MAX)]);

        let mut ini = ChangesIni::new();
        ini.add_ssf("my_creature.ssf", &base, &modified);

        assert!(ini.render().contains("Search=-1\n"));
    }

    #[test]
    fn the_unnamed_slots_are_still_addressable() {
        let base = soundset(&[]);
        let mut modified = SsfFile::new();
        modified.set_entry("Unknown(30)", 7).unwrap();

        let mut ini = ChangesIni::new();
        ini.add_ssf("my_creature.ssf", &base, &modified);

        assert!(ini.render().contains("Unknown(30)=7\n"));
    }
}
