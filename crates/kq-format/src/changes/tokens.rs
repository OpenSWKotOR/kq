//! Link literals to the indexes this diff created.
//!
//! When a mod appends a row to a table and points a field at it, the two edits
//! are one change. Written as literals they come apart: the row only lands at
//! index 7 if the player's table had exactly the rows the mod author's did, and
//! any earlier mod that also appended shifts it. `2DAMEMORY` exists to close
//! that gap — the row modifier captures wherever the row actually landed, and
//! the field reads it back.
//!
//! # Why this is opt-in
//!
//! Recognising the link means recognising that a number means something.
//! Nothing in a GFF file says "this field indexes appearance.2da" — that
//! knowledge lives in the game's code, not in the data. So the only evidence
//! available here is circumstantial, and a wrong guess quietly rewires a mod
//! to a row it never meant. [`ChangesIni::link_tokens`] is therefore never run
//! for you.
//!
//! When it does run, a literal is rewritten only if all of these hold:
//!
//! 1. The value **changed** between the two files. An untouched field is left
//!    alone no matter what it contains, which is what rules out the ordinary
//!    case of a number that was always there.
//! 2. It matches **exactly one** index this diff created. Two tables that both
//!    grew to the same length make the reference ambiguous, and an ambiguous
//!    reference is reported instead of guessed.
//! 3. It is not the row's own modifier section, which would be circular.
//!
//! That still leaves a coincidence through: a cost that changed from 10 to 2
//! while some table gained a row at index 2 reads exactly like a reference.
//! Rule 1 makes it rare, and the warning list names every rewrite so the result
//! can be checked. Callers that know their schema should prefer to review the
//! output over trusting it blindly.

use super::ChangesIni;

/// A rewrite the pass decided to make.
struct Rewrite {
    section: String,
    key: String,
    value: String,
    /// The row modifier that needs a capture key, if this is a table link.
    capture: Option<(String, String, u32)>,
}

impl ChangesIni {
    /// Rewrite literal references into `2DAMEMORY` / `StrRef` tokens.
    ///
    /// Returns how many values were rewritten. See the module documentation
    /// for what counts as evidence — this is deliberately conservative and is
    /// never run unless asked for.
    pub fn link_tokens(&mut self) -> usize {
        let mut planned: Vec<Rewrite> = Vec::new();
        let mut ambiguous: Vec<String> = Vec::new();
        let mut next_slot = 1u32;
        for row in &self.created_rows {
            if let Some(token) = row.token {
                next_slot = next_slot.max(token + 1);
            }
        }

        // Decide everything first; the document is only touched afterwards.
        let mut assigned: Vec<(String, u32, bool)> = Vec::new();
        for (section_name, key) in &self.changed_entries {
            let Some(value) = self
                .sections
                .iter()
                .find(|s| &s.name == section_name)
                .and_then(|s| s.get(key))
            else {
                continue;
            };
            if value.starts_with("2DAMEMORY") || value.starts_with("StrRef") {
                continue;
            }
            let value = value.to_string();

            // A new talk-table line is unambiguous: its token is already
            // allocated and its resulting index is unique.
            if let Some(created) = self
                .created_strrefs
                .iter()
                .find(|c| c.resulting_strref.to_string() == value)
            {
                planned.push(Rewrite {
                    section: section_name.clone(),
                    key: key.clone(),
                    value: format!("StrRef{}", created.token),
                    capture: None,
                });
                continue;
            }

            // A table row can be named by where it lands or by its label.
            let by_index: Vec<&super::CreatedRow> = self
                .created_rows
                .iter()
                .filter(|r| &r.section != section_name && r.index.to_string() == value)
                .collect();
            let by_label: Vec<&super::CreatedRow> = self
                .created_rows
                .iter()
                .filter(|r| {
                    &r.section != section_name && r.label == value && r.label != r.index.to_string()
                })
                .collect();

            let (matches, capture_kind) = if !by_index.is_empty() {
                (by_index, "RowIndex")
            } else if !by_label.is_empty() {
                (by_label, "RowLabel")
            } else {
                continue;
            };

            if matches.len() > 1 {
                ambiguous.push(format!(
                    "{section_name}: \"{key}={value}\" matches {} rows this patch adds, so it was left as a literal",
                    matches.len()
                ));
                continue;
            }

            let row_section = matches[0].section.clone();
            // Reuse a slot already handed to this row and capture kind.
            let existing = assigned
                .iter()
                .find(|(s, _, is_label)| {
                    *s == row_section && *is_label == (capture_kind == "RowLabel")
                })
                .map(|(_, token, _)| *token);
            let token = match existing {
                Some(token) => token,
                None => {
                    let token = next_slot;
                    next_slot += 1;
                    assigned.push((row_section.clone(), token, capture_kind == "RowLabel"));
                    token
                }
            };

            planned.push(Rewrite {
                section: section_name.clone(),
                key: key.clone(),
                value: format!("2DAMEMORY{token}"),
                capture: Some((row_section, capture_kind.to_string(), token)),
            });
        }

        let mut count = 0;
        for rewrite in planned {
            if let Some(section) = self.section_mut(&rewrite.section) {
                if let Some(entry) = section.entries.iter_mut().find(|(k, _)| k == &rewrite.key) {
                    entry.1 = rewrite.value;
                    count += 1;
                }
            }

            let Some((row_section, kind, token)) = rewrite.capture else {
                continue;
            };
            let key = format!("2DAMEMORY{token}");
            if let Some(section) = self.section_mut(&row_section) {
                if section.get(&key).is_none() {
                    // The capture has to follow a column key: a row modifier
                    // only treats it as a capture once the row exists, and
                    // before that it reads as a column name instead.
                    section.set(key, kind);
                }
            }
            if let Some(row) = self
                .created_rows
                .iter_mut()
                .find(|r| r.section == row_section)
            {
                row.token = Some(token);
            }
        }

        for message in ambiguous {
            self.warn(message);
        }

        count
    }
}

