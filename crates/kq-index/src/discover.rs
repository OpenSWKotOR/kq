//! Finding an installation and its folders.
//!
//! KotOR shipped on Windows, so its folder names are cased inconsistently
//! (`Override` vs `override`, `Modules` vs `modules`). Every lookup here is
//! case-insensitive and resolves once, at index time, to a real path.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::game::Game;

/// A located installation: the root, the game, and the folders that exist.
#[derive(Clone, Debug)]
pub struct Install {
    pub root: PathBuf,
    pub game: Game,
    pub chitin: PathBuf,
    pub data: Option<PathBuf>,
    pub modules: Option<PathBuf>,
    pub override_dir: Option<PathBuf>,
    pub lips: Option<PathBuf>,
    pub texturepacks: Option<PathBuf>,
    pub rims: Option<PathBuf>,
    pub streams: Vec<PathBuf>,
    /// The master string table at the install root. Not part of any
    /// container, so nothing else would ever notice it exists.
    pub talk_tables: Vec<PathBuf>,
}

/// Case-insensitively resolve a direct child of `dir`.
pub fn child(dir: &Path, name: &str) -> Option<PathBuf> {
    let direct = dir.join(name);
    if direct.exists() {
        return Some(direct);
    }
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .find(|e| e.file_name().eq_ignore_ascii_case(name))
        .map(|e| e.path())
}

fn child_dir(dir: &Path, name: &str) -> Option<PathBuf> {
    child(dir, name).filter(|p| p.is_dir())
}

/// True when `path` looks like a KotOR install root.
pub fn is_install_root(path: &Path) -> bool {
    path.is_dir() && child(path, "chitin.key").is_some()
}

/// Open an installation rooted at `root`.
///
/// The game is detected from what the install actually contains rather than
/// from the folder name, so renamed and repackaged installs still work.
pub fn open(root: &Path) -> Result<Install> {
    let root = root
        .canonicalize()
        .with_context(|| format!("cannot resolve installation path {}", root.display()))?;
    let Some(chitin) = child(&root, "chitin.key") else {
        bail!("{} is not a KotOR installation (no chitin.key)", root.display());
    };

    let modules = child_dir(&root, "modules");
    let game = detect_game(&root, modules.as_deref());

    let mut streams = Vec::new();
    for name in ["streammusic", "streamsounds", "streamwaves", "streamvoice"] {
        if let Some(p) = child_dir(&root, name) {
            streams.push(p);
        }
    }

    let talk_tables =
        ["dialog.tlk", "dialogf.tlk"].iter().filter_map(|n| child(&root, n)).collect();

    Ok(Install {
        chitin,
        data: child_dir(&root, "data"),
        modules,
        override_dir: child_dir(&root, "override"),
        lips: child_dir(&root, "lips"),
        texturepacks: child_dir(&root, "texturepacks"),
        rims: child_dir(&root, "rims"),
        streams,
        talk_tables,
        game,
        root,
    })
}

/// Decide K1 vs K2 from install contents.
///
/// The executable name is checked first because it is unambiguous when
/// present. Digital and repackaged releases often drop it, so the fallback is
/// a module root that only ever shipped with one of the two games.
fn detect_game(root: &Path, modules: Option<&Path>) -> Game {
    for (name, game) in [
        ("swkotor2.exe", Game::K2),
        ("KOTOR2.exe", Game::K2),
        ("swkotor.exe", Game::K1),
        ("KOTOR.exe", Game::K1),
    ] {
        if child(root, name).is_some() {
            return game;
        }
    }
    // TSL-only signature files, then a K1-only module.
    for name in ["streamvoice", "lips"] {
        if name == "streamvoice" && child_dir(root, name).is_some() {
            return Game::K2;
        }
    }
    if let Some(modules) = modules {
        if child(modules, "001EBO.mod").is_some() || child(modules, "003EBO.rim").is_some() {
            return Game::K2;
        }
        if child(modules, "danm13.rim").is_some() || child(modules, "end_m01aa.rim").is_some() {
            return Game::K1;
        }
    }
    Game::K1
}

/// Walk upward from `start` looking for an install root.
///
/// Lets `kq` be run from inside `Override/` or `modules/` the way `git` can
/// be run from anywhere in a work tree.
pub fn find_upward(start: &Path) -> Option<PathBuf> {
    let mut current = start.canonicalize().ok()?;
    loop {
        if is_install_root(&current) {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}
