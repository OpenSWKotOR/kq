use serde::{Deserialize, Serialize};

/// Where in the installation a resource was found.
///
/// The engine resolves a ResRef by walking locations in a fixed order, so a
/// source is not just a label — it is the thing that decides which of several
/// identically-named resources actually loads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceKind {
    /// Loose files in `Override/`. Beats everything.
    Override,
    /// A `<root>.mod` in `modules/`.
    ModuleMod,
    /// A `<root>.rim`, `<root>_s.rim` or `<root>_dlg.erf` in `modules/`.
    /// Bare `<root>.rim` is the CURRENTGAME IFO/ARE/GIT table and outranks
    /// [`ModuleMod`]; `_s.rim` / `_dlg.erf` lose to `.mod`.
    ModuleRim,
    /// A `.mod` in `lips/`.
    Lips,
    /// One of the `swpc_tex_*.erf` texture packs.
    TexturePack,
    /// A capsule in the K1-only `rims/` folder.
    Rims,
    /// Loose audio under `streammusic/`, `streamsounds/`, `streamwaves/`,
    /// `streamvoice/`.
    Stream,
    /// A BIF indexed by `chitin.key`. The base game; loses to everything.
    Chitin,
    /// `dialog.tlk` / `dialogf.tlk` at the install root.
    TalkTable,
    /// A path the user pointed at directly, outside any install.
    Loose,
}

impl SourceKind {
    /// Lower wins. Gaps leave room for per-source tie-breaks (texture pack
    /// order; CURRENTGAME `NAME.rim` uses precedence 50 instead of 200).
    pub fn base_precedence(self) -> u32 {
        match self {
            SourceKind::Override => 0,
            SourceKind::ModuleMod => 100,
            // Default for `_s.rim` / `_dlg.erf`. Bare `NAME.rim` uses 50.
            SourceKind::ModuleRim => 200,
            SourceKind::Lips => 300,
            SourceKind::TexturePack => 400,
            SourceKind::Rims => 500,
            SourceKind::Stream => 600,
            SourceKind::Chitin => 700,
            SourceKind::TalkTable => 750,
            SourceKind::Loose => 800,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            SourceKind::Override => "override",
            SourceKind::ModuleMod => "module-mod",
            SourceKind::ModuleRim => "module-rim",
            SourceKind::Lips => "lips",
            SourceKind::TexturePack => "texturepack",
            SourceKind::Rims => "rims",
            SourceKind::Stream => "stream",
            SourceKind::Chitin => "chitin",
            SourceKind::TalkTable => "talktable",
            SourceKind::Loose => "loose",
        }
    }
}

/// One place resources come from: a capsule, a BIF, or a folder.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Source {
    pub kind: SourceKind,
    /// Human-facing name, e.g. `danm13.mod` or `data/2da.bif`.
    pub label: String,
    /// Resolved precedence. Lower wins.
    pub precedence: u32,
    /// Module root (`danm13`) when this source belongs to a module.
    pub module_root: Option<String>,
}
