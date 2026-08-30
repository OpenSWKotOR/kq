//! Work out the `[2DAList]` instructions that turn one table into another.
//!
//! Three shapes of edit are expressible: a new column, a changed cell in a row
//! both tables have, and a whole new row. Deletions are not — TSLPatcher has
//! no `DeleteRow`, so a table that lost a row is reported as a warning instead
//! of being silently half-patched.

use kotor_formats::twoda::{default_cell, TwoDaFile};

use super::{escape, slug, ChangesIni, Section};

impl ChangesIni {
    /// Add the instructions that turn `base` into `modified`.
    ///
    /// `filename` is the table as it will be named in `[2DAList]`, for example
    /// `spells.2da`.
    pub fn add_twoda(&mut self, filename: &str, base: &TwoDaFile, modified: &TwoDaFile) {
        let stem = slug(filename);
        let file_section_name = filename.to_string();
        let mut file_section = Section::new(&file_section_name);
        let mut pending = Vec::new();

        // Columns are matched by label, since a mod may reorder them.
        let base_columns = labels(base, TwoDaFile::column_count, TwoDaFile::column_label);
        let modified_columns = labels(modified, TwoDaFile::column_count, TwoDaFile::column_label);

        for name in &base_columns {
            if !modified_columns.contains(name) {
                self.warn(format!(
                    "{filename}: column \"{name}\" was removed, which TSLPatcher cannot express"
                ));
            }
        }

        // New columns first. Their per-row values are indexed by row position,
        // so they have to be settled before any row is appended.
        let mut add_column_index = 0;
        for (column, name) in modified_columns.iter().enumerate() {
            if base_columns.contains(name) {
                continue;
            }
            let section_name = self.unique_name(&format!("{stem}_addcolumn_{add_column_index}"));
            let mut section = Section::new(&section_name);
            section.set("ColumnLabel", name);

            // Only rows that already exist can be addressed by index here;
            // rows this diff appends carry their own value.
            let shared_rows = base.row_count().min(modified.row_count());
            let values: Vec<String> = (0..shared_rows)
                .map(|row| {
                    modified
                        .cell(row, column)
                        .unwrap_or(default_cell())
                        .to_string()
                })
                .collect();

            let default = most_common(&values).unwrap_or_else(|| default_cell().to_string());
            section.set("DefaultValue", escape(&default));

            for (row, value) in values.iter().enumerate() {
                if value != &default {
                    section.set(format!("I{row}"), escape(value));
                }
            }

            file_section.set(format!("AddColumn{add_column_index}"), &section_name);
            pending.push(section);
            add_column_index += 1;
        }

        // Changed cells, considering only columns both tables have.
        let shared_rows = base.row_count().min(modified.row_count());
        let mut change_row_index = 0;
        for row in 0..shared_rows {
            let mut changes: Vec<(String, String)> = Vec::new();

            for name in &base_columns {
                let (Ok(bc), Ok(mc)) = (base.column_by_label(name), modified.column_by_label(name))
                else {
                    continue;
                };
                let (Ok(before), Ok(after)) = (base.cell(row, bc), modified.cell(row, mc)) else {
                    continue;
                };
                if before != after {
                    changes.push((name.clone(), after.to_string()));
                }
            }

            let label_changed = match (base.row_label(row), modified.row_label(row)) {
                (Ok(before), Ok(after)) => before != after,
                _ => false,
            };
            if label_changed {
                self.warn(format!(
                    "{filename}: row {row} was relabelled, which TSLPatcher cannot express"
                ));
            }

            if changes.is_empty() {
                continue;
            }

            let section_name = self.unique_name(&format!("{stem}_changerow_{change_row_index}"));
            let mut section = Section::new(&section_name);
            // The selector has to come first; the patcher reads it before the
            // columns it applies to.
            section.set("RowIndex", row.to_string());
            for (column, value) in changes {
                // A cell that moved is evidence a token pass can act on.
                self.changed_entries
                    .push((section_name.clone(), column.clone()));
                section.set(column, escape(&value));
            }

            file_section.set(format!("ChangeRow{change_row_index}"), &section_name);
            pending.push(section);
            change_row_index += 1;
        }

        if modified.row_count() < base.row_count() {
            self.warn(format!(
                "{filename}: {} row(s) were removed, which TSLPatcher cannot express",
                base.row_count() - modified.row_count()
            ));
        }

        // Appended rows carry a value for every column, new ones included.
        let mut add_row_index = 0;
        for row in base.row_count()..modified.row_count() {
            let section_name = self.unique_name(&format!("{stem}_addrow_{add_row_index}"));
            let mut section = Section::new(&section_name);

            // A row appended without a label is labelled with its own index,
            // so only say otherwise when the mod did.
            if let Ok(label) = modified.row_label(row) {
                if label != row.to_string() {
                    section.set("RowLabel", escape(label));
                }
            }

            for (column, name) in modified_columns.iter().enumerate() {
                let Ok(value) = modified.cell(row, column) else {
                    continue;
                };
                if value != default_cell() {
                    section.set(name, escape(value));
                }
            }

            // Remember where this row lands so a token pass can offer it as a
            // capture target instead of letting other files hardcode the index.
            let label = modified
                .row_label(row)
                .map(str::to_string)
                .unwrap_or_else(|_| row.to_string());
            self.created_rows.push(super::CreatedRow {
                section: section_name.clone(),
                index: row,
                label,
                token: None,
            });

            file_section.set(format!("AddRow{add_row_index}"), &section_name);
            pending.push(section);
            add_row_index += 1;
        }

        if file_section.entries.is_empty() {
            return;
        }

        if !self.twoda_files.iter().any(|f| f == filename) {
            self.twoda_files.push(filename.to_string());
        }
        self.push(file_section);
        for section in pending {
            self.push(section);
        }
    }
}

