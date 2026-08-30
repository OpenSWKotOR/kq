//! Work out the `[TLKList]` instructions that add a mod's new strings.
//!
//! Only appends are generated. `StrRef<token>=<line in append.tlk>` is the one
//! form both the original Delphi patcher and its successors accept: Delphi has
//! no replace-by-file path at all — `ReplaceEntry` exists in `UTLKFile.pas` but
//! nothing ever calls it — so `Replace<n>=` and the inline `<n>\Text=` keys are
//! later additions. Generated output sticks to the common subset.
//!
//! The caller writes the returned entries to `append.tlk` in `tslpatchdata`.
//! Token numbers here start at 0, unlike `2DAMEMORY`: a `StrRef` token is a
//! lookup key rather than a slot in a fixed array, and the contract's own
//! example opens with `StrRef0=0`.

use kotor_formats::tlk::{TlkEntry, TlkFile};

use super::{ChangesIni, CreatedStrRef};

impl ChangesIni {
    /// Add the instructions that append `modified`'s new strings.
    ///
    /// Returns the entries to write to `append.tlk`, in order. An empty result
    /// means the two tables carry the same strings.
    pub fn add_tlk(&mut self, base: &TlkFile, modified: &TlkFile) -> Vec<TlkEntry> {
        let mut appended = Vec::new();

        for (index, entry) in modified.entries().iter().enumerate() {
            match base.entry(index) {
                // A line both tables carry, but whose text the mod rewrote.
                // Rewriting an existing line is a replace, and the original
                // patcher cannot do it, so say so rather than pretend.
                Some(before) if !before.same_content(entry) => {
                    self.warn(format!(
                        "dialog.tlk: line {index} was rewritten, which TSLPatcher cannot express"
                    ));
                }
                Some(_) => {}
                None => {
                    let token = self.tlk_tokens.len() as u32;
                    let append_index = appended.len();
                    self.tlk_tokens.push((token, append_index));
                    self.created_strrefs.push(CreatedStrRef {
                        token,
                        resulting_strref: index as u32,
                    });
                    appended.push(entry.clone());
                }
            }
        }

        if modified.count() < base.count() {
            self.warn(format!(
                "dialog.tlk: {} line(s) were removed, which TSLPatcher cannot express",
                base.count() - modified.count()
            ));
        }

        appended
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(lines: &[&str]) -> TlkFile {
        let mut tlk = TlkFile::new_file();
        for text in lines {
            tlk.add_entry(TlkEntry {
                flags: 1,
                text: (*text).to_string(),
                ..TlkEntry::default()
            })
            .unwrap();
        }
        tlk
    }

    #[test]
    fn an_unchanged_table_produces_nothing() {
        let base = table(&["one", "two"]);
        let modified = table(&["one", "two"]);

        let mut ini = ChangesIni::new();
        let appended = ini.add_tlk(&base, &modified);

        assert!(appended.is_empty());
        assert!(!ini.render().contains("[TLKList]"));
    }

    #[test]
    fn new_lines_become_strref_tokens_over_an_append_file() {
        let base = table(&["one"]);
        let modified = table(&["one", "two", "three"]);

        let mut ini = ChangesIni::new();
        let appended = ini.add_tlk(&base, &modified);

        assert_eq!(appended.len(), 2);
        assert_eq!(appended[0].text, "two");
        assert_eq!(appended[1].text, "three");

        let out = ini.render();
        // Token n maps to line n of append.tlk, not to the game's index.
        assert!(out.contains("[TLKList]\nStrRef0=0\nStrRef1=1\n"));
    }

    #[test]
    fn a_rewritten_line_is_reported_rather_than_appended() {
        let base = table(&["one", "two"]);
        let modified = table(&["one", "CHANGED"]);

        let mut ini = ChangesIni::new();
        let appended = ini.add_tlk(&base, &modified);

        assert!(appended.is_empty());
        assert!(ini
            .warnings()
            .iter()
            .any(|w| w.contains("line 1 was rewritten")));
    }

    #[test]
    fn a_removed_line_is_reported() {
        let base = table(&["one", "two"]);
        let modified = table(&["one"]);

        let mut ini = ChangesIni::new();
        ini.add_tlk(&base, &modified);

        assert!(ini
            .warnings()
            .iter()
            .any(|w| w.contains("line(s) were removed")));
    }
}
