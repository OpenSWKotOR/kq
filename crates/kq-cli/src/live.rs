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
    pub reachable: HashSet<String>,
    /// Reachable ResRefs plus VO names on used talk-table rows.
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

    let mut scan_ids: Vec<u32> = (0..index.resources.len() as u32).collect();
    Filter::dedup_winners(index, &mut scan_ids);
    scan_ids.retain(|&i| is_scan_source(index.resources[i as usize].restype));

    crate::output::warn(format!(
        "catalog {} ResRefs; scanning {} for mentions…",
        catalog.len(),
        scan_ids.len()
    ));

    let hits = Mutex::new(Vec::<(String, HashSet<String>, HashSet<i64>)>::new());
    scan_ids.par_iter().for_each(|&i| {
        let r = &index.resources[i as usize];
        let Ok(bytes) = read::read(index, r) else {
            return;
        };
        let Ok(decoded) = render::decode(&bytes, Some(r.restype), &r.filename()) else {
            return;
        };
        let mut mentions = HashSet::new();
        let mut strrefs = HashSet::new();
        collect(
            &decoded,
            &catalog,
            &r.resref,
            strref_mode(r.restype),
            &mut mentions,
            &mut strrefs,
        );
        if !mentions.is_empty() || !strrefs.is_empty() {
            hits.lock()
                .expect("live scan lock")
                .push((r.resref.clone(), mentions, strrefs));
        }
    });

    let mut edges: HashMap<String, HashSet<String>> = HashMap::new();
    let mut strrefs_by: HashMap<String, HashSet<i64>> = HashMap::new();
    for (resref, mentions, strrefs) in hits.into_inner().expect("live scan lock") {
        if !mentions.is_empty() {
            edges.entry(resref.clone()).or_default().extend(mentions);
        }
        if !strrefs.is_empty() {
            strrefs_by.entry(resref).or_default().extend(strrefs);
        }
    }

    let seeds = seeds(index, &catalog);
    let reachable = bfs(&seeds, &edges);

    let tlk = load_dialog_tlk(index)?;
    let tlk_len = tlk.len() as i64;
    let mut used_strrefs = HashSet::new();
    for r in &reachable {
        if let Some(refs) = strrefs_by.get(r) {
            for &n in refs {
                if n >= 0 && n < tlk_len {
                    used_strrefs.insert(n);
                }
            }
        }
    }

    let mut used = reachable.clone();
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
        seeds,
        reachable,
        used,
        used_strrefs,
        tlk,
        scanned: scan_ids.len(),
    })
}

