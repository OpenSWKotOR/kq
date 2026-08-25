//! Building the resource index for an installation.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use kq_format::{bif, erf, key, restype::ResType, rim, Entry};

use crate::discover::Install;
use crate::game::Game;
use crate::source::{Source, SourceKind};

/// Bumped whenever the on-disk index layout or parsing changes. A stale cache
/// is then a miss rather than a wrong answer.
pub const SCHEMA_VERSION: u32 = 3;

/// What `root` actually points at. Every command works the same way
/// regardless — this is metadata for display, not a second code path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RootKind {
    /// A full KotOR installation rooted at a `chitin.key`.
    Install,
    /// A single standalone ERF/RIM/MOD/SAV file.
    Capsule,
    /// A directory of loose resource files, not itself an installation.
    Folder,
    /// One resource file, indexed as a container of exactly one entry.
    File,
}

/// One resource, resolved to the exact bytes on disk that hold it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Resource {
    pub resref: String,
    pub restype: ResType,
    /// Index into [`Index::files`].
    pub file: u32,
    pub offset: u64,
    pub size: u64,
    /// Index into [`Index::sources`].
    pub source: u32,
}

impl Resource {
    pub fn filename(&self) -> String {
        match self.restype.extension() {
            Some(ext) => format!("{}.{}", self.resref, ext),
            None => format!("{}.type{}", self.resref, self.restype.0),
        }
    }
}

/// Every resource an installation can load, and where each one lives.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Index {
    pub schema: u32,
    pub root: PathBuf,
    pub kind: RootKind,
    pub game: Game,
    pub fingerprint: u64,
    pub files: Vec<PathBuf>,
    pub sources: Vec<Source>,
    pub resources: Vec<Resource>,
    /// Problems found while indexing. Reported, never fatal — one corrupt
    /// archive should not make the other 300 unreadable.
    pub warnings: Vec<String>,
    #[serde(skip)]
    lookup: HashMap<String, Vec<u32>>,
}

impl Index {
    pub fn file(&self, r: &Resource) -> &Path {
        &self.files[r.file as usize]
    }

    pub fn source(&self, r: &Resource) -> &Source {
        &self.sources[r.source as usize]
    }

    /// Every resource with this ResRef, best match first.
    ///
    /// Returning the whole chain rather than only the winner is deliberate:
    /// "which copy of this actually loads, and what is it shadowing" is the
    /// question people get wrong about KotOR.
    pub fn lookup(&self, resref: &str) -> &[u32] {
        self.lookup
            .get(&resref.to_ascii_lowercase())
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// The resource the engine would load for this ResRef.
    pub fn resolve(&self, resref: &str, restype: Option<ResType>) -> Option<&Resource> {
        self.lookup(resref)
            .iter()
            .map(|&i| &self.resources[i as usize])
            .find(|r| restype.is_none_or(|t| r.restype == t))
    }

    /// Rebuild the ResRef lookup. Called after building and after loading a
    /// cached index, since the map is derived state and is not persisted.
    pub fn reindex(&mut self) {
        let mut order: Vec<u32> = (0..self.resources.len() as u32).collect();
        let prec: Vec<u32> = self
            .resources
            .iter()
            .map(|r| self.sources[r.source as usize].precedence)
            .collect();
        order.sort_by_key(|&i| (prec[i as usize], i));

        let mut lookup: HashMap<String, Vec<u32>> = HashMap::with_capacity(self.resources.len());
        for i in order {
            lookup
                .entry(self.resources[i as usize].resref.clone())
                .or_default()
                .push(i);
        }
        self.lookup = lookup;
    }

    pub fn module_roots(&self) -> Vec<&str> {
        let mut roots: Vec<&str> = self
            .sources
            .iter()
            .filter_map(|s| s.module_root.as_deref())
            .collect();
        roots.sort_unstable();
        roots.dedup();
        roots
    }

    /// Path of the backing file, relative to the install (or capsule) root.
    pub fn rel_file(&self, r: &Resource) -> String {
        rel_to_root(&self.root, self.file(r))
    }

    /// Where a human should look: install-relative, with archives as folders.
    ///
    /// `modules/end_m01aa.mod/m01aa.git`, `data/templates.bif/c_drdg.utc`,
    /// `Override/g_assassindrd01.utc`. A `.mod` and the `*_s.rim` / `*_dlg.erf`
    /// trio share a [`Source::module_root`]; the `.mod` still wins on name
    /// collisions because its precedence is lower.
    pub fn virt_path(&self, r: &Resource) -> String {
        virtual_path(&self.root, self.file(r), &r.filename())
    }
}

/// Read a file, memory-mapping it when it is large enough to be worth the
/// syscall. Container headers sit at the front and the tables near it, so a
/// mapped 400 MB texture pack costs a handful of pages, not 400 MB.
fn map_file(path: &Path) -> Result<MappedFile> {
    let file =
        std::fs::File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    if len > 1 << 20 {
        // SAFETY: the index is read-only and a concurrent truncation would at
        // worst produce a parse error, which is already handled per-archive.
        let mmap = unsafe { memmap2::Mmap::map(&file) }
            .with_context(|| format!("cannot map {}", path.display()))?;
        Ok(MappedFile::Mapped(mmap))
    } else {
        Ok(MappedFile::Owned(std::fs::read(path)?))
    }
}

enum MappedFile {
    Mapped(memmap2::Mmap),
    Owned(Vec<u8>),
}

impl std::ops::Deref for MappedFile {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            MappedFile::Mapped(m) => m,
            MappedFile::Owned(v) => v,
        }
    }
}

