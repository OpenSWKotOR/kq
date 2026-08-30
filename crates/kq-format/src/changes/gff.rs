//! Work out the `[GFFList]` instructions that turn one structured file into
//! another.
//!
//! A field that both files carry and that changed becomes a plain
//! `path=value` line — the patcher reads the type off the field already there,
//! so the instruction does not restate it. A field only the modified file
//! carries becomes an `AddField` section, and that one *does* name the type,
//! which is why this walks the shared crate's 18-variant `FieldValue` rather
//! than kq's 10-variant display type: `Byte` and `DWORD` are separate keywords
//! and the display type cannot tell them apart.
//!
//! Removal is not expressible. TSLPatcher has no `DeleteField`, so a field the
//! mod dropped is reported as a warning.

use kotor_formats::gff::{FieldValue, GffFile, GffStruct};

use super::{escape, slug, ChangesIni, Section};

/// Sections produced while walking one file, plus where they attach.
struct Pending {
    sections: Vec<Section>,
}

impl ChangesIni {
    /// Add the instructions that turn `base` into `modified`.
    ///
    /// `filename` is the resource as it will be named in `[GFFList]`, for
    /// example `my_item.uti`.
    pub fn add_gff(&mut self, filename: &str, base: &GffFile, modified: &GffFile) {
        let stem = slug(filename);
        let mut file_section = Section::new(filename);
        let mut pending = Pending {
            sections: Vec::new(),
        };
        let mut warnings = Vec::new();
        let mut add_field_index = 0;

        walk(
            &base.root,
            &modified.root,
            "",
            filename,
            &stem,
            &mut file_section,
            &mut pending,
            &mut warnings,
            &mut add_field_index,
        );

        for warning in warnings {
            self.warn(warning);
        }

        if file_section.entries.is_empty() {
            return;
        }

        if !self.gff_files.iter().any(|f| f == filename) {
            self.gff_files.push(filename.to_string());
        }

        // Everything in the file's own section other than an `AddField`
        // reference is a field whose value moved, which is what a token pass
        // is allowed to reconsider. A field the mod created has no previous
        // value to have moved from, so it is deliberately not offered.
        for (key, _) in &file_section.entries {
            if !key.starts_with("AddField") {
                self.changed_entries
                    .push((filename.to_string(), key.clone()));
            }
        }

        // Section names are only made unique once the whole file is walked, so
        // the nested references built during the walk stay valid.
        self.push(file_section);
        for section in pending.sections {
            self.push(section);
        }
    }
}

/// Compare two structures and record what it takes to get from one to the other.
#[allow(clippy::too_many_arguments)]
fn walk(
    base: &GffStruct,
    modified: &GffStruct,
    path: &str,
    filename: &str,
    stem: &str,
    file_section: &mut Section,
    pending: &mut Pending,
    warnings: &mut Vec<String>,
    add_field_index: &mut usize,
) {
    for field in modified.fields() {
        let label = field.label();
        let child_path = join(path, &label);

        match base.field(&label) {
            None => {
                // New field: needs a section that names its type.
                let name = format!("{stem}_addfield_{}", *add_field_index);
                let start = pending.sections.len();
                let section = build_add_field(&name, path, &label, &field.value, pending, stem);
                file_section.set(format!("AddField{}", *add_field_index), &name);
                *add_field_index += 1;
                // Ahead of anything it created, so a reader meets the parent
                // before its children.
                pending.sections.insert(start, section);
            }
            Some(before) => match (&before.value, &field.value) {
                (FieldValue::Struct(b), FieldValue::Struct(m)) => walk(
                    b,
                    m,
                    &child_path,
                    filename,
                    stem,
                    file_section,
                    pending,
                    warnings,
                    add_field_index,
                ),
                (FieldValue::List(b), FieldValue::List(m)) => {
                    for (i, item) in m.iter().enumerate() {
                        match b.get(i) {
                            Some(base_item) => walk(
                                base_item,
                                item,
                                &join(&child_path, &i.to_string()),
                                filename,
                                stem,
                                file_section,
                                pending,
                                warnings,
                                add_field_index,
                            ),
                            None => {
                                // Appending to a list: an unnamed struct whose
                                // own fields hang off it.
                                let name = format!("{stem}_addfield_{}", *add_field_index);
                                let start = pending.sections.len();
                                let section = build_add_struct_to_list(
                                    &name,
                                    &child_path,
                                    item,
                                    pending,
                                    stem,
                                );
                                file_section
                                    .set(format!("AddField{}", *add_field_index), &name);
                                *add_field_index += 1;
                                pending.sections.insert(start, section);
                            }
                        }
                    }
                    if b.len() > m.len() {
                        warnings.push(format!(
                            "{filename}: {} item(s) removed from list \"{child_path}\", which TSLPatcher cannot express",
                            b.len() - m.len()
                        ));
                    }
                }
                (FieldValue::ExoLocString(b), FieldValue::ExoLocString(m)) => {
                    if b.strref != m.strref {
                        file_section
                            .set(format!("{child_path}(strref)"), strref_text(m.strref));
                    }
                    for sub in &m.substrings {
                        let before = b
                            .substrings
                            .iter()
                            .find(|s| s.string_id == sub.string_id)
                            .map(|s| s.text.as_str());
                        if before != Some(sub.text.as_str()) {
                            file_section.set(
                                format!("{child_path}(lang{})", sub.string_id),
                                escape(&sub.text),
                            );
                        }
                    }
                }
                (b, m) if b != m => match scalar(m) {
                    Some(value) => file_section.set(&child_path, value),
                    None => warnings.push(format!(
                        "{filename}: field \"{child_path}\" changed to a type the instruction format has no syntax for"
                    )),
                },
                _ => {}
            },
        }
    }

    for field in base.fields() {
        if modified.field(&field.label()).is_none() {
            warnings.push(format!(
                "{filename}: field \"{}\" was removed, which TSLPatcher cannot express",
                join(path, &field.label())
            ));
        }
    }
}