/// Collect every label a table carries, in order.
fn labels(
    table: &TwoDaFile,
    count: fn(&TwoDaFile) -> usize,
    label: fn(&TwoDaFile, usize) -> kotor_formats::error::Result<&str>,
) -> Vec<String> {
    (0..count(table))
        .filter_map(|i| label(table, i).ok().map(str::to_string))
        .collect()
}

/// The value that appears most often, breaking ties toward the first seen.
fn most_common(values: &[String]) -> Option<String> {
    let mut best: Option<(&String, usize)> = None;
    for value in values {
        let count = values.iter().filter(|v| *v == value).count();
        match best {
            Some((_, best_count)) if count <= best_count => {}
            _ => best = Some((value, count)),
        }
    }
    best.map(|(v, _)| v.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kotor_formats::twoda::build;

    fn table(columns: &[&str], rows: &[(&str, Vec<&str>)]) -> TwoDaFile {
        build(columns, rows)
    }

    #[test]
    fn an_unchanged_table_produces_nothing() {
        let base = table(&["label"], &[("0", vec!["a"])]);
        let modified = table(&["label"], &[("0", vec!["a"])]);

        let mut ini = ChangesIni::new();
        ini.add_twoda("spells.2da", &base, &modified);

        assert!(ini.is_empty());
        assert!(!ini.render().contains("[2DAList]"));
    }

    #[test]
    fn a_changed_cell_becomes_a_change_row_keyed_by_index() {
        let base = table(&["label", "cost"], &[("0", vec!["one", "10"])]);
        let modified = table(&["label", "cost"], &[("0", vec!["one", "99"])]);

        let mut ini = ChangesIni::new();
        ini.add_twoda("spells.2da", &base, &modified);
        let out = ini.render();

        assert!(out.contains("[2DAList]\nTable0=spells.2da\n"));
        assert!(out.contains("[spells.2da]\nChangeRow0=spells_2da_changerow_0\n"));
        // The selector precedes the column it applies to.
        assert!(out.contains("[spells_2da_changerow_0]\nRowIndex=0\ncost=99\n"));
        // An untouched column is left out.
        assert!(!out.contains("label=one"));
    }

    #[test]
    fn an_appended_row_lists_every_non_default_cell() {
        let base = table(&["label", "cost"], &[("0", vec!["one", "10"])]);
        let modified = table(
            &["label", "cost"],
            &[("0", vec!["one", "10"]), ("1", vec!["two", "20"])],
        );

        let mut ini = ChangesIni::new();
        ini.add_twoda("spells.2da", &base, &modified);
        let out = ini.render();

        assert!(out.contains("AddRow0=spells_2da_addrow_0"));
        assert!(out.contains("[spells_2da_addrow_0]\nlabel=two\ncost=20\n"));
        // Label matches the index it will land on, so it is not restated.
        assert!(!out.contains("RowLabel="));
    }

    #[test]
    fn an_appended_row_states_a_label_that_is_not_its_index() {
        let base = table(&["label"], &[("0", vec!["one"])]);
        let modified = table(&["label"], &[("0", vec!["one"]), ("custom", vec!["two"])]);

        let mut ini = ChangesIni::new();
        ini.add_twoda("spells.2da", &base, &modified);

        assert!(ini.render().contains("RowLabel=custom"));
    }

    #[test]
    fn a_blank_cell_is_left_out_of_an_appended_row() {
        let base = table(&["label", "note"], &[("0", vec!["one", "x"])]);
        let modified = table(
            &["label", "note"],
            &[("0", vec!["one", "x"]), ("1", vec!["two", "****"])],
        );

        let mut ini = ChangesIni::new();
        ini.add_twoda("spells.2da", &base, &modified);
        let out = ini.render();

        assert!(out.contains("label=two"));
        // **** is the blank marker; writing it back would be noise.
        assert!(!out.contains("note=****"));
    }

    #[test]
    fn a_new_column_carries_a_default_and_only_the_rows_that_differ() {
        let base = table(
            &["label"],
            &[("0", vec!["a"]), ("1", vec!["b"]), ("2", vec!["c"])],
        );
        let modified = table(
            &["label", "flag"],
            &[
                ("0", vec!["a", "0"]),
                ("1", vec!["b", "7"]),
                ("2", vec!["c", "0"]),
            ],
        );

        let mut ini = ChangesIni::new();
        ini.add_twoda("spells.2da", &base, &modified);
        let out = ini.render();

        assert!(out.contains("AddColumn0=spells_2da_addcolumn_0"));
        assert!(out.contains("ColumnLabel=flag"));
        // 0 is the common value, so only row 1 needs stating.
        assert!(out.contains("DefaultValue=0"));
        assert!(out.contains("I1=7"));
        assert!(!out.contains("I0="));
        assert!(!out.contains("I2="));
    }

    #[test]
    fn a_new_column_does_not_also_report_every_row_as_changed() {
        let base = table(&["label"], &[("0", vec!["a"])]);
        let modified = table(&["label", "flag"], &[("0", vec!["a", "1"])]);

        let mut ini = ChangesIni::new();
        ini.add_twoda("spells.2da", &base, &modified);
        let out = ini.render();

        assert!(out.contains("AddColumn0="));
        assert!(!out.contains("ChangeRow0="));
    }

    #[test]
    fn a_removed_row_is_reported_rather_than_half_patched() {
        let base = table(&["label"], &[("0", vec!["a"]), ("1", vec!["b"])]);
        let modified = table(&["label"], &[("0", vec!["a"])]);

        let mut ini = ChangesIni::new();
        ini.add_twoda("spells.2da", &base, &modified);

        assert!(ini
            .warnings()
            .iter()
            .any(|w| w.contains("row(s) were removed")));
    }

    #[test]
    fn a_removed_column_is_reported() {
        let base = table(&["label", "gone"], &[("0", vec!["a", "x"])]);
        let modified = table(&["label"], &[("0", vec!["a"])]);

        let mut ini = ChangesIni::new();
        ini.add_twoda("spells.2da", &base, &modified);

        assert!(ini
            .warnings()
            .iter()
            .any(|w| w.contains("column \"gone\" was removed")));
    }

    #[test]
    fn a_reordered_column_is_matched_by_label_not_position() {
        let base = table(&["label", "cost"], &[("0", vec!["one", "10"])]);
        let modified = table(&["cost", "label"], &[("0", vec!["10", "one"])]);

        let mut ini = ChangesIni::new();
        ini.add_twoda("spells.2da", &base, &modified);

        // Same data, different column order: nothing to patch.
        assert!(ini.is_empty());
    }
}
