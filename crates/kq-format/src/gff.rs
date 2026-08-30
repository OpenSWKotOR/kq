//! GFF — the tree format behind creatures, doors, dialog, areas, journals and
//! about twenty other extensions.
//!
//! One parser covers all of them: the extension only names the schema, never
//! the encoding. Layout (V3.2) is six (offset, count) pairs pointing at the
//! struct, field, label, field-data, field-index and list-index arrays.

use std::collections::BTreeMap;
use std::path::Path;

use kotor_formats::gff::{
    FieldValue, GffParseOptions, GffStruct as SharedStruct, GffFile as SharedGff,
};

use crate::error::Result;
use crate::shared::{cp1252_display, format_error};

/// A GFF value, decoded into something a query language can walk.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i64),
    UInt(u64),
    Float(f64),
    /// CExoString and ResRef both land here; the distinction is schema, not
    /// data, and keeping them apart would only complicate every query.
    Str(String),
    /// A localized string: a talk-table reference plus any inline overrides.
    LocString {
        strref: i64,
        substrings: BTreeMap<u32, String>,
    },
    /// Raw bytes, kept as-is.
    Void(Vec<u8>),
    Struct(Struct),
    List(Vec<Struct>),
    Vector([f32; 3]),
    Orientation([f32; 4]),
    /// A talk-table index. Resolvable against dialog.tlk.
    StrRef(i64),
}

/// One GFF struct: an id plus ordered named fields.
#[derive(Clone, Debug, PartialEq)]
pub struct Struct {
    pub id: u32,
    /// Field order is the file's order, which is meaningful for diffing.
    pub fields: Vec<(String, Value)>,
}

impl Struct {
    pub fn get(&self, label: &str) -> Option<&Value> {
        self.fields
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(label))
            .map(|(_, v)| v)
    }
}

/// A parsed GFF file.
#[derive(Clone, Debug)]
pub struct Gff {
    /// The four-character type tag, e.g. `UTC `, trimmed.
    pub file_type: String,
    pub version: String,
    pub root: Struct,
}

/// True when the bytes start with any GFF-family signature.
///
/// GFF has no single magic number — the first four bytes are the content
/// type — so the version field four bytes in is what identifies it.
pub fn sniff(data: &[u8]) -> bool {
    data.len() >= 8 && (&data[4..8] == b"V3.2" || &data[4..8] == b"V3.3")
}

pub fn read(data: &[u8], path: &Path) -> Result<Gff> {
    let display = path.display().to_string();
    // kq only reads, so it accepts more than the games themselves write: V3.3
    // headers and StrRef fields, both of which appear in Aurora-family tooling.
    let parsed = SharedGff::parse_with(data, &display, GffParseOptions::lenient())
        .map_err(|e| format_error(e, path))?;

    Ok(Gff {
        file_type: parsed.type_name().trim().to_string(),
        version: String::from_utf8_lossy(&parsed.file_version)
            .trim()
            .to_string(),
        root: project_struct(&parsed.root),
    })
}

/// Narrow a shared structure into kq's query-shaped one.
fn project_struct(node: &SharedStruct) -> Struct {
    Struct {
        id: node.type_id,
        fields: node
            .fields()
            .iter()
            .map(|f| (f.label(), project_value(&f.value)))
            .collect(),
    }
}

/// Narrow a shared value, collapsing distinctions a query does not need.
///
/// The shared type keeps every on-disk width apart because it has to write
/// them back; kq only displays them, so the integer widths fold into two
/// signed/unsigned buckets and both string types become [`Value::Str`].
fn project_value(value: &FieldValue) -> Value {
    match value {
        FieldValue::Byte(v) => Value::UInt(*v as u64),
        FieldValue::Char(v) => Value::Int(*v as i8 as i64),
        FieldValue::Word(v) => Value::UInt(*v as u64),
        FieldValue::Short(v) => Value::Int(*v as i64),
        FieldValue::Dword(v) => Value::UInt(*v as u64),
        FieldValue::Int(v) => Value::Int(*v as i64),
        FieldValue::Dword64(raw) => Value::UInt(u64::from_le_bytes(*raw)),
        FieldValue::Int64(v) => Value::Int(*v),
        FieldValue::Float(v) => Value::Float(*v as f64),
        FieldValue::Double(v) => Value::Float(*v),
        FieldValue::ExoString(s) => Value::Str(cp1252_display(s)),
        FieldValue::ResRef(s) => Value::Str(cp1252_display(s)),
        FieldValue::ExoLocString(loc) => Value::LocString {
            // 0xFFFFFFFF is "no table entry", which reads as -1.
            strref: loc.strref as i32 as i64,
            substrings: loc
                .substrings
                .iter()
                .map(|sub| (sub.string_id as u32, cp1252_display(&sub.text)))
                .collect::<BTreeMap<_, _>>(),
        },
        FieldValue::Void(bytes) => Value::Void(bytes.clone()),
        FieldValue::Struct(node) => Value::Struct(project_struct(node)),
        FieldValue::List(items) => Value::List(items.iter().map(project_struct).collect()),
        FieldValue::Orientation(q) => Value::Orientation(*q),
        FieldValue::Position(v) => Value::Vector(*v),
        FieldValue::StrRef { value, .. } => Value::StrRef(*value as i64),
    }
}