/// Build the section that creates one new field.
fn build_add_field(
    name: &str,
    path: &str,
    label: &str,
    value: &FieldValue,
    pending: &mut Pending,
    stem: &str,
) -> Section {
    let mut section = Section::new(name);
    section.set("FieldType", type_name(value));
    // An absent Path inherits the parent's; only the outermost section states
    // one, and only when the field is not at the root.
    if !path.is_empty() {
        section.set("Path", path);
    }
    section.set("Label", label);
    fill_value(&mut section, value, pending, stem, name);
    section
}

/// Build the section that appends an unnamed structure to a list.
fn build_add_struct_to_list(
    name: &str,
    list_path: &str,
    item: &GffStruct,
    pending: &mut Pending,
    stem: &str,
) -> Section {
    let mut section = Section::new(name);
    section.set("FieldType", "Struct");
    section.set("Path", list_path);
    // An empty label is what marks this as an append rather than a named field.
    section.set("Label", "");
    section.set("TypeId", item.type_id.to_string());
    add_children(&mut section, item, pending, stem, name);
    section
}

/// Write the value keys for a field, recursing into structures and lists.
fn fill_value(
    section: &mut Section,
    value: &FieldValue,
    pending: &mut Pending,
    stem: &str,
    parent: &str,
) {
    match value {
        FieldValue::ExoLocString(loc) => {
            section.set("StrRef", strref_text(loc.strref));
            for sub in &loc.substrings {
                section.set(format!("lang{}", sub.string_id), escape(&sub.text));
            }
        }
        FieldValue::Struct(inner) => {
            section.set("TypeId", inner.type_id.to_string());
            add_children(section, inner, pending, stem, parent);
        }
        FieldValue::List(items) => {
            for (i, item) in items.iter().enumerate() {
                let name = format!("{parent}_s{i}");
                let mut child = Section::new(&name);
                child.set("FieldType", "Struct");
                child.set("Label", "");
                child.set("TypeId", item.type_id.to_string());
                section.set(format!("AddField{i}"), &name);
                let start = pending.sections.len();
                add_children(&mut child, item, pending, stem, &name);
                pending.sections.insert(start, child);
            }
        }
        other => {
            if let Some(text) = scalar(other) {
                section.set("Value", text);
            }
        }
    }
}

/// Attach a structure's own fields as nested `AddField` sections.
fn add_children(
    section: &mut Section,
    inner: &GffStruct,
    pending: &mut Pending,
    stem: &str,
    parent: &str,
) {
    for (i, field) in inner.fields().iter().enumerate() {
        let name = format!("{parent}_f{i}");
        let mut child = Section::new(&name);
        child.set("FieldType", type_name(&field.value));
        // Path is deliberately absent: a nested section inherits the path of
        // the field that created it. Writing an empty Path would mean the
        // root instead, which is a different place.
        child.set("Label", field.label());
        section.set(format!("AddField{i}"), &name);
        let start = pending.sections.len();
        fill_value(&mut child, &field.value, pending, stem, &name);
        pending.sections.insert(start, child);
    }
}