/// Strip module piece suffixes to get the logical module root.
///
/// `danm13.rim` + `danm13_s.rim` + `danm13_dlg.erf` are one composite
/// module. `danm13.mod` is the same root and outranks all three.
pub fn module_root(filename: &str) -> String {
    let stem = filename
        .rsplit_once('.')
        .map(|(s, _)| s)
        .unwrap_or(filename)
        .to_ascii_lowercase();
    for suffix in ["_dlg", "_adx", "_s", "_a"] {
        if let Some(base) = stem.strip_suffix(suffix) {
            return base.to_string();
        }
    }
    stem
}

/// Collected output of one archive, before it is folded into the index.
struct Parsed {
    source: Source,
    entries: Vec<Entry>,
    warning: Option<String>,
}

/// Build the index for an installation by reading every container header.
pub fn build(install: &Install) -> Result<Index> {
    let mut warnings = Vec::new();
    let mut sources: Vec<Source> = Vec::new();
    let mut resources: Vec<Resource> = Vec::new();
    let mut files: Vec<PathBuf> = Vec::new();
    let mut file_ids: HashMap<PathBuf, u32> = HashMap::new();

    let mut intern = |files: &mut Vec<PathBuf>, p: &Path| -> u32 {
        if let Some(&id) = file_ids.get(p) {
            return id;
        }
        let id = files.len() as u32;
        files.push(p.to_path_buf());
        file_ids.insert(p.to_path_buf(), id);
        id
    };

    // ---- chitin.key + the BIFs it indexes -------------------------------
    {
        let data = map_file(&install.chitin)?;
        let key = key::Key::parse(&data, &install.chitin, &install.root)
            .with_context(|| format!("reading {}", install.chitin.display()))?;

        let tables: Vec<(Vec<bif::BifResource>, Option<String>)> = key
            .bifs
            .par_iter()
            .map(
                |b| match map_file(&b.path).and_then(|d| Ok(bif::read_table(&d, &b.path)?)) {
                    Ok(t) => (t, None),
                    Err(e) => (Vec::new(), Some(format!("{}: {e}", b.name))),
                },
            )
            .collect();

        let mut bif_sources = Vec::with_capacity(key.bifs.len());
        for (i, b) in key.bifs.iter().enumerate() {
            if let Some(w) = &tables[i].1 {
                warnings.push(w.clone());
            }
            bif_sources.push(sources.len() as u32);
            sources.push(Source {
                kind: SourceKind::Chitin,
                label: b.name.clone(),
                precedence: SourceKind::Chitin.base_precedence() + i as u32,
                module_root: None,
            });
        }

        for k in &key.keys {
            let Some(res) = tables[k.bif_index].0.get(k.resource_index) else {
                continue;
            };
            let file = intern(&mut files, &key.bifs[k.bif_index].path);
            resources.push(Resource {
                resref: k.resref.clone(),
                restype: k.restype,
                file,
                offset: res.offset as u64,
                size: res.size as u64,
                source: bif_sources[k.bif_index],
            });
        }
    }

    // ---- capsule folders -------------------------------------------------
    let mut capsule_jobs: Vec<(SourceKind, PathBuf)> = Vec::new();
    for (dir, kind) in [
        (install.modules.as_ref(), SourceKind::ModuleMod),
        (install.lips.as_ref(), SourceKind::Lips),
        (install.texturepacks.as_ref(), SourceKind::TexturePack),
        (install.rims.as_ref(), SourceKind::Rims),
    ] {
        let Some(dir) = dir else { continue };
        let mut found = list_dir(dir, &mut warnings);
        found.sort();
        for path in found {
            if is_capsule_ext(&path) {
                capsule_jobs.push((kind, path));
            }
        }
    }

    let parsed: Vec<Parsed> = capsule_jobs
        .par_iter()
        .map(|(kind, path)| parse_capsule(*kind, path))
        .collect();

    for p in parsed {
        if let Some(w) = p.warning {
            warnings.push(w);
        }
        if p.entries.is_empty() {
            continue;
        }
        let source_id = sources.len() as u32;
        sources.push(p.source);
        for e in p.entries {
            let file = intern(&mut files, &e.file);
            resources.push(Resource {
                resref: e.resref,
                restype: e.restype,
                file,
                offset: e.offset,
                size: e.size,
                source: source_id,
            });
        }
    }

    // ---- loose-file folders ---------------------------------------------
    for (dir, kind, label) in [
        (
            install.override_dir.as_ref(),
            SourceKind::Override,
            "Override",
        ),
        (install.data.as_ref(), SourceKind::Chitin, "data"),
    ] {
        let Some(dir) = dir else { continue };
        if kind != SourceKind::Override {
            continue; // data/ is covered by chitin.key
        }
        let source_id = sources.len() as u32;
        sources.push(Source {
            kind,
            label: label.to_string(),
            precedence: kind.base_precedence(),
            module_root: None,
        });
        for path in walk_files(dir, &mut warnings) {
            if let Some(r) = loose_resource(&path, source_id, &mut files, &mut file_ids) {
                resources.push(r);
            }
        }
    }

    // ---- talk tables: dialog.tlk / dialogf.tlk at the install root -------
    for path in &install.talk_tables {
        let Some(stem) = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_ascii_lowercase())
        else {
            continue;
        };
        let size = match std::fs::metadata(path) {
            Ok(m) => m.len(),
            Err(e) => {
                warnings.push(format!("{}: {e}", path.display()));
                continue;
            }
        };
        let label = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let source_id = sources.len() as u32;
        sources.push(Source {
            kind: SourceKind::TalkTable,
            label,
            precedence: SourceKind::TalkTable.base_precedence(),
            module_root: None,
        });
        let file = *file_ids.entry(path.clone()).or_insert_with(|| {
            let id = files.len() as u32;
            files.push(path.clone());
            id
        });
        resources.push(Resource {
            resref: stem,
            restype: ResType::from_extension("tlk").expect("tlk is a known type"),
            file,
            offset: 0,
            size,
            source: source_id,
        });
    }

    for dir in &install.streams {
        let label = dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let source_id = sources.len() as u32;
        sources.push(Source {
            kind: SourceKind::Stream,
            label,
            precedence: SourceKind::Stream.base_precedence(),
            module_root: None,
        });
        for path in walk_files(dir, &mut warnings) {
            if let Some(r) = loose_resource(&path, source_id, &mut files, &mut file_ids) {
                resources.push(r);
            }
        }
    }

    let mut index = Index {
        schema: SCHEMA_VERSION,
        root: install.root.clone(),
        kind: RootKind::Install,
        game: install.game,
        fingerprint: crate::cache::fingerprint(install),
        files,
        sources,
        resources,
        warnings,
        lookup: HashMap::new(),
    };
    index.reindex();
    Ok(index)
}

