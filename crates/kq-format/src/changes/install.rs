//! Build the `[InstallList]` instructions that copy whole files into place.
//!
//! Each key in `[InstallList]` names a section; the value is where that
//! section's files land — a folder below the game, or an archive already
//! there. The files themselves are listed inside that section.
//!
//! Whether an existing file is overwritten is decided by the key name alone.
//! The original patcher reads `File<n>` and `Replace<n>` in `[InstallList]`
//! and never looks at `!ReplaceFile` there, so the key prefix is the only
//! thing that carries the intent.

use super::{ChangesIni, Section};

/// One file to install, and whether it may overwrite what is already there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallFile {
    /// Name of the file inside `tslpatchdata`.
    pub name: String,
    /// Overwrite an existing copy rather than leaving it alone.
    pub replace: bool,
}

impl InstallFile {
    /// A file that is skipped when one of the same name is already installed.
    pub fn keep(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            replace: false,
        }
    }

    /// A file that overwrites an installed copy.
    pub fn replace(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            replace: true,
        }
    }
}

impl ChangesIni {
    /// Copy `files` into `destination`.
    ///
    /// `destination` is a folder below the game such as `override`, or an
    /// archive that already exists such as `modules\danm13.mod`. A blank
    /// destination installs to `override`.
    pub fn add_install(&mut self, destination: &str, files: &[InstallFile]) {
        if files.is_empty() {
            return;
        }

        let target = if destination.is_empty() {
            "override"
        } else {
            destination
        };

        let section_name = self.unique_name(&format!("install_{}", super::slug(target)));
        let mut section = Section::new(&section_name);

        // Each kind is numbered from zero on its own, which is how the
        // patcher walks them.
        let mut keep_index = 0;
        let mut replace_index = 0;
        for file in files {
            if file.replace {
                section.set(format!("Replace{replace_index}"), &file.name);
                replace_index += 1;
            } else {
                section.set(format!("File{keep_index}"), &file.name);
                keep_index += 1;
            }
        }

        self.install_folders
            .push((section_name.clone(), target.to_string()));
        self.push(section);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_to_install_writes_no_list() {
        let mut ini = ChangesIni::new();
        ini.add_install("override", &[]);

        assert!(!ini.render().contains("[InstallList]"));
    }

    #[test]
    fn a_folder_names_a_section_that_lists_its_files() {
        let mut ini = ChangesIni::new();
        ini.add_install(
            "override",
            &[
                InstallFile::keep("new_icon.tga"),
                InstallFile::keep("extra.tga"),
            ],
        );
        let out = ini.render();

        assert!(out.contains("[InstallList]\ninstall_override=override\n"));
        assert!(out.contains("[install_override]\nFile0=new_icon.tga\nFile1=extra.tga\n"));
    }

    #[test]
    fn overwriting_is_carried_by_the_key_name() {
        let mut ini = ChangesIni::new();
        ini.add_install(
            "override",
            &[
                InstallFile::keep("leave_alone.tga"),
                InstallFile::replace("take_over.tga"),
            ],
        );
        let out = ini.render();

        assert!(out.contains("File0=leave_alone.tga"));
        assert!(out.contains("Replace0=take_over.tga"));
    }

    #[test]
    fn an_archive_is_a_destination_like_any_other() {
        let mut ini = ChangesIni::new();
        ini.add_install("modules\\danm13.mod", &[InstallFile::keep("area.git")]);
        let out = ini.render();

        assert!(out.contains("install_modules_danm13_mod=modules\\danm13.mod\n"));
    }

    #[test]
    fn a_blank_destination_means_override() {
        let mut ini = ChangesIni::new();
        ini.add_install("", &[InstallFile::keep("x.tga")]);

        assert!(ini.render().contains("install_override=override\n"));
    }
}
