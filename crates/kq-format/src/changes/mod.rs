//! Generate a TSLPatcher `changes.ini` from a pair of resources.
//!
//! Given the same file before and after a mod's edits, this works out what
//! instructions would reproduce the edit and writes them as INI. The output
//! targets the original Delphi TSLPatcher contract, documented in OdyPatcher's
//! `docs/instructions.md` and enforced by `UTSLPatcher.pas`.
//!
//! The diff runs over the shared `kotor-formats` types, not kq's own
//! query-shaped ones. It has to: an `AddField` section names the field's exact
//! on-disk type (`Byte` and `DWORD` are different keywords), and kq's `Value`
//! folds every integer width into one variant. The rich type keeps the type
//! tag, so it is the only side that can answer the question.
//!
//! Tokens are not emitted while diffing. [`ChangesIni::link_tokens`] is a
//! separate, opt-in pass that notices when a literal in one file matches an
//! index the diff itself created — which is why the document is kept as
//! ordered key/value pairs rather than finished text until [`ChangesIni::render`].

pub mod gff;
pub mod install;
pub mod ssf;
pub mod tlk;
pub mod tokens;
pub mod twoda;

/// One `[name]` block and its keys, in the order they must be written.
///
/// Order is load-bearing: TSLPatcher walks a modifier section's keys top to
/// bottom, and a row selector such as `RowIndex` has to arrive before the
/// columns it applies to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// Text between the brackets.
    pub name: String,
    /// Keys and values, in write order.
    pub entries: Vec<(String, String)>,
}

impl Section {
    /// An empty section.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            entries: Vec::new(),
        }
    }

    /// Append a key and value.
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.entries.push((key.into(), value.into()));
    }

    /// Look up the first value stored under a key.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

/// A row this diff appends, and the section that appends it.
///
/// Recorded so [`ChangesIni::link_tokens`] can turn a literal reference to the
/// row's eventual position into a `2DAMEMORY` token, and add the capture key
/// that fills the slot.
#[derive(Debug, Clone)]
pub(crate) struct CreatedRow {
    /// The section that carries the `AddRow` keys.
    pub section: String,
    /// Where the row lands once appended.
    pub index: usize,
    /// The label it will carry.
    pub label: String,
    /// Slot number, once something references it.
    pub token: Option<u32>,
}

/// A string this diff appends to the talk table.
#[derive(Debug, Clone)]
pub(crate) struct CreatedStrRef {
    /// The `StrRef<n>` token that names it.
    pub token: u32,
    /// Where the string lands in the game's table once appended.
    pub resulting_strref: u32,
}

/// A generated instruction file, still in pieces.
#[derive(Debug, Clone, Default)]
pub struct ChangesIni {
    twoda_files: Vec<String>,
    gff_files: Vec<String>,
    ssf_files: Vec<String>,
    /// `StrRef<token>=<index into append.tlk>` pairs.
    tlk_tokens: Vec<(u32, usize)>,
    /// `<section name>=<destination>` pairs for `[InstallList]`.
    install_folders: Vec<(String, String)>,
    sections: Vec<Section>,
    warnings: Vec<String>,
    pub(crate) created_rows: Vec<CreatedRow>,
    pub(crate) created_strrefs: Vec<CreatedStrRef>,
    /// `(section, key)` for entries whose value differs from the base file.
    ///
    /// Only these are eligible for token rewriting: a value that changed is
    /// the one piece of evidence available that a number means something
    /// rather than being a number.
    pub(crate) changed_entries: Vec<(String, String)>,
}

impl ChangesIni {
    /// An instruction file with nothing in it.
    pub fn new() -> Self {
        Self::default()
    }

    /// Every section, in write order.
    ///
    /// Exposed so a later token pass can rewrite literal values into
    /// `2DAMEMORY` / `StrRef` references.
    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    /// Every section, mutable.
    pub fn sections_mut(&mut self) -> &mut [Section] {
        &mut self.sections
    }

    /// Edits that were seen but cannot be expressed as instructions.
    ///
    /// TSLPatcher has no way to delete a row, a column, or a field, so a mod
    /// that removes one leaves a gap the generated file cannot close. Callers
    /// should surface these rather than let the difference pass silently.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// True when nothing was found to change.
    pub fn is_empty(&self) -> bool {
        self.sections.is_empty()
    }

    fn warn(&mut self, message: impl Into<String>) {
        self.warnings.push(message.into());
    }

    fn push(&mut self, section: Section) {
        self.sections.push(section);
    }

