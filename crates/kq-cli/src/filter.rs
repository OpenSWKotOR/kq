//! Selecting resources by name, type, module and source — the criteria `ls`,
//! `grep` and `winners` all share.

use anyhow::Result;
use kq_format::ResType;
use kq_index::Index;

use crate::glob;

#[derive(clap::Args, Default)]
pub struct Filter {
    /// Only this resource type, e.g. `utc`, `2da`, `dlg`. Repeatable.
    #[arg(short = 't', long = "type", value_name = "EXT")]
    pub types: Vec<String>,

    /// Only resources from this module root, e.g. `danm13`. Repeatable.
    #[arg(short = 'm', long = "module", value_name = "ROOT")]
    pub modules: Vec<String>,

    /// Only this source kind: override, module-mod, module-rim, lips,
    /// texturepack, rims, stream, chitin, talktable.
    #[arg(short = 's', long = "source", value_name = "KIND")]
    pub sources: Vec<String>,
}

impl Filter {
    fn types(&self) -> Result<Vec<ResType>> {
        let types: Vec<ResType> = self
            .types
            .iter()
            .filter_map(|t| ResType::from_extension(t))
            .collect();
        if types.len() != self.types.len() {
            let bad: Vec<&str> = self
                .types
                .iter()
                .filter(|t| ResType::from_extension(t).is_none())
                .map(String::as_str)
                .collect();
            anyhow::bail!("unknown resource type(s): {}", bad.join(", "));
        }
        Ok(types)
    }

    /// Every resource index matching `pattern` and this filter, sorted by
    /// name then precedence — the order every command agrees on.
    pub fn select(&self, index: &Index, pattern: &str) -> Result<Vec<u32>> {
        let want_types = self.types()?;
        let mut selected: Vec<u32> = Vec::new();

        for (i, r) in index.resources.iter().enumerate() {
            if !glob::matches(pattern, &r.resref) {
                continue;
            }
            if !want_types.is_empty() && !want_types.contains(&r.restype) {
                continue;
            }
            let source = index.source(r);
            if !self.sources.is_empty()
                && !self
                    .sources
                    .iter()
                    .any(|s| s.eq_ignore_ascii_case(source.kind.as_str()))
            {
                continue;
            }
            if !self.modules.is_empty() {
                let Some(root) = source.module_root.as_deref() else {
                    continue;
                };
                if !self.modules.iter().any(|m| m.eq_ignore_ascii_case(root)) {
                    continue;
                }
            }
            selected.push(i as u32);
        }

        selected.sort_by(|&a, &b| {
            let (ra, rb) = (&index.resources[a as usize], &index.resources[b as usize]);
            ra.resref
                .cmp(&rb.resref)
                .then(ra.restype.cmp(&rb.restype))
                .then(
                    index.sources[ra.source as usize]
                        .precedence
                        .cmp(&index.sources[rb.source as usize].precedence),
                )
        });
        Ok(selected)
    }

    /// Keep only the highest-precedence copy of each (resref, type).
    pub fn dedup_winners(index: &Index, selected: &mut Vec<u32>) {
        selected.dedup_by(|&mut a, &mut b| {
            let (ra, rb) = (&index.resources[a as usize], &index.resources[b as usize]);
            ra.resref == rb.resref && ra.restype == rb.restype
        });
    }
}