/// The `FieldType` keyword for a value.
fn type_name(value: &FieldValue) -> &'static str {
    match value {
        FieldValue::Byte(_) => "Byte",
        FieldValue::Char(_) => "Char",
        FieldValue::Word(_) => "Word",
        FieldValue::Short(_) => "Short",
        FieldValue::Dword(_) => "DWORD",
        FieldValue::Int(_) => "Int",
        FieldValue::Dword64(_) => "DWORD64",
        FieldValue::Int64(_) => "Int64",
        FieldValue::Float(_) => "Float",
        FieldValue::Double(_) => "Double",
        FieldValue::ExoString(_) => "ExoString",
        FieldValue::ResRef(_) => "ResRef",
        FieldValue::ExoLocString(_) => "ExoLocString",
        FieldValue::Void(_) => "Binary",
        FieldValue::Struct(_) => "Struct",
        FieldValue::List(_) => "List",
        FieldValue::Orientation(_) => "Orientation",
        FieldValue::Position(_) => "Position",
        FieldValue::StrRef { .. } => "StrRef",
    }
}

/// Render a value as the text an instruction carries.
///
/// Returns `None` for the shapes that have no single-value syntax: structures
/// and lists are described by their own sections, and a 64-bit unsigned field
/// has no keyword the patcher will parse.
fn scalar(value: &FieldValue) -> Option<String> {
    Some(match value {
        FieldValue::Byte(v) => v.to_string(),
        FieldValue::Char(v) => v.to_string(),
        FieldValue::Word(v) => v.to_string(),
        FieldValue::Short(v) => v.to_string(),
        FieldValue::Dword(v) => v.to_string(),
        FieldValue::Int(v) => v.to_string(),
        FieldValue::Int64(v) => v.to_string(),
        FieldValue::Float(v) => v.to_string(),
        FieldValue::Double(v) => v.to_string(),
        FieldValue::ExoString(v) => escape(v),
        FieldValue::ResRef(v) => escape(v),
        FieldValue::Void(bytes) => {
            let mut out = String::from("0x");
            for byte in bytes {
                out.push_str(&format!("{byte:02x}"));
            }
            out
        }
        FieldValue::Orientation(v) => v
            .iter()
            .map(|f| f.to_string())
            .collect::<Vec<_>>()
            .join("|"),
        FieldValue::Position(v) => v
            .iter()
            .map(|f| f.to_string())
            .collect::<Vec<_>>()
            .join("|"),
        FieldValue::Dword64(_)
        | FieldValue::StrRef { .. }
        | FieldValue::ExoLocString(_)
        | FieldValue::Struct(_)
        | FieldValue::List(_) => return None,
    })
}

/// A string-table reference, with the "none" marker written as -1.
fn strref_text(strref: u32) -> String {
    if strref == u32::MAX {
        "-1".to_string()
    } else {
        strref.to_string()
    }
}

