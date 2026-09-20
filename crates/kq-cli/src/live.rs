//! Live mention graph: catalog every ResRef, then keep only what the
//! engine can actually reach.
//!
//! Install mode seeds from hardcoded tables, talk files, scripts, starting
//! modules, and `StartingModule` in the ini — not from every folder under
//! `modules/` or `rims/`. A pair of templates that only name each other
//! stays dead. Talk-table rows count as used only when a *reachable*
//! GFF / 2DA / SSF cites them.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::Mutex;

use anyhow::Result;
use kq_format::{tlk, ResType};
use kq_index::{Index, RootKind};
use rayon::prelude::*;
use serde_json::Value as J;

use crate::filter::Filter;
use crate::read;
use crate::render::{self, Decoded};

/// Types whose inbound references we often cannot see. Left out of leftover
/// *resource* reports unless `--assets` or an explicit `-t` is given.
pub const ASSET_EXTS: &[&str] = &[
    "tpc", "tga", "dds", "mdl", "mdx", "wav", "bmu", "mp3", "txi", "plt", "fxp",
];

/// Never loaded by the engine. Same ResRef as a compiled script does not
/// mean the source file is used.
const SOURCE_EXTS: &[&str] = &["nss"];

/// Area/module entry files. A module *folder* (`end_m01aa`) is not a ResRef;
/// the IFO is always `module.ifo` and the GIT/ARE are often `m01aa.*`.
/// Chitin `lyt`/`vis`/`pth` are reached from ARE via `add_are_layout_edges`, not as module entries.
#[allow(dead_code)]
const MODULE_ENTRY_EXTS: &[&str] = &["ifo", "are", "git", "lyt", "vis", "pth"];

/// Engine-opened talk files and the include the compiler always sees.
/// `nwscript` stays a seed name; `.nss` is not scanned so comments cannot seed the graph.
const ENGINE_ALWAYS: &[&str] = &["dialog", "dialogf", "nwscript"];

/// 2DA filenames that appear as strings in `swkotor.exe` (K1 GOG/Steam) and
/// exist on disk. Intersection, not the C++ `Load2DArrays_*` symbol names —
/// those do not always match the file (`AppearanceSounds` → `appearancesndset`).
const ENGINE_2DAS: &[&str] = &[
    "acbonus",
    "aiscripts",
    "ambientmusic",
    "ambientsound",
    "ammunitiontypes",
    "animations",
    "appearance",
    "appearancesndset",
    "baseitems",
    "bindablekeys",
    "bodybag",
    "camerastyle",
    "categories",
    "classes",
    "classpowergain",
    "cls_spgn_jedi",
    "combatanimations",
    "comptypes",
    "creaturesize",
    "creaturespeed",
    "credits",
    "cursors",
    "damagehitvisual",
    "dialoganimations",
    "difficultyopt",
    "diffsettings",
    "disease",
    "doortypes",
    "droiddischarge",
    "effecticon",
    "encdifficulty",
    "excitedduration",
    "exptable",
    "feat",
    "featgain",
    "feedbacktext",
    "footstepsounds",
    "forceadjust",
    "forceshields",
    "formations",
    "fractionalcr",
    "gameeffects",
    "gamma",
    "gender",
    "genericdoors",
    "globalcat",
    "grenadesnd",
    "guisounds",
    "heads",
    "inventorysnds",
    "iprp_bonuscost",
    "iprp_costtable",
    "iprp_damagecost",
    "iprp_meleecost",
    "iprp_monstcost",
    "iprp_neg5cost",
    "iprp_onhit",
    "iprp_onhitdur",
    "iprp_paramtable",
    "iprp_srcost",
    "itempropdef",
    "itemprops",
    "itemvalue",
    "keymap",
    "lightcolor",
    "loadscreenhints",
    "loadscreens",
    "masterfeats",
    "modulesave",
    "movies",
    "namefilter",
    "names",
    "npc",
    "pazaakdecks",
    "phenotype",
    "placeableobjsnds",
    "placeables",
    "planetary",
    "plot",
    "poison",
    "portraits",
    "prioritygroups",
    "racialtypes",
    "ranges",
    "regeneration",
    "removefxondeath",
    "repadjust",
    "repute",
    "skills",
    "soundprovider",
    "soundset",
    "soundsettype",
    "spells",
    "statescripts",
    "stringtokens",
    "subrace",
    "surfacemat",
    "texpacks",
    "traps",
    "tutorial",
    "upcrystals",
    "upgrade",
    "vfx_persistent",
    "videoeffects",
    "visualeffects",
    "waypoint",
    "weapondischarge",
    "weaponsounds",
    "xptable",
];

/// Module roots hardcoded in K1 `swkotor.exe` (new game, Ebon Hawk, Taris).
const K1_MODULES: &[&str] = &["end_m01aa", "ebo_m12aa", "ebo_m40ad", "tar_m02af"];

/// Default-script / galaxy-map names hardcoded in K1 `swkotor.exe`.
const K1_SCRIPTS: &[&str] = &[
    "k_computer_spike",
    "k_def_blocked01",
    "k_def_damage01",
    "k_def_pathfail01",
    "k_def_spellat01",
    "k_def_userdef01",
    "k_hen_attacked01",
    "k_hen_combend01",
    "k_hen_enter5m",
    "k_hen_exit5m",
    "k_hen_heartbt01",
    "k_hen_leadchng",
    "k_hen_percept01",
    "k_hen_spawn01",
    "k_pend_screenchg",
    "k_repair_part",
    "k_sup_galaxymap",
    "k_sup_gohawk",
    "k_sup_guiopen",
    "k_sup_solo",
    "k_trg_transfail",
];

/// TSL new-game start. Further modules are reached through scripts / GIT.
const K2_MODULES: &[&str] = &["001ebo"];