    /// Find a section by name.
    pub(crate) fn section_mut(&mut self, name: &str) -> Option<&mut Section> {
        self.sections.iter_mut().find(|s| s.name == name)
    }

    /// Reserve a section name that no other section is using.
    fn unique_name(&self, wanted: &str) -> String {
        if !self.sections.iter().any(|s| s.name == wanted) {
            return wanted.to_string();
        }
        for n in 2.. {
            let candidate = format!("{wanted}_{n}");
            if !self.sections.iter().any(|s| s.name == candidate) {
                return candidate;
            }
        }
        unreachable!("an unused suffix always exists")
    }

    /// Write the instruction file.
    ///
    /// A `[Settings]` block comes first with `FileExists=1`, because the
    /// original patcher refuses to start without it — it reads the key
    /// defaulting to false and exits when it is not set.
    pub fn render(&self) -> String {
        let mut out = String::new();

        out.push_str("[Settings]\n");
        out.push_str("FileExists=1\n");
        out.push_str("InstallerMode=1\n");
        out.push_str("LogLevel=3\n");

        if !self.twoda_files.is_empty() {
            out.push_str("\n[2DAList]\n");
            for (i, file) in self.twoda_files.iter().enumerate() {
                out.push_str(&format!("Table{i}={file}\n"));
            }
        }

        if !self.gff_files.is_empty() {
            out.push_str("\n[GFFList]\n");
            for (i, file) in self.gff_files.iter().enumerate() {
                out.push_str(&format!("File{i}={file}\n"));
            }
        }

        if !self.ssf_files.is_empty() {
            out.push_str("\n[SSFList]\n");
            for (i, file) in self.ssf_files.iter().enumerate() {
                out.push_str(&format!("File{i}={file}\n"));
            }
        }

        // The talk table is its own list: the keys map a token to a line in
        // the appended file, and the source file is named alongside them.
        if !self.tlk_tokens.is_empty() {
            out.push_str("\n[TLKList]\n");
            for (token, append_index) in &self.tlk_tokens {
                out.push_str(&format!("StrRef{token}={append_index}\n"));
            }
        }

        // A key here names a section; its value is where those files land.
        if !self.install_folders.is_empty() {
            out.push_str("\n[InstallList]\n");
            for (section, destination) in &self.install_folders {
                out.push_str(&format!("{section}={destination}\n"));
            }
        }

        for section in &self.sections {
            out.push_str(&format!("\n[{}]\n", section.name));
            for (key, value) in &section.entries {
                out.push_str(&format!("{key}={value}\n"));
            }
        }

        out
    }
}

/// Turn a filename into something usable as part of a section name.
fn slug(filename: &str) -> String {
    let mut out: String = filename
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    while out.contains("__") {
        out = out.replace("__", "_");
    }
    out.trim_matches('_').to_lowercase()
}

/// Protect a value that would otherwise break the line it sits on.
///
/// The original patcher's INI reader turns `<#LF#>` and `<#CR#>` back into
/// line breaks when it reads a value, so that is how a multi-line string is
/// carried through a single-line format.
fn escape(value: &str) -> String {
    value.replace('\r', "<#CR#>").replace('\n', "<#LF#>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slug_is_safe_inside_brackets() {
        assert_eq!(slug("spells.2da"), "spells_2da");
        assert_eq!(slug("my item.uti"), "my_item_uti");
        assert_eq!(slug("__weird--name__"), "weird_name");
    }

    #[test]
    fn line_breaks_survive_a_single_line_format() {
        assert_eq!(escape("one\ntwo"), "one<#LF#>two");
        assert_eq!(escape("one\r\ntwo"), "one<#CR#><#LF#>two");
        assert_eq!(escape("plain"), "plain");
    }

    #[test]
    fn section_names_do_not_collide() {
        let mut ini = ChangesIni::new();
        ini.push(Section::new("dup"));
        assert_eq!(ini.unique_name("dup"), "dup_2");
        ini.push(Section::new("dup_2"));
        assert_eq!(ini.unique_name("dup"), "dup_3");
        assert_eq!(ini.unique_name("fresh"), "fresh");
    }

    #[test]
    fn an_empty_document_still_starts_the_patcher() {
        // FileExists is not optional: the original reads it defaulting to
        // false and refuses to run when it is missing.
        let rendered = ChangesIni::new().render();
        assert!(rendered.starts_with("[Settings]\n"));
        assert!(rendered.contains("FileExists=1\n"));
        assert!(!rendered.contains("[2DAList]"));
        assert!(!rendered.contains("[GFFList]"));
    }
}
