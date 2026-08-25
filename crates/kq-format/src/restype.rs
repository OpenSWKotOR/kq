//! Resource types: the numeric ids the engines store, and the extensions
//! humans type.

use std::fmt;

use serde::{Deserialize, Serialize};

include!("restype_table.rs");

/// A resource type as stored in a KEY/BIF/ERF/RIM entry.
///
/// Unknown ids are preserved rather than rejected — mods and unreleased
/// builds do use ids we have no name for, and dropping them would make the
/// index silently lie about what an archive holds.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ResType(pub u16);

impl ResType {
    pub const INVALID: ResType = ResType(0xFFFF);

    /// The canonical extension, or `None` for an id we have no name for.
    pub fn extension(self) -> Option<&'static str> {
        RESOURCE_TYPES
            .binary_search_by_key(&self.0, |&(id, _)| id)
            .ok()
            .map(|i| RESOURCE_TYPES[i].1)
    }

    /// Look up a type by extension, with or without a leading dot.
    pub fn from_extension(ext: &str) -> Option<ResType> {
        let ext = ext.trim_start_matches('.').to_ascii_lowercase();
        RESOURCE_TYPES
            .iter()
            .find(|&&(_, e)| e == ext)
            .map(|&(id, _)| ResType(id))
    }

    pub fn is_known(self) -> bool {
        self.extension().is_some()
    }

    /// True for the GFF-family types — one parser reads all of them.
    pub fn is_gff(self) -> bool {
        matches!(
            self.extension(),
            Some(
                "gff" | "are" | "git" | "ifo" | "dlg" | "jrl" | "fac" | "gui" | "pth" | "itp"
                    | "utc" | "utd" | "ute" | "uti" | "utm" | "utp" | "uts" | "utt" | "utw"
                    | "bic" | "btc" | "btd" | "bte" | "bti" | "btm" | "btp" | "bts" | "btt"
                    | "gic" | "utg" | "btg" | "ptm" | "ptt" | "uta" | "utx" | "res"
            )
        )
    }

    /// True for types that are already plain text on disk.
    pub fn is_plain_text(self) -> bool {
        matches!(
            self.extension(),
            Some("txt" | "nss" | "lyt" | "vis" | "txi" | "ini" | "xml" | "json" | "2da_csv")
        )
    }

    /// True for the container types a walker should descend into.
    pub fn is_container(self) -> bool {
        matches!(self.extension(), Some("erf" | "mod" | "sav" | "hak" | "rim" | "bif" | "key"))
    }
}

impl fmt::Display for ResType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.extension() {
            Some(ext) => f.write_str(ext),
            None => write!(f, "type{}", self.0),
        }
    }
}

impl fmt::Debug for ResType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ResType({}, {})", self.0, self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_sorted_for_binary_search() {
        assert!(RESOURCE_TYPES.windows(2).all(|w| w[0].0 < w[1].0));
    }

    #[test]
    fn round_trips_common_types() {
        for ext in ["utc", "2da", "tlk", "mdl", "tpc", "ncs", "dlg"] {
            let rt = ResType::from_extension(ext).expect(ext);
            assert_eq!(rt.extension(), Some(ext));
        }
    }

    #[test]
    fn unknown_ids_survive() {
        let rt = ResType(31337);
        assert!(!rt.is_known());
        assert_eq!(rt.to_string(), "type31337");
    }

    #[test]
    fn gff_family_recognized() {
        assert!(ResType::from_extension("utc").unwrap().is_gff());
        assert!(ResType::from_extension("dlg").unwrap().is_gff());
        assert!(!ResType::from_extension("2da").unwrap().is_gff());
    }
}