/// How a decoded resource contributes StrRefs.
#[derive(Clone, Copy)]
enum StrRefMode {
    None,
    /// GFF `CExoLocString` / `CResRef` wrappers: only the `strref` key.
    Gff,
    /// 2DA columns that Holocron-style "find references" treats as talk ids.
    TwoDa,
    /// Every cell in an SSF is a StrRef.
    Ssf,
}

#[derive(Clone, Debug)]
pub struct TlkRow {
    pub strref: i64,
    pub text: String,
    pub sound: String,
}

/// Catalog + mention edges + reachability from engine (or capsule) seeds.
pub struct LiveGraph {
    pub catalog: HashSet<String>,
    pub seeds: Vec<String>,
    pub seed_ids: Vec<u32>,
    pub reachable: HashSet<String>,
    /// Winner resource indices the live walk actually entered.
    pub used_ids: HashSet<u32>,
    /// First parent seen during BFS (child → parent resource id).
    pub parent: HashMap<u32, u32>,
    /// ResRef / module-root tokens each used resource mentions.
    pub edges: HashMap<u32, HashSet<String>>,
    /// Mentioned tokens with no scoped winner (filled by Task 8).
    #[allow(dead_code)]
    pub missing: HashMap<u32, HashSet<String>>,
    /// Module-root → scoped entry resource ids (ifo/are/git/pth).
    #[allow(dead_code)]
    pub module_entries: HashMap<String, Vec<u32>>,
    /// ResRefs of `used_ids`, plus VO names on used talk-table rows.
    pub used: HashSet<String>,
    pub used_strrefs: HashSet<i64>,
    pub tlk: Vec<TlkRow>,
    pub scanned: usize,
}

impl LiveGraph {
    pub fn leftover_strings(&self) -> Vec<&TlkRow> {
        self.tlk
            .iter()
            .filter(|row| !self.used_strrefs.contains(&row.strref))
            .collect()
    }
}