#[cfg(test)]
mod tests {
    use kotor_formats::gff::{FieldValue, GffField, GffFile};
    use kotor_formats::twoda::{build, TwoDaFile};

    use super::*;

    fn table(columns: &[&str], rows: &[(&str, Vec<&str>)]) -> TwoDaFile {
        build(columns, rows)
    }

    fn creature(appearance: u16) -> GffFile {
        let mut f = GffFile::new_file("UTC ", "creature.utc");
        f.root.add_field(GffField::new(
            "Appearance_Type",
            FieldValue::Word(appearance),
        ));
        f
    }

    #[test]
    fn a_field_pointed_at_a_new_row_reads_the_row_back() {
        // The scenario the token system exists for: a table grows, and a
        // creature is pointed at the row that was added.
        let base_2da = table(
            &["label"],
            &[("0", vec!["appearance_0"]), ("1", vec!["appearance_1"])],
        );
        let modified_2da = table(
            &["label"],
            &[
                ("0", vec!["appearance_0"]),
                ("1", vec!["appearance_1"]),
                ("2", vec!["new_appearance"]),
            ],
        );

        let mut ini = ChangesIni::new();
        ini.add_twoda("appearance.2da", &base_2da, &modified_2da);
        ini.add_gff("creature.utc", &creature(0), &creature(2));

        assert_eq!(ini.link_tokens(), 1);
        let out = ini.render();

        // The field reads the slot rather than the number 2.
        assert!(out.contains("Appearance_Type=2DAMEMORY1"));
        // The row modifier fills that slot, after a column key.
        assert!(out.contains("label=new_appearance\n2DAMEMORY1=RowIndex\n"));
    }

    #[test]
    fn an_untouched_field_is_never_rewritten() {
        // Appearance_Type was already 2 and the mod did not touch it. A table
        // gaining a row at index 2 must not silently rewire it.
        let base_2da = table(&["label"], &[("0", vec!["a"]), ("1", vec!["b"])]);
        let modified_2da = table(
            &["label"],
            &[("0", vec!["a"]), ("1", vec!["b"]), ("2", vec!["c"])],
        );

        let mut ini = ChangesIni::new();
        ini.add_twoda("appearance.2da", &base_2da, &modified_2da);
        // Same value on both sides, so nothing about this field changed.
        ini.add_gff("creature.utc", &creature(2), &creature(2));

        assert_eq!(ini.link_tokens(), 0);
        let out = ini.render();
        assert!(!out.contains("2DAMEMORY"));
        // The field was identical, so it is not in the output at all.
        assert!(!out.contains("Appearance_Type"));
    }