/// Index a single standalone ERF/RIM/MOD/SAV file with no install around it.
///
/// Every entry gets [`SourceKind::Loose`] at precedence 0 — there is nothing
/// else to shadow or be shadowed by.
pub fn build_capsule(path: &Path) -> Result<Index> {
    let data = map_file(path)?;
    let entries = read_capsule_entries(&data, path)?;
    let label = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();

    let mut files = Vec::new();
    let mut file_ids: HashMap<PathBuf, u32> = HashMap::new();
    let sources = vec![Source {
        kind: SourceKind::Loose,
        label,
        precedence: 0,
        module_root: None,
    }];
    let resources = entries
        .into_iter()
        .map(|e| {
            let file = *file_ids.entry(e.file.clone()).or_insert_with(|| {
                let id = files.len() as u32;
                files.push(e.file.clone());
                id
            });
            Resource {
                resref: e.resref,
                restype: e.restype,
                file,
                offset: e.offset,
                size: e.size,
                source: 0,
            }
        })
        .collect();

    let mut index = Index {
        schema: SCHEMA_VERSION,
        root: path.to_path_buf(),
        kind: RootKind::Capsule,
        game: Game::K1,
        fingerprint: 0,
        files,
        sources,
        resources,
        warnings: Vec::new(),
        lookup: HashMap::new(),
    };
    index.reindex();
    Ok(index)
}

