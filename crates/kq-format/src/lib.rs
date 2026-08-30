//! Readers for the BioWare Aurora/Odyssey formats KotOR ships.
//!
//! Every reader takes a byte slice plus the path it came from, so callers can
//! memory-map once and parse without copying, and so errors name their file.

pub mod bif;
pub mod bwm;
pub mod changes;
pub mod container;
pub mod delta;
pub mod erf;
pub mod error;
pub mod gff;
pub mod key;
pub mod lip;
pub mod ltr;
pub mod mdl;
pub mod ncs;
pub mod ncs_actions;
pub mod reader;
pub mod restype;
pub mod rim;
mod shared;
pub mod ssf;
pub mod text;
pub mod tlk;
pub mod tpc;
pub mod twoda;
pub mod wav;

pub use container::{ContainerKind, Entry};
pub use error::{FormatError, Result};
pub use restype::ResType;

use std::path::Path;

/// Identify a container from its first bytes.
///
/// Extension is ignored: KotOR installs ship `.mod` files that are ERFs,
/// `.sav` files that are ERFs, and mods rename archives freely.
pub fn sniff(data: &[u8]) -> Option<ContainerKind> {
    if data.len() < 8 {
        return None;
    }
    if data.starts_with(b"KEY ") {
        Some(ContainerKind::Key)
    } else if data.starts_with(b"BIFF") {
        Some(ContainerKind::Bif)
    } else if erf::sniff(data) {
        Some(ContainerKind::Erf)
    } else if rim::sniff(data) {
        Some(ContainerKind::Rim)
    } else {
        None
    }
}

/// Read the entries of any ERF- or RIM-family container.
///
/// KEY and BIF are excluded because neither is self-describing: a KEY needs
/// its BIFs to resolve sizes, and a BIF has no names of its own.
pub fn read_container_entries(data: &[u8], path: &Path) -> Result<Vec<Entry>> {
    match sniff(data) {
        Some(ContainerKind::Erf) => erf::read_entries(data, path),
        Some(ContainerKind::Rim) => rim::read_entries(data, path),
        Some(ContainerKind::Bif) => Ok(bif::read_table(data, path)?
            .into_iter()
            .enumerate()
            .map(|(i, res)| Entry {
                resref: format!("{i}"),
                restype: res.restype,
                file: path.to_path_buf(),
                offset: res.offset as u64,
                size: res.size as u64,
            })
            .collect()),
        Some(ContainerKind::Key) => Err(FormatError::Malformed {
            path: path.to_path_buf(),
            message: "KEY files index BIFs; read them with kq_format::key::Key::parse".into(),
        }),
        None => Err(FormatError::BadSignature {
            path: path.to_path_buf(),
            expected: "ERF/MOD/SAV/HAK/RIM/BIF/KEY",
            found: String::from_utf8_lossy(&data[..8.min(data.len())]).into_owned(),
        }),
    }
}
