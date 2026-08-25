//! The shape every BioWare container reduces to: a list of entries, each
//! naming a resource and where its bytes live.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::restype::ResType;

/// One resource inside a container.
///
/// `offset`/`size` are relative to `file`, so an entry is enough to read the
/// bytes without re-parsing the container it came from. For KEY entries
/// `file` is the BIF the key points at, not chitin.key itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Lowercase ResRef, at most 16 characters.
    pub resref: String,
    pub restype: ResType,
    /// The file the bytes actually live in.
    pub file: PathBuf,
    pub offset: u64,
    pub size: u64,
}

impl Entry {
    /// `name.ext` — how humans and `rg` refer to the resource.
    pub fn filename(&self) -> String {
        match self.restype.extension() {
            Some(ext) => format!("{}.{}", self.resref, ext),
            None => format!("{}.type{}", self.resref, self.restype.0),
        }
    }
}

/// What kind of container a path turned out to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContainerKind {
    Key,
    Bif,
    Erf,
    Rim,
}

impl ContainerKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ContainerKind::Key => "key",
            ContainerKind::Bif => "bif",
            ContainerKind::Erf => "erf",
            ContainerKind::Rim => "rim",
        }
    }
}