fn read_capsule_entries(data: &[u8], path: &Path) -> Result<Vec<Entry>> {
    if erf::sniff(data) {
        Ok(erf::read_entries(data, path)?)
    } else if rim::sniff(data) {
        Ok(rim::read_entries(data, path)?)
    } else {
        anyhow::bail!("{}: not an ERF or RIM archive", path.display())
    }
}

/// Index a directory of loose resource files that is not itself an
/// installation. Recurses, same as `Override/` inside a real install.
pub fn build_folder(dir: &Path) -> Result<Index> {
    let mut warnings = Vec::new();
    let mut files = Vec::new();
    let mut file_ids: HashMap<PathBuf, u32> = HashMap::new();
    let label = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| dir.display().to_string());
    let sources = vec![Source {
        kind: SourceKind::Loose,
        label,
        precedence: 0,
        module_root: None,
    }];

    let resources = walk_files(dir, &mut warnings)
        .iter()
        .filter_map(|p| loose_resource(p, 0, &mut files, &mut file_ids))
        .collect();

    let mut index = Index {
        schema: SCHEMA_VERSION,
        root: dir.to_path_buf(),
        kind: RootKind::Folder,
        game: Game::K1,
        fingerprint: 0,
        files,
        sources,
        resources,
        warnings,
        lookup: HashMap::new(),
    };
    index.reindex();
    Ok(index)
}

/// Index one loose resource file as a one-entry container, so every command
/// works on a lone `.utc` the same way it works on a whole install.
pub fn build_single_file(path: &Path) -> Result<Index> {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("{}: no file name", path.display()))?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    let restype = ResType::from_extension(ext).ok_or_else(|| {
        anyhow::anyhow!("{}: unrecognized resource type '.{ext}'", path.display())
    })?;
    let size = std::fs::metadata(path)
        .with_context(|| format!("cannot stat {}", path.display()))?
        .len();
    let label = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();

    let mut index = Index {
        schema: SCHEMA_VERSION,
        root: path.to_path_buf(),
        kind: RootKind::File,
        game: Game::K1,
        fingerprint: 0,
        files: vec![path.to_path_buf()],
        sources: vec![Source {
            kind: SourceKind::Loose,
            label,
            precedence: 0,
            module_root: None,
        }],
        resources: vec![Resource {
            resref: stem,
            restype,
            file: 0,
            offset: 0,
            size,
            source: 0,
        }],
        warnings: Vec::new(),
        lookup: HashMap::new(),
    };
    index.reindex();
    Ok(index)
}

fn is_capsule_ext(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("mod" | "rim" | "erf" | "sav" | "hak")
    )
}

fn parse_capsule(kind: SourceKind, path: &Path) -> Parsed {
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();

    // Inside modules/, a `.mod` outranks the `.rim` trio for the same root.
    let kind = if kind == SourceKind::ModuleMod && ext != "mod" {
        SourceKind::ModuleRim
    } else {
        kind
    };
    let module_root =
        matches!(kind, SourceKind::ModuleMod | SourceKind::ModuleRim).then(|| module_root(&name));

    let source = Source {
        kind,
        label: name.clone(),
        precedence: kind.base_precedence() + texture_pack_rank(&name),
        module_root,
    };

    match map_file(path) {
        Ok(data) => {
            let parsed = if erf::sniff(&data) {
                erf::read_entries(&data, path)
            } else if rim::sniff(&data) {
                rim::read_entries(&data, path)
            } else {
                return Parsed {
                    source,
                    entries: Vec::new(),
                    warning: Some(format!("{name}: not an ERF or RIM archive; skipped")),
                };
            };
            match parsed {
                Ok(entries) => Parsed {
                    source,
                    entries,
                    warning: None,
                },
                Err(e) => Parsed {
                    source,
                    entries: Vec::new(),
                    warning: Some(e.to_string()),
                },
            }
        }
        Err(e) => Parsed {
            source,
            entries: Vec::new(),
            warning: Some(e.to_string()),
        },
    }
}