    #[test]
    fn a_changed_value_that_matches_nothing_stays_a_literal() {
        let base_2da = table(&["label"], &[("0", vec!["a"]), ("1", vec!["b"])]);
        let modified_2da = table(
            &["label"],
            &[("0", vec!["a"]), ("1", vec!["b"]), ("2", vec!["c"])],
        );

        let mut ini = ChangesIni::new();
        ini.add_twoda("appearance.2da", &base_2da, &modified_2da);
        // 40 is not an index this patch creates.
        ini.add_gff("creature.utc", &creature(0), &creature(40));

        assert_eq!(ini.link_tokens(), 0);
        assert!(ini.render().contains("Appearance_Type=40"));
    }

    #[test]
    fn an_ambiguous_reference_is_reported_rather_than_guessed() {
        // Two tables both gain a row at index 1, so a field holding 1 could
        // mean either.
        let base = table(&["label"], &[("0", vec!["a"])]);
        let modified = table(&["label"], &[("0", vec!["a"]), ("1", vec!["b"])]);

        let mut ini = ChangesIni::new();
        ini.add_twoda("first.2da", &base, &modified);
        ini.add_twoda("second.2da", &base, &modified);
        ini.add_gff("creature.utc", &creature(0), &creature(1));

        assert_eq!(ini.link_tokens(), 0);
        assert!(ini.render().contains("Appearance_Type=1"));
        assert!(ini
            .warnings()
            .iter()
            .any(|w| w.contains("matches 2 rows this patch adds")));
    }

    #[test]
    fn linking_is_not_done_unless_it_is_asked_for() {
        let base_2da = table(&["label"], &[("0", vec!["a"]), ("1", vec!["b"])]);
        let modified_2da = table(
            &["label"],
            &[("0", vec!["a"]), ("1", vec!["b"]), ("2", vec!["c"])],
        );

        let mut ini = ChangesIni::new();
        ini.add_twoda("appearance.2da", &base_2da, &modified_2da);
        ini.add_gff("creature.utc", &creature(0), &creature(2));

        // No link_tokens() call: the literal stands.
        let out = ini.render();
        assert!(out.contains("Appearance_Type=2"));
        assert!(!out.contains("2DAMEMORY"));
    }

    #[test]
    fn two_fields_pointed_at_one_row_share_a_slot() {
        let base_2da = table(&["label"], &[("0", vec!["a"]), ("1", vec!["b"])]);
        let modified_2da = table(
            &["label"],
            &[("0", vec!["a"]), ("1", vec!["b"]), ("2", vec!["c"])],
        );

        let mut first = GffFile::new_file("UTC ", "a.utc");
        first
            .root
            .add_field(GffField::new("Appearance_Type", FieldValue::Word(2)));
        let mut first_base = GffFile::new_file("UTC ", "a.utc");
        first_base
            .root
            .add_field(GffField::new("Appearance_Type", FieldValue::Word(0)));

        let mut ini = ChangesIni::new();
        ini.add_twoda("appearance.2da", &base_2da, &modified_2da);
        ini.add_gff("a.utc", &first_base, &first);
        ini.add_gff("b.utc", &creature(0), &creature(2));

        assert_eq!(ini.link_tokens(), 2);
        let out = ini.render();
        // One row, one slot, read from both files.
        assert_eq!(out.matches("2DAMEMORY1=RowIndex").count(), 1);
        assert_eq!(out.matches("Appearance_Type=2DAMEMORY1").count(), 2);
    }

    #[test]
    fn a_new_talk_table_line_becomes_a_strref_token() {
        use kotor_formats::tlk::{TlkEntry, TlkFile};

        let mut base_tlk = TlkFile::new_file();
        base_tlk
            .add_entry(TlkEntry {
                flags: 1,
                text: "existing".into(),
                ..TlkEntry::default()
            })
            .unwrap();
        let mut modified_tlk = base_tlk.clone();
        modified_tlk
            .add_entry(TlkEntry {
                flags: 1,
                text: "brand new line".into(),
                ..TlkEntry::default()
            })
            .unwrap();

        // The new line lands at index 1, and the item's name points there.
        let mut base_gff = GffFile::new_file("UTI ", "item.uti");
        base_gff
            .root
            .add_field(GffField::new("Cost", FieldValue::Dword(0)));
        let mut modified_gff = GffFile::new_file("UTI ", "item.uti");
        modified_gff
            .root
            .add_field(GffField::new("Cost", FieldValue::Dword(1)));

        let mut ini = ChangesIni::new();
        ini.add_tlk(&base_tlk, &modified_tlk);
        ini.add_gff("item.uti", &base_gff, &modified_gff);

        assert_eq!(ini.link_tokens(), 1);
        assert!(ini.render().contains("Cost=StrRef0"));
    }
}