pub fn is_asset(t: ResType) -> bool {
    ASSET_EXTS.contains(&t.extension().unwrap_or(""))
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

fn seeds(index: &Index, known: &HashSet<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    match index.kind {
        RootKind::Install => {
            for name in ENGINE_ALWAYS.iter().chain(ENGINE_2DAS) {
                push_known(&mut out, known, name);
            }
            let extra: &[&str] = match index.game {
                kq_index::Game::K1 => K1_MODULES,
                kq_index::Game::K2 => K2_MODULES,
            };
            for name in extra.iter().chain(K1_SCRIPTS) {
                push_known(&mut out, known, name);
            }
            for name in ini_starting_modules(&index.root) {
                push_known(&mut out, known, &name);
            }
        }
        RootKind::Capsule | RootKind::Folder | RootKind::File => {
            for r in &index.resources {
                if matches!(r.restype.extension(), Some("ifo" | "are" | "git")) {
                    push_known(&mut out, known, &r.resref);
                }
            }
            if out.is_empty() {
                let mut names: Vec<String> =
                    index.resources.iter().map(|r| r.resref.clone()).collect();
                names.sort();
                names.dedup();
                out = names;
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn push_known(out: &mut Vec<String>, known: &HashSet<String>, name: &str) {
    let name = name.to_ascii_lowercase();
    if known.contains(&name) {
        out.push(name);
    }
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

fn bfs(seeds: &[String], edges: &HashMap<String, HashSet<String>>) -> HashSet<String> {
    let mut seen = HashSet::new();
    let mut q = VecDeque::new();
    for s in seeds {
        if seen.insert(s.clone()) {
            q.push_back(s.clone());
        }
    }
    while let Some(n) = q.pop_front() {
        let Some(outs) = edges.get(&n) else {
            continue;
        };
        for m in outs {
            if seen.insert(m.clone()) {
                q.push_back(m.clone());
            }
        }
    }
    seen
}

fn collect(
    decoded: &Decoded,
    known: &HashSet<String>,
    self_ref: &str,
    mode: StrRefMode,
    mentions: &mut HashSet<String>,
    strrefs: &mut HashSet<i64>,
) {
    match decoded {
        Decoded::Value(v) => walk_json(v, known, self_ref, mode, None, mentions, strrefs),
        Decoded::Text(s) => take_tokens(s, known, self_ref, mentions),
        Decoded::Opaque { .. } => {}
    }
}

fn walk_json(
    v: &J,
    known: &HashSet<String>,
    self_ref: &str,
    mode: StrRefMode,
    key: Option<&str>,
    mentions: &mut HashSet<String>,
    strrefs: &mut HashSet<i64>,
) {
    match v {
        J::String(s) => {
            take_tokens(s, known, self_ref, mentions);
            if matches!(mode, StrRefMode::TwoDa) && key.is_some_and(is_strref_column) {
                if let Some(n) = parse_strref(s) {
                    strrefs.insert(n);
                }
            }
        }
        J::Number(n) => {
            if let Some(i) = n.as_i64() {
                match mode {
                    StrRefMode::Ssf => {
                        strrefs.insert(i);
                    }
                    StrRefMode::Gff if key.is_some_and(|k| k.eq_ignore_ascii_case("strref")) => {
                        strrefs.insert(i);
                    }
                    StrRefMode::TwoDa if key.is_some_and(is_strref_column) => {
                        strrefs.insert(i);
                    }
                    _ => {}
                }
            }
        }
        J::Array(items) => {
            for item in items {
                walk_json(item, known, self_ref, mode, key, mentions, strrefs);
            }
        }
        J::Object(map) => {
            for (k, val) in map {
                take_tokens(k, known, self_ref, mentions);
                walk_json(val, known, self_ref, mode, Some(k), mentions, strrefs);
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

fn take_tokens(s: &str, known: &HashSet<String>, self_ref: &str, out: &mut HashSet<String>) {
    let lower = s.to_ascii_lowercase();
    let mut start = None;
    for (i, c) in lower.char_indices() {
        if c.is_ascii_alphanumeric() || c == '_' {
            if start.is_none() {
                start = Some(i);
            }
        } else if let Some(st) = start.take() {
            consider_token(&lower[st..i], known, self_ref, out);
        }
    }
    if let Some(st) = start {
        consider_token(&lower[st..], known, self_ref, out);
    }
}

fn consider_token(tok: &str, known: &HashSet<String>, self_ref: &str, out: &mut HashSet<String>) {
    if tok.is_empty() || tok.len() > 16 || tok == self_ref || tok == "****" {
        return;
    }
    if known.contains(tok) {
        out.insert(tok.to_string());
        return;
    }
    if let Some((base, ext)) = tok.rsplit_once('.') {
        if ResType::from_extension(ext).is_some() && base != self_ref && known.contains(base) {
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
        let mut out = HashSet::new();
        take_tokens(
            "Tag=n_bastila Script=k_ai_master ****",
            &known,
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
        edges.insert("dead_a".into(), HashSet::from(["dead_b".into()]));
        edges.insert("dead_b".into(), HashSet::from(["dead_a".into()]));
        edges.insert("end_m01aa".into(), HashSet::from(["n_endsol01".into()]));
        let seen = bfs(&["end_m01aa".into()], &edges);
        assert!(seen.contains("end_m01aa"));
        assert!(seen.contains("n_endsol01"));
        assert!(!seen.contains("dead_a"));
        assert!(!seen.contains("dead_b"));
    }

    #[test]
    fn filename_token_counts_as_resref() {
        let known = HashSet::from(["n_bastila".into()]);
        let mut out = HashSet::new();
        take_tokens("n_bastila.utc", &known, "k_ai_master", &mut out);
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