pub fn build(index: &Index) -> Result<LiveGraph> {
    let catalog: HashSet<String> = index.resources.iter().map(|r| r.resref.clone()).collect();
    let module_roots: HashSet<String> = index
        .module_roots()
        .into_iter()
        .map(|s| s.to_ascii_lowercase())
        .collect();

    let winners_map = scoped_winners(index);
    let winners: Vec<u32> = {
        let mut v: Vec<u32> = winners_map.values().copied().collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    let module_entries = module_entry_ids(index, &winners_map);

    let mut scan_ids = winners.clone();
    scan_ids.retain(|&i| is_scan_source(index.resources[i as usize].restype));

    crate::output::warn(format!(
        "catalog {} ResRefs; scanning {} for mentions…",
        catalog.len(),
        scan_ids.len()
    ));

    let hits = Mutex::new(Vec::<(u32, HashSet<String>, HashSet<i64>)>::new());
    scan_ids.par_iter().for_each(|&i| {
        let r = &index.resources[i as usize];
        let Ok(bytes) = read::read(index, r) else {
            return;
        };
        let Ok(decoded) = render::decode_resource(index, r, &bytes) else {
            return;
        };
        let mut mentions = HashSet::new();
        let mut strrefs = HashSet::new();
        collect(
            &decoded,
            &catalog,
            &module_roots,
            &r.resref,
            r.restype.extension(),
            strref_mode(r.restype),
            &mut mentions,
            &mut strrefs,
        );
        if !mentions.is_empty() || !strrefs.is_empty() {
            hits.lock()
                .expect("live scan lock")
                .push((i, mentions, strrefs));
        }
    });

    let mut edges: HashMap<u32, HashSet<String>> = HashMap::new();
    let mut strrefs_by: HashMap<u32, HashSet<i64>> = HashMap::new();
    for (id, mentions, strrefs) in hits.into_inner().expect("live scan lock") {
        if !mentions.is_empty() {
            edges.insert(id, mentions);
        }
        if !strrefs.is_empty() {
            strrefs_by.insert(id, strrefs);
        }
    }
    add_are_layout_edges(index, &winners_map, &mut edges);

    let (seed_labels, seed_ids) = seed_ids(index, &catalog, &winners_map, &module_entries);
    let (used_ids, parent) = bfs(index, &seed_ids, &edges, &winners_map, &module_entries);

    let tlk = load_dialog_tlk(index)?;
    let tlk_len = tlk.len() as i64;
    let mut used_strrefs = HashSet::new();
    for &id in &used_ids {
        if let Some(refs) = strrefs_by.get(&id) {
            for &n in refs {
                if n >= 0 && n < tlk_len {
                    used_strrefs.insert(n);
                }
            }
        }
    }

    let mut used: HashSet<String> = used_ids
        .iter()
        .map(|&i| index.resources[i as usize].resref.clone())
        .collect();
    let reachable = used.clone();
    for row in &tlk {
        if used_strrefs.contains(&row.strref)
            && !row.sound.is_empty()
            && catalog.contains(&row.sound)
        {
            used.insert(row.sound.clone());
        }
    }

    Ok(LiveGraph {
        catalog,
        seeds: seed_labels,
        seed_ids,
        reachable,
        used_ids,
        parent,
        edges,
        missing: HashMap::new(),
        module_entries,
        used,
        used_strrefs,
        tlk,
        scanned: scan_ids.len(),
    })
}

pub fn is_asset(t: ResType) -> bool {
    ASSET_EXTS.contains(&t.extension().unwrap_or(""))
}

/// Types omitted from leftover reports unless `-t` or `--assets` is given.
/// Only `.nss` — the engine loads compiled `.ncs`, not source.
pub fn is_noise(t: ResType) -> bool {
    SOURCE_EXTS.contains(&t.extension().unwrap_or(""))
}

/// `None` = global (override / chitin / etc.); `Some(module_root)` for module-local sources.
pub type Scope = Option<String>;

/// ModuleMod / ModuleRim → Some(module_root); everything else → None (global).
pub fn resource_scope(index: &Index, r: &kq_index::Resource) -> Scope {
    let source = index.source(r);
    match source.kind {
        kq_index::SourceKind::ModuleMod | kq_index::SourceKind::ModuleRim => {
            source.module_root.as_ref().map(|s| s.to_ascii_lowercase())
        }
        _ => None,
    }
}

/// Lowest precedence id per (scope, resref, restype).
pub fn scoped_winners(index: &Index) -> HashMap<(Scope, String, ResType), u32> {
    let mut best: HashMap<(Scope, String, ResType), (u32, u32)> = HashMap::new();
    for (i, r) in index.resources.iter().enumerate() {
        let scope = resource_scope(index, r);
        let key = (scope, r.resref.clone(), r.restype);
        let prec = index.sources[r.source as usize].precedence;
        best.entry(key)
            .and_modify(|(id, p)| {
                if prec < *p || (prec == *p && (i as u32) < *id) {
                    *id = i as u32;
                    *p = prec;
                }
            })
            .or_insert((i as u32, prec));
    }
    best.into_iter().map(|(k, (id, _))| (k, id)).collect()
}

/// Resource ids that win in their own scope (module-local or global).
pub fn scoped_winner_id_set(index: &Index) -> HashSet<u32> {
    scoped_winners(index).into_values().collect()
}

/// Higher-precedence copy in the same scope, or Override of the same
/// `(resref, type)` when `id` is a module-scoped copy.
pub fn shadowed_by(index: &Index, id: u32) -> Option<u32> {
    let r = &index.resources[id as usize];
    let scope = resource_scope(index, r);
    let winners = scoped_winners(index);
    let winner = winners
        .get(&(scope.clone(), r.resref.clone(), r.restype))
        .copied()?;
    if winner == id {
        if let Some(&ov) = winners.get(&(None, r.resref.clone(), r.restype)) {
            if index.source(&index.resources[ov as usize]).kind == kq_index::SourceKind::Override
                && scope.is_some()
            {
                return Some(ov);
            }
        }
        return None;
    }
    Some(winner)
}

pub fn is_shadowed(index: &Index, id: u32) -> bool {
    shadowed_by(index, id).is_some()
}

#[allow(dead_code)] // retained for Task 5 / global fallback tooling
fn all_winners(index: &Index) -> Vec<u32> {
    let mut ids: Vec<u32> = (0..index.resources.len() as u32).collect();
    ids.sort_by(|&a, &b| {
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
    Filter::dedup_winners(index, &mut ids);
    ids
}

fn module_entry_ids(
    index: &Index,
    winners: &HashMap<(Scope, String, ResType), u32>,
) -> HashMap<String, Vec<u32>> {
    let _ = index;
    const ENTRY: &[&str] = &["ifo", "are", "git", "pth"]; // lyt/vis chitin handled in Task 5
    let mut map: HashMap<String, Vec<u32>> = HashMap::new();
    for ((scope, _resref, restype), &id) in winners {
        let Some(root) = scope.as_deref() else {
            continue;
        };
        let Some(ext) = restype.extension() else {
            continue;
        };
        if !ENTRY.contains(&ext) {
            continue;
        }
        map.entry(root.to_string()).or_default().push(id);
    }
    map
}

fn is_scan_source(t: ResType) -> bool {
    if t.is_gff() {
        return true;
    }
    match t.extension() {
        Some("2da" | "ncs" | "ssf" | "mdl") => true,
        Some("nss") => false,
        Some(_) if t.is_plain_text() => true,
        _ => false,
    }
}

fn strref_mode(t: ResType) -> StrRefMode {
    if t.is_gff() {
        return StrRefMode::Gff;
    }
    match t.extension() {
        Some("2da") => StrRefMode::TwoDa,
        Some("ssf") => StrRefMode::Ssf,
        _ => StrRefMode::None,
    }
}

fn seed_ids(
    index: &Index,
    known: &HashSet<String>,
    winners: &HashMap<(Scope, String, ResType), u32>,
    module_entries: &HashMap<String, Vec<u32>>,
) -> (Vec<String>, Vec<u32>) {
    let mut labels = Vec::new();
    let mut ids = Vec::new();

    let push_resref = |name: &str, labels: &mut Vec<String>, ids: &mut Vec<u32>| {
        let name = name.to_ascii_lowercase();
        let list = resolve_in_scope(index, winners, module_entries, &None, &name);
        if !list.is_empty() {
            labels.push(name);
            ids.extend(list);
        }
    };
    let push_module = |name: &str, labels: &mut Vec<String>, ids: &mut Vec<u32>| {
        let name = name.to_ascii_lowercase();
        if let Some(list) = module_entries.get(&name) {
            labels.push(name.clone());
            ids.extend(list.iter().copied());
            return;
        }
        let mut any = false;
        for (i, r) in index.resources.iter().enumerate() {
            if index
                .source(r)
                .module_root
                .as_deref()
                .is_some_and(|m| m.eq_ignore_ascii_case(&name))
            {
                ids.push(i as u32);
                any = true;
            }
        }
        if any {
            labels.push(name);
        } else {
            crate::output::warn(format!("seed module `{name}` unresolved (no resources)"));
        }
    };

    match index.kind {
        RootKind::Install => {
            for name in ENGINE_ALWAYS.iter().chain(ENGINE_2DAS) {
                push_resref(name, &mut labels, &mut ids);
            }
            for name in K1_SCRIPTS {
                push_resref(name, &mut labels, &mut ids);
            }
            let extra: &[&str] = match index.game {
                kq_index::Game::K1 => K1_MODULES,
                kq_index::Game::K2 => K2_MODULES,
            };
            for name in extra {
                push_module(name, &mut labels, &mut ids);
            }
            for name in ini_starting_modules(&index.root) {
                push_module(&name, &mut labels, &mut ids);
                push_resref(&name, &mut labels, &mut ids);
            }
            // Warn once per missing hardcoded resref seed.
            let expected: Vec<&str> = ENGINE_ALWAYS
                .iter()
                .chain(ENGINE_2DAS.iter())
                .chain(K1_SCRIPTS.iter())
                .copied()
                .collect();
            for name in expected {
                let key = name.to_ascii_lowercase();
                if !labels.iter().any(|l| l == &key) {
                    crate::output::warn(format!("seed `{key}` unresolved (no winner)"));
                }
            }
        }
        RootKind::Capsule | RootKind::Folder | RootKind::File => {
            for r in &index.resources {
                if matches!(r.restype.extension(), Some("ifo" | "are" | "git")) {
                    let list = resolve_in_scope(index, winners, module_entries, &None, &r.resref);
                    if !list.is_empty() {
                        labels.push(r.resref.clone());
                        ids.extend(list);
                    }
                }
            }
            if let Some(root) = index
                .resources
                .first()
                .and_then(|r| index.source(r).module_root.as_deref())
            {
                push_module(root, &mut labels, &mut ids);
            }
            if ids.is_empty() {
                for name in known {
                    push_resref(name, &mut labels, &mut ids);
                }
            }
        }
    }

    labels.sort();
    labels.dedup();
    ids.sort_unstable();
    ids.dedup();
    (labels, ids)
}

fn ini_starting_modules(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for name in ["swkotor.ini", "swkotor2.ini", "kotor.ini"] {
        let Ok(text) = std::fs::read_to_string(root.join(name)) else {
            continue;
        };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            if !key.trim().eq_ignore_ascii_case("startingmodule") {
                continue;
            }
            let value = value.trim().trim_matches('"').to_ascii_lowercase();
            if !value.is_empty() && value.len() <= 16 {
                out.push(value);
            }
        }
    }
    out
}

fn load_dialog_tlk(index: &Index) -> Result<Vec<TlkRow>> {
    let Some(tlk_ty) = ResType::from_extension("tlk") else {
        return Ok(Vec::new());
    };
    let Some(r) = index.resolve("dialog", Some(tlk_ty)) else {
        return Ok(Vec::new());
    };
    let bytes = read::read(index, r)?;
    let table = tlk::read(&bytes, Path::new("dialog.tlk"))?;
    Ok(table
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| TlkRow {
            strref: i as i64,
            text: e.text.clone(),
            sound: e.sound.to_ascii_lowercase(),
        })
        .collect())
}

fn resolve_in_scope(
    index: &Index,
    winners: &HashMap<(Scope, String, ResType), u32>,
    module_entries: &HashMap<String, Vec<u32>>,
    scope: &Scope,
    tok: &str,
) -> Vec<u32> {
    let mut out = Vec::new();
    // Collect distinct restypes for this resref from the index.
    let mut types: Vec<ResType> = index
        .lookup(tok)
        .iter()
        .map(|&i| index.resources[i as usize].restype)
        .collect();
    types.sort_by_key(|t| t.0);
    types.dedup();

    for ty in types {
        if let Some(&id) = winners.get(&(None, tok.to_string(), ty)) {
            let kind = index.source(&index.resources[id as usize]).kind;
            if kind == kq_index::SourceKind::Override {
                out.push(id);
                continue;
            }
        }
        if let Some(m) = scope {
            if let Some(&id) = winners.get(&(Some(m.clone()), tok.to_string(), ty)) {
                out.push(id);
                continue;
            }
        }
        if let Some(&id) = winners.get(&(None, tok.to_string(), ty)) {
            out.push(id);
        }
    }
    if let Some(ids) = module_entries.get(tok) {
        out.extend(ids.iter().copied());
    }
    out.sort_unstable();
    out.dedup();
    out
}

fn bfs(
    index: &Index,
    seeds: &[u32],
    edges: &HashMap<u32, HashSet<String>>,
    winners: &HashMap<(Scope, String, ResType), u32>,
    module_entries: &HashMap<String, Vec<u32>>,
) -> (HashSet<u32>, HashMap<u32, u32>) {
    let mut seen = HashSet::new();
    let mut parent = HashMap::new();
    let mut q = VecDeque::new();
    for &id in seeds {
        if seen.insert(id) {
            q.push_back(id);
        }
    }
    while let Some(id) = q.pop_front() {
        let scope = resource_scope(index, &index.resources[id as usize]);
        let Some(tokens) = edges.get(&id) else {
            continue;
        };
        for tok in tokens {
            for j in resolve_in_scope(index, winners, module_entries, &scope, tok) {
                if seen.insert(j) {
                    parent.insert(j, id);
                    q.push_back(j);
                }
            }
        }
    }
    (seen, parent)
}

/// Insert same-resref mentions from each ARE winner onto global `lyt`/`vis`/`pth`
/// when those types exist. Tokenizer still skips bare self-resref; this is the
/// supported path onto chitin layouts.
fn add_are_layout_edges(
    index: &Index,
    winners: &HashMap<(Scope, String, ResType), u32>,
    edges: &mut HashMap<u32, HashSet<String>>,
) {
    let _ = index;
    let are = ResType::from_extension("are").unwrap();
    for ((scope, resref, ty), &id) in winners {
        if *ty != are {
            continue;
        }
        let _ = scope;
        for ext in ["lyt", "vis", "pth"] {
            let Some(layout_ty) = ResType::from_extension(ext) else {
                continue;
            };
            if winners.contains_key(&(None, resref.clone(), layout_ty)) {
                edges.entry(id).or_default().insert(resref.clone());
                break;
            }
        }
    }
}

fn collect(
    decoded: &Decoded,
    known: &HashSet<String>,
    module_roots: &HashSet<String>,
    self_ref: &str,
    self_ext: Option<&str>,
    mode: StrRefMode,
    mentions: &mut HashSet<String>,
    strrefs: &mut HashSet<i64>,
) {
    match decoded {
        Decoded::Value(v) => walk_json(
            v,
            &mut Walk {
                known,
                module_roots,
                self_ref,
                self_ext,
                mode,
                mentions,
                strrefs,
            },
            None,
        ),
        Decoded::Text(s) => take_tokens(s, known, module_roots, self_ref, self_ext, mentions),
        Decoded::Opaque { .. } => {}
    }
}

struct Walk<'a> {
    known: &'a HashSet<String>,
    module_roots: &'a HashSet<String>,
    self_ref: &'a str,
    self_ext: Option<&'a str>,
    mode: StrRefMode,
    mentions: &'a mut HashSet<String>,
    strrefs: &'a mut HashSet<i64>,
}

