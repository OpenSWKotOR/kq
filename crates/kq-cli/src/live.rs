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
const MODULE_ENTRY_EXTS: &[&str] = &["ifo", "are", "git", "lyt", "vis", "pth"];

/// Engine-opened talk files and the include the compiler always sees.
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

    let winners = all_winners(index);
    let by_resref = winners_by_resref(index, &winners);
    let module_entries = module_entry_ids(index, &winners);

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

    let (seed_labels, seed_ids) = seed_ids(index, &catalog, &by_resref, &module_entries);
    let (used_ids, parent) = bfs(&seed_ids, &edges, &by_resref, &module_entries);

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

fn winners_by_resref(index: &Index, winners: &[u32]) -> HashMap<String, Vec<u32>> {
    let mut map: HashMap<String, Vec<u32>> = HashMap::new();
    for &i in winners {
        map.entry(index.resources[i as usize].resref.clone())
            .or_default()
            .push(i);
    }
    map
}

fn module_entry_ids(index: &Index, winners: &[u32]) -> HashMap<String, Vec<u32>> {
    let mut map: HashMap<String, Vec<u32>> = HashMap::new();
    for &i in winners {
        let r = &index.resources[i as usize];
        let Some(ext) = r.restype.extension() else {
            continue;
        };
        if !MODULE_ENTRY_EXTS.contains(&ext) {
            continue;
        }
        let Some(root) = index.source(r).module_root.as_deref() else {
            continue;
        };
        map.entry(root.to_ascii_lowercase()).or_default().push(i);
    }
    map
}