/// Map one Windows-1252 byte to its Unicode code point.
pub fn cp1252_char(b: u8) -> char {
    // 0x80..0x9F is where Windows-1252 differs from Latin-1; everything else
    // is identity.
    const HIGH: [char; 32] = [
        '\u{20AC}', '\u{81}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}',
        '\u{2021}', '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{8D}',
        '\u{017D}', '\u{8F}', '\u{90}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}',
        '\u{2013}', '\u{2014}', '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}',
        '\u{9D}', '\u{017E}', '\u{0178}',
    ];
    if (0x80..0xA0).contains(&b) {
        HIGH[(b - 0x80) as usize]
    } else {
        b as char
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kotor_formats::gff::{
        ExoLocString, FieldValue as Fv, GffField, GffFile as Shared, GffStruct,
    };

    /// Build bytes through the shared writer, then read them back through kq.
    fn round_trip(build: impl FnOnce(&mut Shared)) -> Gff {
        let mut file = Shared::new_file("UTI ", "test.uti");
        build(&mut file);
        let bytes = file.to_bytes().unwrap();
        read(&bytes, Path::new("test.uti")).unwrap()
    }

    #[test]
    fn integer_widths_fold_into_signed_and_unsigned() {
        let gff = round_trip(|f| {
            f.root.add_field(GffField::new("B", Fv::Byte(200)));
            f.root.add_field(GffField::new("C", Fv::Char(0xFF)));
            f.root.add_field(GffField::new("W", Fv::Word(65535)));
            f.root.add_field(GffField::new("S", Fv::Short(-2)));
            f.root.add_field(GffField::new("D", Fv::Dword(4000000000)));
            f.root.add_field(GffField::new("I", Fv::Int(-7)));
        });

        assert_eq!(gff.root.get("B"), Some(&Value::UInt(200)));
        // Char is signed in kq's view, so 0xFF reads as -1.
        assert_eq!(gff.root.get("C"), Some(&Value::Int(-1)));
        assert_eq!(gff.root.get("W"), Some(&Value::UInt(65535)));
        assert_eq!(gff.root.get("S"), Some(&Value::Int(-2)));
        assert_eq!(gff.root.get("D"), Some(&Value::UInt(4000000000)));
        assert_eq!(gff.root.get("I"), Some(&Value::Int(-7)));
    }

    #[test]
    fn both_string_types_collapse_to_str() {
        let gff = round_trip(|f| {
            f.root
                .add_field(GffField::new("Tag", Fv::ExoString("hello".into())));
            f.root
                .add_field(GffField::new("Ref", Fv::ResRef("some_ref".into())));
        });

        assert_eq!(gff.root.get("Tag"), Some(&Value::Str("hello".into())));
        assert_eq!(gff.root.get("Ref"), Some(&Value::Str("some_ref".into())));
    }

    #[test]
    fn high_bytes_render_as_windows_1252() {
        // 0x92 is stored losslessly as U+0092 and shown as a curly apostrophe.
        let gff = round_trip(|f| {
            f.root
                .add_field(GffField::new("Tag", Fv::ExoString("don\u{92}t".into())));
        });
        assert_eq!(gff.root.get("Tag"), Some(&Value::Str("don\u{2019}t".into())));
    }

    #[test]
    fn a_locstring_without_a_table_entry_reads_as_minus_one() {
        let gff = round_trip(|f| {
            let mut loc = ExoLocString::default();
            loc.add_string(0, "A Blade").unwrap();
            f.root.add_field(GffField::new("Name", Fv::ExoLocString(loc)));
        });

        match gff.root.get("Name") {
            Some(Value::LocString { strref, substrings }) => {
                assert_eq!(*strref, -1);
                assert_eq!(substrings.get(&0).map(String::as_str), Some("A Blade"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn nested_structs_and_lists_keep_their_ids_and_order() {
        let gff = round_trip(|f| {
            let mut item = GffStruct::new();
            item.type_id = 9;
            item.add_field(GffField::new("First", Fv::Int(1)));
            item.add_field(GffField::new("Second", Fv::Int(2)));
            f.root
                .add_field(GffField::new("List", Fv::List(vec![item])));
        });

        match gff.root.get("List") {
            Some(Value::List(items)) => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].id, 9);
                let names: Vec<&str> =
                    items[0].fields.iter().map(|(k, _)| k.as_str()).collect();
                assert_eq!(names, ["First", "Second"]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn strref_fields_are_read() {
        // Type 18 is not something the games write, so the shared crate only
        // yields it because kq asks for a lenient parse.
        let gff = round_trip(|f| {
            f.root.add_field(GffField::new(
                "Ref",
                Fv::StrRef {
                    byte_size: 4,
                    value: 1234,
                },
            ));
        });
        assert_eq!(gff.root.get("Ref"), Some(&Value::StrRef(1234)));
    }

    #[test]
    fn v3_3_headers_are_accepted_and_reported() {
        let mut file = Shared::new_file("UTI ", "test.uti");
        file.root.add_field(GffField::new("Cost", Fv::Dword(1)));
        let mut bytes = file.to_bytes().unwrap();
        bytes[4..8].copy_from_slice(b"V3.3");

        let gff = read(&bytes, Path::new("test.uti")).unwrap();
        assert_eq!(gff.version, "V3.3");
        assert_eq!(gff.file_type, "UTI");
    }

    #[test]
    fn a_malformed_file_names_the_path_it_came_from() {
        let err = read(b"not a gff file at all!!!", Path::new("bad.uti")).unwrap_err();
        assert!(format!("{err}").contains("bad.uti"));
    }
}