fn walk_json(v: &J, w: &mut Walk<'_>, key: Option<&str>) {
    match v {
        J::String(s) => {
            take_tokens(
                s,
                w.known,
                w.module_roots,
                w.self_ref,
                w.self_ext,
                w.mentions,
            );
            if matches!(w.mode, StrRefMode::TwoDa) && key.is_some_and(is_strref_column) {
                if let Some(n) = parse_strref(s) {
                    w.strrefs.insert(n);
                }
            }
        }
        J::Number(n) => {
            if let Some(i) = n.as_i64() {
                match w.mode {
                    StrRefMode::Ssf => {
                        w.strrefs.insert(i);
                    }
                    StrRefMode::Gff if key.is_some_and(|k| k.eq_ignore_ascii_case("strref")) => {
                        w.strrefs.insert(i);
                    }
                    StrRefMode::TwoDa if key.is_some_and(is_strref_column) => {
                        w.strrefs.insert(i);
                    }
                    _ => {}
                }
            }
        }
        J::Array(items) => {
            for item in items {
                walk_json(item, w, key);
            }
        }
        J::Object(map) => {
            for (k, val) in map {
                // Do not tokenize object keys (column labels, NCS field names, etc.).
                walk_json(val, w, Some(k));
            }
        }
        _ => {}
    }
}