fn is_scan_source(t: ResType) -> bool {
    t.is_gff() || t.is_plain_text() || matches!(t.extension(), Some("2da" | "ncs" | "ssf" | "mdl"))
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
    by_resref: &HashMap<String, Vec<u32>>,
    module_entries: &HashMap<String, Vec<u32>>,
) -> (Vec<String>, Vec<u32>) {
    let mut labels = Vec::new();
    let mut ids = Vec::new();

    let push_resref = |name: &str, labels: &mut Vec<String>, ids: &mut Vec<u32>| {
        let name = name.to_ascii_lowercase();
        if let Some(list) = by_resref.get(&name) {
            labels.push(name);
            ids.extend(list.iter().copied());
        }
    };
    let push_module = |name: &str, labels: &mut Vec<String>, ids: &mut Vec<u32>| {
        let name = name.to_ascii_lowercase();
        if let Some(list) = module_entries.get(&name) {
            labels.push(name);
            ids.extend(list.iter().copied());
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
        }
        RootKind::Capsule | RootKind::Folder | RootKind::File => {
            for r in &index.resources {
                if matches!(r.restype.extension(), Some("ifo" | "are" | "git")) {
                    if let Some(list) = by_resref.get(&r.resref) {
                        labels.push(r.resref.clone());
                        ids.extend(list.iter().copied());
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

fn bfs(
    seeds: &[u32],
    edges: &HashMap<u32, HashSet<String>>,
    by_resref: &HashMap<String, Vec<u32>>,
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
        let Some(tokens) = edges.get(&id) else {
            continue;
        };
        for tok in tokens {
            if let Some(ids) = by_resref.get(tok) {
                for &j in ids {
                    if seen.insert(j) {
                        parent.insert(j, id);
                        q.push_back(j);
                    }
                }
            }
            if let Some(ids) = module_entries.get(tok) {
                for &j in ids {
                    if seen.insert(j) {
                        parent.insert(j, id);
                        q.push_back(j);
                    }
                }
            }
        }
    }
    (seen, parent)
}

fn collect(
    decoded: &Decoded,
    known: &HashSet<String>,
    module_roots: &HashSet<String>,
    self_ref: &str,
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
                mode,
                mentions,
                strrefs,
            },
            None,
        ),
        Decoded::Text(s) => take_tokens(s, known, module_roots, self_ref, mentions),
        Decoded::Opaque { .. } => {}
    }
}

struct Walk<'a> {
    known: &'a HashSet<String>,
    module_roots: &'a HashSet<String>,
    self_ref: &'a str,
    mode: StrRefMode,
    mentions: &'a mut HashSet<String>,
    strrefs: &'a mut HashSet<i64>,
}

fn walk_json(v: &J, w: &mut Walk<'_>, key: Option<&str>) {
    match v {
        J::String(s) => {
            take_tokens(s, w.known, w.module_roots, w.self_ref, w.mentions);
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
                take_tokens(k, w.known, w.module_roots, w.self_ref, w.mentions);
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
    out: &mut HashSet<String>,
) {
    let lower = s.to_ascii_lowercase();
    let mut start = None;
    for (i, c) in lower.char_indices() {
        if c.is_ascii_alphanumeric() || c == '_' {
            if start.is_none() {
                start = Some(i);
            }
        } else if let Some(st) = start.take() {
            consider_token(&lower[st..i], known, module_roots, self_ref, out);
        }
    }
    if let Some(st) = start {
        consider_token(&lower[st..], known, module_roots, self_ref, out);
    }
}

fn consider_token(
    tok: &str,
    known: &HashSet<String>,
    module_roots: &HashSet<String>,
    self_ref: &str,
    out: &mut HashSet<String>,
) {
    if tok.is_empty() || tok.len() > 16 || tok == self_ref || tok == "****" {
        return;
    }
    if known.contains(tok) || module_roots.contains(tok) {
        out.insert(tok.to_string());
        return;
    }
    if let Some((base, ext)) = tok.rsplit_once('.') {
        if ResType::from_extension(ext).is_some()
            && base != self_ref
            && (known.contains(base) || module_roots.contains(base))
        {
            out.insert(base.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            &mut out,
        );
        assert!(out.contains("k_ai_master"));
        assert!(!out.contains("n_bastila"));
        assert!(!out.contains("****"));
    }

    #[test]
    fn isolated_cycle_is_not_reachable() {
        let mut edges = HashMap::new();
        edges.insert(1, HashSet::from(["dead_b".into()]));
        edges.insert(2, HashSet::from(["dead_a".into()]));
        edges.insert(10, HashSet::from(["n_endsol01".into()]));
        let mut by_resref = HashMap::new();
        by_resref.insert("dead_a".into(), vec![1]);
        by_resref.insert("dead_b".into(), vec![2]);
        by_resref.insert("n_endsol01".into(), vec![11]);
        let (seen, _parent) = bfs(&[10], &edges, &by_resref, &HashMap::new());
        assert!(seen.contains(&10));
        assert!(seen.contains(&11));
        assert!(!seen.contains(&1));
        assert!(!seen.contains(&2));
    }

    #[test]
    fn module_folder_enters_git_not_shared_ifo_name() {
        // end_m01aa is a folder. The GIT is m01aa.git (id 20). Every module
        // also has a module.ifo; only this module's ifo (id 21) is an entry.
        let mut edges = HashMap::new();
        edges.insert(20, HashSet::from(["end_trask".into()]));
        edges.insert(99, HashSet::from(["other_mod_utc".into()]));
        let mut by_resref = HashMap::new();
        by_resref.insert("end_trask".into(), vec![30]);
        by_resref.insert("other_mod_utc".into(), vec![31]);
        let mut module_entries = HashMap::new();
        module_entries.insert("end_m01aa".into(), vec![20, 21]);
        let (seen, _parent) = bfs(&[20, 21], &edges, &by_resref, &module_entries);
        assert!(seen.contains(&30));
        assert!(!seen.contains(&31));
        assert!(!seen.contains(&99));
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
            &mut out,
        );
        assert!(out.contains("end_m01aa"));
    }

    #[test]
    fn filename_token_counts_as_resref() {
        let known = HashSet::from(["n_bastila".into()]);
        let roots = HashSet::new();
        let mut out = HashSet::new();
        take_tokens("n_bastila.utc", &known, &roots, "k_ai_master", &mut out);
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
}