/// Join a parent path and a label with the separator the patcher expects.
fn join(path: &str, label: &str) -> String {
    if path.is_empty() {
        label.to_string()
    } else {
        format!("{path}\\{label}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kotor_formats::gff::{ExoLocString, GffField, SubString};

    fn file(fields: Vec<GffField>) -> GffFile {
        let mut f = GffFile::new_file("UTI ", "test.uti");
        for field in fields {
            f.root.add_field(field);
        }
        f
    }

    fn field(label: &str, value: FieldValue) -> GffField {
        GffField::new(label, value)
    }

    #[test]
    fn an_unchanged_file_produces_nothing() {
        let base = file(vec![field("Cost", FieldValue::Dword(10))]);
        let modified = file(vec![field("Cost", FieldValue::Dword(10))]);

        let mut ini = ChangesIni::new();
        ini.add_gff("my_item.uti", &base, &modified);

        assert!(ini.is_empty());
    }

    #[test]
    fn a_changed_field_is_a_bare_path_and_does_not_restate_its_type() {
        let base = file(vec![
            field("Cost", FieldValue::Dword(10)),
            field("Tag", FieldValue::ExoString("old".into())),
        ]);
        let modified = file(vec![
            field("Cost", FieldValue::Dword(500)),
            field("Tag", FieldValue::ExoString("new".into())),
        ]);

        let mut ini = ChangesIni::new();
        ini.add_gff("my_item.uti", &base, &modified);
        let out = ini.render();

        assert!(out.contains("[GFFList]\nFile0=my_item.uti\n"));
        assert!(out.contains("[my_item.uti]\nCost=500\nTag=new\n"));
        // The patcher reads the type off the field already there.
        assert!(!out.contains("FieldType"));
    }

    #[test]
    fn every_scalar_type_renders() {
        let cases: Vec<(&str, FieldValue, &str)> = vec![
            ("B", FieldValue::Byte(7), "7"),
            ("C", FieldValue::Char(65), "65"),
            ("W", FieldValue::Word(300), "300"),
            ("S", FieldValue::Short(-5), "-5"),
            ("D", FieldValue::Dword(9), "9"),
            ("I", FieldValue::Int(-9), "-9"),
            ("L", FieldValue::Int64(-2), "-2"),
            ("F", FieldValue::Float(1.5), "1.5"),
            ("O", FieldValue::Double(2.25), "2.25"),
            ("R", FieldValue::ResRef("res".into()), "res"),
            ("P", FieldValue::Position([1.0, 2.0, 3.0]), "1|2|3"),
            (
                "Q",
                FieldValue::Orientation([0.0, 0.0, 0.0, 1.0]),
                "0|0|0|1",
            ),
            ("V", FieldValue::Void(vec![0xDE, 0xAD]), "0xdead"),
        ];

        for (label, value, expected) in cases {
            assert_eq!(scalar(&value).as_deref(), Some(expected), "{label}");
        }
    }

    #[test]
    fn a_new_field_names_the_exact_on_disk_type() {
        // The whole reason this walks the rich type: Byte and DWORD are
        // different keywords and a display type folds them together.
        let base = file(vec![]);
        let modified = file(vec![
            field("Charges", FieldValue::Byte(42)),
            field("Cost", FieldValue::Dword(42)),
        ]);

        let mut ini = ChangesIni::new();
        ini.add_gff("my_item.uti", &base, &modified);
        let out = ini.render();

        assert!(out.contains("AddField0=my_item_uti_addfield_0"));
        assert!(out.contains("FieldType=Byte\nLabel=Charges\nValue=42\n"));
        assert!(out.contains("FieldType=DWORD\nLabel=Cost\nValue=42\n"));
        // At the root, Path is left out entirely.
        assert!(!out.contains("Path="));
    }

    #[test]
    fn a_localized_field_splits_into_strref_and_language_keys() {
        let base = file(vec![field(
            "Name",
            FieldValue::ExoLocString(ExoLocString {
                byte_size: 8,
                strref: 100,
                substrings: vec![],
            }),
        )]);
        let modified = file(vec![field(
            "Name",
            FieldValue::ExoLocString(ExoLocString {
                byte_size: 8,
                strref: 200,
                substrings: vec![SubString {
                    string_id: 0,
                    text: "Sword".into(),
                }],
            }),
        )]);

        let mut ini = ChangesIni::new();
        ini.add_gff("my_item.uti", &base, &modified);
        let out = ini.render();

        assert!(out.contains("Name(strref)=200"));
        assert!(out.contains("Name(lang0)=Sword"));
    }

    #[test]
    fn a_missing_strref_is_written_as_minus_one() {
        assert_eq!(strref_text(u32::MAX), "-1");
        assert_eq!(strref_text(12), "12");
    }

    #[test]
    fn a_nested_field_is_addressed_through_its_parents() {
        let mut base_inner = GffStruct::new();
        base_inner.add_field(field("Value", FieldValue::Int(1)));
        let mut modified_inner = GffStruct::new();
        modified_inner.add_field(field("Value", FieldValue::Int(2)));

        let base = file(vec![field("Prop", FieldValue::Struct(base_inner))]);
        let modified = file(vec![field("Prop", FieldValue::Struct(modified_inner))]);

        let mut ini = ChangesIni::new();
        ini.add_gff("my_item.uti", &base, &modified);

        assert!(ini.render().contains("Prop\\Value=2"));
    }

    #[test]
    fn a_field_inside_a_list_item_is_addressed_by_position() {
        let mut base_item = GffStruct::new();
        base_item.add_field(field("CostValue", FieldValue::Int(1)));
        let mut modified_item = GffStruct::new();
        modified_item.add_field(field("CostValue", FieldValue::Int(12)));

        let base = file(vec![field(
            "PropertiesList",
            FieldValue::List(vec![base_item]),
        )]);
        let modified = file(vec![field(
            "PropertiesList",
            FieldValue::List(vec![modified_item]),
        )]);

        let mut ini = ChangesIni::new();
        ini.add_gff("my_item.uti", &base, &modified);

        assert!(ini.render().contains("PropertiesList\\0\\CostValue=12"));
    }

    #[test]
    fn appending_to_a_list_adds_an_unnamed_struct_with_its_children() {
        let mut item = GffStruct::new();
        item.type_id = 111;
        item.add_field(field("CostValue", FieldValue::Int(5)));

        let base = file(vec![field("PropertiesList", FieldValue::List(vec![]))]);
        let modified = file(vec![field("PropertiesList", FieldValue::List(vec![item]))]);

        let mut ini = ChangesIni::new();
        ini.add_gff("my_item.uti", &base, &modified);
        let out = ini.render();

        assert!(out.contains("FieldType=Struct"));
        assert!(out.contains("Path=PropertiesList"));
        // An empty label is what makes this an append rather than a named field.
        assert!(out.contains("Label=\n"));
        assert!(out.contains("TypeId=111"));
        // The struct's own field hangs off it, and inherits its path.
        assert!(out.contains("AddField0="));
        assert!(out.contains("FieldType=Int\nLabel=CostValue\nValue=5\n"));
    }

    #[test]
    fn a_nested_section_does_not_write_an_empty_path() {
        // An absent Path inherits the parent's; an empty one means the root.
        // Writing the wrong one puts the field in the wrong place.
        let mut item = GffStruct::new();
        item.add_field(field("Inner", FieldValue::Int(1)));

        let base = file(vec![]);
        let modified = file(vec![field("Outer", FieldValue::Struct(item))]);

        let mut ini = ChangesIni::new();
        ini.add_gff("my_item.uti", &base, &modified);

        assert!(!ini.render().contains("Path=\n"));
    }

    #[test]
    fn a_parent_section_is_written_before_the_children_it_creates() {
        let mut item = GffStruct::new();
        item.add_field(field("Inner", FieldValue::Int(1)));

        let base = file(vec![]);
        let modified = file(vec![field("Outer", FieldValue::Struct(item))]);

        let mut ini = ChangesIni::new();
        ini.add_gff("my_item.uti", &base, &modified);
        let names: Vec<&str> = ini.sections().iter().map(|s| s.name.as_str()).collect();

        let parent = names.iter().position(|n| *n == "my_item_uti_addfield_0");
        let child = names.iter().position(|n| *n == "my_item_uti_addfield_0_f0");
        assert!(parent.is_some() && child.is_some());
        assert!(parent < child, "parent should precede its child: {names:?}");
    }

    #[test]
    fn a_new_list_field_gets_one_struct_section_per_item() {
        let mut first = GffStruct::new();
        first.type_id = 1;
        first.add_field(field("A", FieldValue::Int(1)));
        let mut second = GffStruct::new();
        second.type_id = 2;
        second.add_field(field("B", FieldValue::Int(2)));

        let base = file(vec![]);
        let modified = file(vec![field("Items", FieldValue::List(vec![first, second]))]);

        let mut ini = ChangesIni::new();
        ini.add_gff("my_item.uti", &base, &modified);
        let out = ini.render();

        assert!(out.contains("FieldType=List\nLabel=Items\n"));
        // Item names come from their position, not a running section count,
        // so the same input always produces the same file.
        assert!(out.contains("AddField0=my_item_uti_addfield_0_s0"));
        assert!(out.contains("AddField1=my_item_uti_addfield_0_s1"));
        assert!(out.contains("TypeId=1"));
        assert!(out.contains("TypeId=2"));
    }

    #[test]
    fn a_removed_field_is_reported_rather_than_dropped() {
        let base = file(vec![
            field("Cost", FieldValue::Dword(10)),
            field("Gone", FieldValue::Int(1)),
        ]);
        let modified = file(vec![field("Cost", FieldValue::Dword(10))]);

        let mut ini = ChangesIni::new();
        ini.add_gff("my_item.uti", &base, &modified);

        assert!(ini
            .warnings()
            .iter()
            .any(|w| w.contains("field \"Gone\" was removed")));
    }

    #[test]
    fn a_shortened_list_is_reported() {
        let base = file(vec![field(
            "PropertiesList",
            FieldValue::List(vec![GffStruct::new(), GffStruct::new()]),
        )]);
        let modified = file(vec![field(
            "PropertiesList",
            FieldValue::List(vec![GffStruct::new()]),
        )]);

        let mut ini = ChangesIni::new();
        ini.add_gff("my_item.uti", &base, &modified);

        assert!(ini
            .warnings()
            .iter()
            .any(|w| w.contains("removed from list")));
    }

    #[test]
    fn a_multi_line_string_survives_the_line_based_format() {
        let base = file(vec![field("Text", FieldValue::ExoString("one".into()))]);
        let modified = file(vec![field(
            "Text",
            FieldValue::ExoString("one\ntwo".into()),
        )]);

        let mut ini = ChangesIni::new();
        ini.add_gff("dlg.dlg", &base, &modified);
        let out = ini.render();

        assert!(out.contains("Text=one<#LF#>two"));
        // The value stays on one line.
        assert_eq!(out.matches("Text=").count(), 1);
    }
}