fn is_strref_column(col: &str) -> bool {
    let c = col.to_ascii_lowercase();
    if c == "_row" || c == "label" {
        return false;
    }
    c == "name"
        || c == "description"
        || c == "desc"
        || c == "text"
        || c == "title"
        || c == "feedback"
        || c == "tooltip"
        || c.contains("strref")
        || c.contains("stringref")
        || c.ends_with("string")
}

fn parse_strref(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() || s == "****" {
        return None;
    }
    let n: i64 = s.parse().ok()?;
    if n < 0 {
        return None;
    }
    Some(n)
}

fn take_tokens(
    s: &str,
    known: &HashSet<String>,
    module_roots: &HashSet<String>,
    self_ref: &str,
    self_ext: Option<&str>,
    out: &mut HashSet<String>,
) {
    let lower = s.to_ascii_lowercase();
    let mut start = None;
    for (i, c) in lower.char_indices() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '.' {
            if start.is_none() {
                start = Some(i);
            }
        } else if let Some(st) = start.take() {
            consider_token(&lower[st..i], known, module_roots, self_ref, self_ext, out);
        }
    }
    if let Some(st) = start {
        consider_token(&lower[st..], known, module_roots, self_ref, self_ext, out);
    }
}

fn consider_token(
    tok: &str,
    known: &HashSet<String>,
    module_roots: &HashSet<String>,
    self_resref: &str,
    self_ext: Option<&str>,
    out: &mut HashSet<String>,
) {
    if tok.is_empty() || tok == "****" {
        return;
    }
    if !tok.is_empty() && tok.chars().all(|c| c.is_ascii_digit()) {
        return;
    }
    // Skip tokenizer noise for *this* resource (bare resref). Synthetic ARE
    // edges are the supported path onto same-resref lyt/vis/pth. Dotted names
    // of a different type (m01aa.lyt while scanning m01aa.are) still count.
    if tok == self_resref {
        return;
    }
    if let Some(ext) = self_ext {
        if tok.eq_ignore_ascii_case(&format!("{self_resref}.{ext}")) {
            return;
        }
    }
    let ident_len = tok
        .rsplit_once('.')
        .map(|(base, _)| base.len())
        .unwrap_or(tok.len());
    if ident_len == 0 || ident_len > 16 {
        return;
    }
    if known.contains(tok) || module_roots.contains(tok) {
        out.insert(tok.to_string());
        return;
    }
    if let Some((base, ext)) = tok.rsplit_once('.') {
        if ResType::from_extension(ext).is_some()
            && (known.contains(base) || module_roots.contains(base))
        {
            out.insert(base.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_two_modules_shared_are() -> Index {
        use serde_json::json;
        let are = ResType::from_extension("are").unwrap().0;
        let ifo = ResType::from_extension("ifo").unwrap().0;
        let ncs = ResType::from_extension("ncs").unwrap().0;
        let mut index: Index = serde_json::from_value(json!({
            "schema": 3,
            "root": "/game",
            "kind": "install",
            "game": "k1",
            "fingerprint": 0,
            "files": [
                "/game/modules/ebo_m12aa.mod",
                "/game/modules/ebo_m40ad.mod",
                "/game/modules/ebo_m12aa_s.rim",
                "/game/Override/shared.ncs",
                "/game/data/scripts.bif"
            ],
            "sources": [
                {"kind":"module-mod","label":"ebo_m12aa.mod","precedence":100,"module_root":"ebo_m12aa"},
                {"kind":"module-mod","label":"ebo_m40ad.mod","precedence":100,"module_root":"ebo_m40ad"},
                {"kind":"module-rim","label":"ebo_m12aa_s.rim","precedence":200,"module_root":"ebo_m12aa"},
                {"kind":"override","label":"Override","precedence":0,"module_root":null},
                {"kind":"chitin","label":"scripts.bif","precedence":700,"module_root":null}
            ],
            "resources": [
                {"resref":"module","restype":ifo,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"module","restype":ifo,"file":1,"offset":0,"size":1,"source":1},
                {"resref":"m12aa","restype":are,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"m12aa","restype":are,"file":1,"offset":0,"size":1,"source":1},
                {"resref":"local","restype":ncs,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"local","restype":ncs,"file":2,"offset":0,"size":1,"source":2},
                {"resref":"shared","restype":ncs,"file":3,"offset":0,"size":1,"source":3},
                {"resref":"shared","restype":ncs,"file":0,"offset":0,"size":1,"source":0}
            ],
            "warnings": []
        }))
        .unwrap();
        index.reindex();
        index
    }

    #[test]
    fn ebo_m40ad_seed_survives_colliding_m12aa_are() {
        let index = fixture_two_modules_shared_are();
        let winners_map = scoped_winners(&index);
        let module_entries = module_entry_ids(&index, &winners_map);
        assert!(module_entries.contains_key("ebo_m12aa"));
        assert!(module_entries.contains_key("ebo_m40ad"));

        let catalog: HashSet<String> = index.resources.iter().map(|r| r.resref.clone()).collect();
        let (labels, ids) = seed_ids(&index, &catalog, &winners_map, &module_entries);
        assert!(labels.iter().any(|s| s == "ebo_m40ad"), "labels={labels:?}");
        assert!(!ids.is_empty());
        // At least one seed id must belong to ebo_m40ad's module source.
        assert!(ids.iter().any(|&i| {
            index
                .source(&index.resources[i as usize])
                .module_root
                .as_deref()
                == Some("ebo_m40ad")
        }));
    }

    #[test]
    fn scoped_winners_keep_per_module_ifo_and_are() {
        let index = fixture_two_modules_shared_are();
        let winners = scoped_winners(&index);
        let ifo = ResType::from_extension("ifo").unwrap();
        let are = ResType::from_extension("are").unwrap();
        let a = winners
            .get(&(Some("ebo_m12aa".into()), "module".into(), ifo))
            .copied();
        let b = winners
            .get(&(Some("ebo_m40ad".into()), "module".into(), ifo))
            .copied();
        assert!(a.is_some() && b.is_some() && a != b);
        let are_a = winners
            .get(&(Some("ebo_m12aa".into()), "m12aa".into(), are))
            .copied();
        let are_b = winners
            .get(&(Some("ebo_m40ad".into()), "m12aa".into(), are))
            .copied();
        assert!(are_a.is_some() && are_b.is_some() && are_a != are_b);
    }

    #[test]
    fn scoped_winners_mod_beats_rim_same_module() {
        let index = fixture_two_modules_shared_are();
        let winners = scoped_winners(&index);
        let ncs = ResType::from_extension("ncs").unwrap();
        let id = *winners
            .get(&(Some("ebo_m12aa".into()), "local".into(), ncs))
            .unwrap();
        assert_eq!(
            index.source(&index.resources[id as usize]).label,
            "ebo_m12aa.mod"
        );
    }

    #[test]
    fn scoped_winners_override_beats_module_in_global_and_lookup() {
        let index = fixture_two_modules_shared_are();
        let winners = scoped_winners(&index);
        let ncs = ResType::from_extension("ncs").unwrap();
        // Global scope winner for "shared" is Override.
        let id = *winners.get(&(None, "shared".into(), ncs)).unwrap();
        assert_eq!(
            index.source(&index.resources[id as usize]).kind,
            kq_index::SourceKind::Override
        );
        // Module still has its own scoped winner.
        assert!(winners.contains_key(&(Some("ebo_m12aa".into()), "shared".into(), ncs)));
    }

    #[test]
    fn tokens_ignore_self_and_stars() {
        let known = HashSet::from(["n_bastila".into(), "k_ai_master".into(), "danm13".into()]);
        let roots = HashSet::new();
        let mut out = HashSet::new();
        take_tokens(
            "Tag=n_bastila Script=k_ai_master ****",
            &known,
            &roots,
            "n_bastila",
            Some("utc"),
            &mut out,
        );
        assert!(out.contains("k_ai_master"));
        assert!(!out.contains("n_bastila"));
        assert!(!out.contains("****"));
    }

    fn fixture_end_m01aa_are_lyt() -> Index {
        let lyt = ResType::from_extension("lyt").unwrap().0;
        let are = ResType::from_extension("are").unwrap().0;
        let ifo = ResType::from_extension("ifo").unwrap().0;
        let mut index: Index = serde_json::from_value(serde_json::json!({
            "schema": 3,
            "root": "/game",
            "kind": "install",
            "game": "k1",
            "fingerprint": 0,
            "files": ["/game/modules/end_m01aa.mod", "/game/data/layouts.bif"],
            "sources": [
                {"kind":"module-mod","label":"end_m01aa.mod","precedence":100,"module_root":"end_m01aa"},
                {"kind":"chitin","label":"layouts.bif","precedence":700,"module_root":null}
            ],
            "resources": [
                {"resref":"module","restype":ifo,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"m01aa","restype":are,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"m01aa","restype":lyt,"file":1,"offset":0,"size":1,"source":1}
            ],
            "warnings": []
        }))
        .unwrap();
        index.reindex();
        index
    }

    #[test]
    fn are_mentions_same_resref_lyt() {
        let index = fixture_end_m01aa_are_lyt();

        let winners = scoped_winners(&index);
        let mut edges = HashMap::new();
        add_are_layout_edges(&index, &winners, &mut edges);
        let are_id = *winners
            .get(&(
                Some("end_m01aa".into()),
                "m01aa".into(),
                ResType::from_extension("are").unwrap(),
            ))
            .unwrap();
        assert!(edges.get(&are_id).unwrap().contains("m01aa"));

        // Tokenize path: scanning ARE text that contains "m01aa.lyt" must keep the token.
        let known = HashSet::from(["m01aa".into()]);
        let roots = HashSet::new();
        let mut out = HashSet::new();
        take_tokens("m01aa.lyt", &known, &roots, "m01aa", Some("are"), &mut out);
        assert!(out.contains("m01aa"));
    }

    #[test]
    fn build_reaches_chitin_lyt_from_reachable_are() {
        let index = fixture_end_m01aa_are_lyt();
        let winners = scoped_winners(&index);
        let lyt_ty = ResType::from_extension("lyt").unwrap();
        let lyt_id = *winners.get(&(None, "m01aa".into(), lyt_ty)).unwrap();

        let graph = build(&index).expect("build");
        assert!(
            graph.used_ids.contains(&lyt_id),
            "global m01aa.lyt must land in used_ids when end_m01aa is seeded; used_ids={:?}",
            graph.used_ids
        );
    }

    #[test]
    fn isolated_cycle_is_not_reachable() {
        use serde_json::json;
        let ncs = ResType::from_extension("ncs").unwrap().0;
        let mut index: Index = serde_json::from_value(json!({
            "schema": 3,
            "root": "/game",
            "kind": "install",
            "game": "k1",
            "fingerprint": 0,
            "files": ["/game/data/scripts.bif"],
            "sources": [
                {"kind":"chitin","label":"scripts.bif","precedence":700,"module_root":null}
            ],
            "resources": [
                {"resref":"seed","restype":ncs,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"dead_a","restype":ncs,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"dead_b","restype":ncs,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"n_endsol01","restype":ncs,"file":0,"offset":0,"size":1,"source":0}
            ],
            "warnings": []
        }))
        .unwrap();
        index.reindex();
        let winners = scoped_winners(&index);
        let module_entries = HashMap::new();
        let mut edges = HashMap::new();
        edges.insert(1, HashSet::from(["dead_b".into()]));
        edges.insert(2, HashSet::from(["dead_a".into()]));
        edges.insert(0, HashSet::from(["n_endsol01".into()]));
        let (seen, _parent) = bfs(&index, &[0], &edges, &winners, &module_entries);
        assert!(seen.contains(&0));
        assert!(seen.contains(&3));
        assert!(!seen.contains(&1));
        assert!(!seen.contains(&2));
    }

    #[test]
    fn module_folder_enters_git_not_shared_ifo_name() {
        use serde_json::json;
        let git = ResType::from_extension("git").unwrap().0;
        let ifo = ResType::from_extension("ifo").unwrap().0;
        let utc = ResType::from_extension("utc").unwrap().0;
        let mut index: Index = serde_json::from_value(json!({
            "schema": 3,
            "root": "/game",
            "kind": "install",
            "game": "k1",
            "fingerprint": 0,
            "files": [
                "/game/modules/end_m01aa.mod",
                "/game/modules/other.mod"
            ],
            "sources": [
                {"kind":"module-mod","label":"end_m01aa.mod","precedence":100,"module_root":"end_m01aa"},
                {"kind":"module-mod","label":"other.mod","precedence":100,"module_root":"other"}
            ],
            "resources": [
                {"resref":"m01aa","restype":git,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"module","restype":ifo,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"end_trask","restype":utc,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"other_mod_utc","restype":utc,"file":1,"offset":0,"size":1,"source":1},
                {"resref":"orphan","restype":utc,"file":1,"offset":0,"size":1,"source":1}
            ],
            "warnings": []
        }))
        .unwrap();
        index.reindex();
        let winners = scoped_winners(&index);
        let module_entries = module_entry_ids(&index, &winners);
        let mut edges = HashMap::new();
        edges.insert(0, HashSet::from(["end_trask".into()]));
        edges.insert(4, HashSet::from(["other_mod_utc".into()]));
        let seeds = module_entries.get("end_m01aa").cloned().unwrap_or_default();
        let (seen, _parent) = bfs(&index, &seeds, &edges, &winners, &module_entries);
        assert!(seen.contains(&2)); // end_trask
        assert!(!seen.contains(&3)); // other_mod_utc
        assert!(!seen.contains(&4)); // orphan
    }

    fn fixture_end_and_m12() -> Index {
        use serde_json::json;
        let ifo = ResType::from_extension("ifo").unwrap().0;
        let ncs = ResType::from_extension("ncs").unwrap().0;
        let mut index: Index = serde_json::from_value(json!({
            "schema": 3,
            "root": "/game",
            "kind": "install",
            "game": "k1",
            "fingerprint": 0,
            "files": [
                "/game/modules/end_m01aa.mod",
                "/game/modules/M12ab.mod"
            ],
            "sources": [
                {"kind":"module-mod","label":"end_m01aa.mod","precedence":100,"module_root":"end_m01aa"},
                {"kind":"module-mod","label":"M12ab.mod","precedence":100,"module_root":"m12ab"}
            ],
            "resources": [
                {"resref":"module","restype":ifo,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"module","restype":ifo,"file":1,"offset":0,"size":1,"source":1},
                {"resref":"k_pend_activate","restype":ncs,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"k_pend_activate","restype":ncs,"file":1,"offset":0,"size":1,"source":1}
            ],
            "warnings": []
        }))
        .unwrap();
        index.reindex();
        index
    }

    fn fixture_tar_rndtalk() -> Index {
        use serde_json::json;
        let dlg = ResType::from_extension("dlg").unwrap().0;
        let ncs = ResType::from_extension("ncs").unwrap().0;
        let mut index: Index = serde_json::from_value(json!({
            "schema": 3,
            "root": "/game",
            "kind": "install",
            "game": "k1",
            "fingerprint": 0,
            "files": [
                "/game/modules/tar_m03aa.mod",
                "/game/modules/tar_m02aa.mod"
            ],
            "sources": [
                {"kind":"module-mod","label":"tar_m03aa.mod","precedence":100,"module_root":"tar_m03aa"},
                {"kind":"module-mod","label":"tar_m02aa.mod","precedence":100,"module_root":"tar_m02aa"}
            ],
            "resources": [
                {"resref":"tar03_citizen","restype":dlg,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"k_ptar_rndtalk0","restype":ncs,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"k_ptar_rndtalk0","restype":ncs,"file":1,"offset":0,"size":1,"source":1}
            ],
            "warnings": []
        }))
        .unwrap();
        index.reindex();
        index
    }

    #[test]
    fn bfs_case1_ifo_script_in_starting_module() {
        let index = fixture_end_and_m12();
        let winners = scoped_winners(&index);
        let module_entries = module_entry_ids(&index, &winners);
        let ifo_id = *winners
            .get(&(
                Some("end_m01aa".into()),
                "module".into(),
                ResType::from_extension("ifo").unwrap(),
            ))
            .unwrap();
        let mut edges = HashMap::new();
        edges.insert(ifo_id, HashSet::from(["k_pend_activate".into()]));
        let (seen, parent) = bfs(&index, &[ifo_id], &edges, &winners, &module_entries);
        let ncs = ResType::from_extension("ncs").unwrap();
        let want = *winners
            .get(&(Some("end_m01aa".into()), "k_pend_activate".into(), ncs))
            .unwrap();
        let foreign = *winners
            .get(&(Some("m12ab".into()), "k_pend_activate".into(), ncs))
            .unwrap();
        assert!(seen.contains(&want));
        assert!(!seen.contains(&foreign));
        assert_eq!(parent.get(&want), Some(&ifo_id));
    }

    #[test]
    fn bfs_case5_seed_module_with_colliding_are() {
        let index = fixture_two_modules_shared_are();
        let winners = scoped_winners(&index);
        let module_entries = module_entry_ids(&index, &winners);
        let seeds = module_entries.get("ebo_m40ad").cloned().unwrap_or_default();
        assert!(!seeds.is_empty());
        let (seen, _) = bfs(&index, &seeds, &HashMap::new(), &winners, &module_entries);
        assert!(seeds.iter().all(|id| seen.contains(id)));
    }

    #[test]
    fn bfs_case7_dlg_resolves_same_module_script_not_foreign_winner() {
        let index = fixture_tar_rndtalk();
        let winners = scoped_winners(&index);
        let module_entries = module_entry_ids(&index, &winners);
        let dlg_ty = ResType::from_extension("dlg").unwrap();
        let dlg = *winners
            .get(&(Some("tar_m03aa".into()), "tar03_citizen".into(), dlg_ty))
            .unwrap();
        let mut edges = HashMap::new();
        edges.insert(dlg, HashSet::from(["k_ptar_rndtalk0".into()]));
        let (seen, _) = bfs(&index, &[dlg], &edges, &winners, &module_entries);
        let ncs = ResType::from_extension("ncs").unwrap();
        let local = *winners
            .get(&(Some("tar_m03aa".into()), "k_ptar_rndtalk0".into(), ncs))
            .unwrap();
        let foreign = *winners
            .get(&(Some("tar_m02aa".into()), "k_ptar_rndtalk0".into(), ncs))
            .unwrap();
        assert!(seen.contains(&local));
        assert!(!seen.contains(&foreign));
    }

    #[test]
    fn start_new_module_token_counts_without_resref() {
        let known = HashSet::new();
        let roots = HashSet::from(["end_m01aa".into()]);
        let mut out = HashSet::new();
        take_tokens(
            "StartNewModule(\"end_m01aa\")",
            &known,
            &roots,
            "k_sup_gohawk",
            Some("ncs"),
            &mut out,
        );
        assert!(out.contains("end_m01aa"));
    }

    #[test]
    fn filename_token_counts_as_resref() {
        let known = HashSet::from(["n_bastila".into()]);
        let roots = HashSet::new();
        let mut out = HashSet::new();
        take_tokens(
            "n_bastila.utc",
            &known,
            &roots,
            "k_ai_master",
            Some("ncs"),
            &mut out,
        );
        assert!(out.contains("n_bastila"));
    }

    #[test]
    fn strref_columns_skip_labels_and_stars() {
        assert!(is_strref_column("name"));
        assert!(is_strref_column("StringRef"));
        assert!(!is_strref_column("label"));
        assert!(!is_strref_column("_row"));
        assert_eq!(parse_strref("****"), None);
        assert_eq!(parse_strref("-1"), None);
        assert_eq!(parse_strref("48012"), Some(48012));
    }

    #[test]
    fn walk_json_does_not_tokenize_object_keys() {
        let known = HashSet::from(["name".into(), "offset".into(), "k_ai_master".into()]);
        let roots = HashSet::new();
        let mut mentions = HashSet::new();
        let mut strrefs = HashSet::new();
        let v = serde_json::json!({"name": "k_ai_master", "offset": 0});
        walk_json(
            &v,
            &mut Walk {
                known: &known,
                module_roots: &roots,
                self_ref: "row",
                self_ext: None,
                mode: StrRefMode::None,
                mentions: &mut mentions,
                strrefs: &mut strrefs,
            },
            None,
        );
        assert!(!mentions.contains("name"));
        assert!(!mentions.contains("offset"));
        assert!(mentions.contains("k_ai_master"));
    }

    #[test]
    fn consider_token_skips_pure_numeric() {
        let known = HashSet::from(["3".into()]);
        let roots = HashSet::new();
        let mut out = HashSet::new();
        consider_token("3", &known, &roots, "x", None, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn nss_is_not_a_scan_source() {
        let nss = ResType::from_extension("nss").unwrap();
        assert!(!is_scan_source(nss));
        let ncs = ResType::from_extension("ncs").unwrap();
        assert!(is_scan_source(ncs));
    }

    #[test]
    fn nwscript_remains_engine_always_seed_name() {
        assert!(ENGINE_ALWAYS.contains(&"nwscript"));
    }
}