/// Texture packs are searched high-to-low quality; keep that order stable.
fn texture_pack_rank(name: &str) -> u32 {
    match name.to_ascii_lowercase().as_str() {
        "swpc_tex_tpa.erf" => 0,
        "swpc_tex_tpb.erf" => 1,
        "swpc_tex_tpc.erf" => 2,
        "swpc_tex_gui.erf" => 3,
        _ => 0,
    }
}

fn loose_resource(
    path: &Path,
    source_id: u32,
    files: &mut Vec<PathBuf>,
    file_ids: &mut HashMap<PathBuf, u32>,
) -> Option<Resource> {
    let stem = path.file_stem()?.to_string_lossy().to_ascii_lowercase();
    let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
    let restype = ResType::from_extension(&ext)?;
    let size = std::fs::metadata(path).ok()?.len();

    let file = *file_ids.entry(path.to_path_buf()).or_insert_with(|| {
        let id = files.len() as u32;
        files.push(path.to_path_buf());
        id
    });
    Some(Resource {
        resref: stem,
        restype,
        file,
        offset: 0,
        size,
        source: source_id,
    })
}

fn list_dir(dir: &Path, warnings: &mut Vec<String>) -> Vec<PathBuf> {
    match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .collect(),
        Err(e) => {
            warnings.push(format!("{}: {e}", dir.display()));
            Vec::new()
        }
    }
}

/// Recursive file walk. `Override/` is commonly organized into subfolders by
/// mod authors, and the engine reads all of them.
fn walk_files(dir: &Path, warnings: &mut Vec<String>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        match std::fs::read_dir(&current) {
            Ok(entries) => {
                for e in entries.flatten() {
                    let p = e.path();
                    match e.file_type() {
                        Ok(t) if t.is_dir() => stack.push(p),
                        Ok(t) if t.is_file() => out.push(p),
                        _ => {}
                    }
                }
            }
            Err(e) => warnings.push(format!("{}: {e}", current.display())),
        }
    }
    out.sort();
    out
}

pub fn is_archive_path(file: &Path) -> bool {
    matches!(
        file.extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("mod" | "erf" | "rim" | "bif" | "hak" | "sav")
    )
}

fn rel_to_root(root: &Path, file: &Path) -> String {
    file.strip_prefix(root)
        .unwrap_or(file)
        .to_string_lossy()
        .replace('\\', "/")
}

pub fn virtual_path(root: &Path, file: &Path, inner_name: &str) -> String {
    let rel = rel_to_root(root, file);
    if is_archive_path(file) {
        if rel.is_empty() {
            inner_name.to_string()
        } else {
            format!("{rel}/{inner_name}")
        }
    } else {
        rel
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn composite_module_pieces_share_a_root() {
        assert_eq!(module_root("end_m01aa.mod"), "end_m01aa");
        assert_eq!(module_root("end_m01aa.rim"), "end_m01aa");
        assert_eq!(module_root("end_m01aa_s.rim"), "end_m01aa");
        assert_eq!(module_root("end_m01aa_dlg.erf"), "end_m01aa");
        assert_eq!(module_root("danm13_s.RIM"), "danm13");
    }

    #[test]
    fn virt_path_treats_archives_as_folders() {
        let root = PathBuf::from("/game");
        assert_eq!(
            virtual_path(&root, Path::new("/game/modules/end_m01aa.mod"), "m01aa.git"),
            "modules/end_m01aa.mod/m01aa.git"
        );
        assert_eq!(
            virtual_path(
                &root,
                Path::new("/game/modules/end_m01aa_s.rim"),
                "end_trask.utc"
            ),
            "modules/end_m01aa_s.rim/end_trask.utc"
        );
        assert_eq!(
            virtual_path(&root, Path::new("/game/data/templates.bif"), "c_drdg.utc"),
            "data/templates.bif/c_drdg.utc"
        );
        assert_eq!(
            virtual_path(
                &root,
                Path::new("/game/Override/g_assassindrd01.utc"),
                "g_assassindrd01.utc"
            ),
            "Override/g_assassindrd01.utc"
        );
    }
}
