//! `kq cat` — print a resource as text.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::Path;

use anyhow::{bail, Result};
use kq_format::{gff, ResType};
use kq_index::{Index, Resource};

use crate::render::{self, DisasmMode, Format};
use crate::resource_json;
use crate::{exit, read, Ctx};

#[derive(clap::Args)]
pub struct Args {
    /// ResRef to print, with or without an extension.
    #[arg(value_name = "RESREF", required_unless_present = "tag")]
    resref: Option<String>,

    /// Read a placed GIT object by its Tag. Requires --module and --type.
    ///
    /// For example: `kq cat --module end_m01aa --tag end_door01 -t utd`.
    #[arg(long, value_name = "TAG", requires = "module", requires = "restype")]
    tag: Option<String>,

    /// Restrict to one resource type when the name is ambiguous.
    #[arg(short = 't', long = "type", value_name = "EXT")]
    restype: Option<String>,

    /// How to render the resource (`json` is the default structured form).
    #[arg(short = 'f', long, value_name = "FORMAT", default_value = "json")]
    format: Format,

    /// Write the resource's exact bytes. Same as `--format raw`.
    #[arg(long)]
    raw: bool,

    /// Show NCS instruction disassembly instead of decompiled NSS.
    #[arg(long)]
    disasm: bool,

    /// Read from a container label or an existing resource/archive file path.
    /// A file path does not require an installation.
    #[arg(long, value_name = "NAME", conflicts_with = "module")]
    from: Option<String>,

    /// Read the highest-precedence copy from this module root.
    #[arg(short = 'm', long, value_name = "ROOT", conflicts_with = "from")]
    module: Option<String>,
}

pub fn run(ctx: &Ctx, args: Args) -> Result<i32> {
    let (name, want) = match (&args.resref, &args.tag) {
        (Some(resref), None) => crate::parse_ref(resref, args.restype.as_deref())?,
        (None, Some(tag)) => (
            tag.to_ascii_lowercase(),
            args.restype.as_deref().map(parse_type).transpose()?,
        ),
        _ => bail!("specify exactly one of RESREF or --tag"),
    };
    // An explicit file is a complete source, even outside an install or when
    // KQ_INSTALL points elsewhere. Bare container labels retain indexed lookup.
    let from_file = args.from.as_deref().map(Path::new).filter(|p| p.is_file());
    let index = match from_file {
        Some(path) => kq_index::open_standalone(path)?,
        None => ctx.index()?,
    };

    let resource = match (&args.tag, &args.from, &args.module) {
        (Some(tag), None, Some(module)) => select_tag_from_module(&index, tag, want, module)?,
        (Some(_), _, _) => bail!("--tag requires --module and cannot be used with --from"),
        (None, Some(_), None) if from_file.is_some() => index.resolve(&name, want),
        (None, Some(container), None) => index
            .lookup(&name)
            .iter()
            .map(|&i| &index.resources[i as usize])
            .find(|r| {
                want.is_none_or(|t| r.restype == t)
                    && index.source(r).label.eq_ignore_ascii_case(container)
            }),
        (None, None, Some(module)) => {
            if !index
                .module_roots()
                .iter()
                .any(|root| root.eq_ignore_ascii_case(module))
            {
                eprintln!("kq: no module root named {module}");
                return Ok(exit::NO_MATCH);
            }
            select_from_module(&index, &name, want, module)?
                // `cat --module end_m01aa end_m01aa -t git` is the natural
                // way to ask for a module's placement graph, but retail GIT
                // resrefs are often the area suffix (`m01aa.git`). Resolve
                // that spelling only when the module contains one GIT.
                .or_else(|| module_git_alias(&index, &name, want, module))
                // Module data commonly refers to stock scripts stored in a
                // shared BIF. A compiled script has one unambiguous type, so
                // after a module miss it is safe to use that global winner.
                .or_else(|| module_script_fallback(&index, &name, want))
        }
        (None, None, None) => index.resolve(&name, want),
        // clap rejects this combination before `run`, but retain a clear
        // invariant for direct callers and future argument refactors.
        (None, Some(_), Some(_)) => bail!("--from and --module cannot be used together"),
    };
    let Some(resource) = resource else {
        match (&args.tag, &args.module) {
            (Some(tag), Some(module)) => eprintln!(
                "kq: no placed object tagged {tag} with type {} in module {module}",
                want.unwrap()
            ),
            (_, Some(module)) => eprintln!("kq: no resource named {name} in module {module}"),
            _ => eprintln!("kq: no resource named {name}"),
        }
        return Ok(exit::NO_MATCH);
    };

    let bytes = read::read(&index, resource)?;
    let stdout = std::io::stdout();
    let mut w = stdout.lock();

    let format = if args.raw {
        Format::Raw
    } else if ctx.out.text {
        match args.format {
            Format::Json => Format::Outline,
            other => other,
        }
    } else {
        Format::Json
    };

    if format == Format::Raw {
        w.write_all(&bytes)?;
        return Ok(exit::OK);
    }

    let disasm = if args.disasm || format == Format::Json {
        DisasmMode::On
    } else {
        DisasmMode::Off
    };
    let decoded = render::decode_resource_mode(&index, resource, &bytes, disasm)?;

    if format == Format::Json {
        let report = resource_json::build_resource_json(&index, resource, &decoded);
        ctx.out.json_value(&report)?;
        return Ok(exit::OK);
    }

    w.write_all(render::render(&decoded, format, &resource.filename())?.as_bytes())?;
    Ok(exit::OK)
}

fn parse_type(ext: &str) -> Result<ResType> {
    ResType::from_extension(ext).ok_or_else(|| anyhow::anyhow!("unknown resource type: {ext}"))
}

/// Resolve an object's Tag through its placed GIT instance and template. GIT
/// instances often omit Tag, in which case the template supplies it.
fn select_tag_from_module<'a>(
    index: &'a Index,
    tag: &str,
    want: Option<ResType>,
    module: &str,
) -> Result<Option<&'a Resource>> {
    let want = want.expect("clap requires --type with --tag");
    let git = ResType::from_extension("git").expect("GIT is a known resource type");
    let mut module_gits = BTreeSet::new();
    for resource in &index.resources {
        if resource.restype == git
            && index
                .source(resource)
                .module_root
                .as_deref()
                .is_some_and(|root| root.eq_ignore_ascii_case(module))
        {
            module_gits.insert(resource.resref.to_ascii_lowercase());
        }
    }

    // A logical module may contain multiple area GITs. Resolve each resref
    // at module precedence, then look for the requested tag across them.
    let mut graphs = Vec::new();
    for name in module_gits {
        let Some(resource) = index
            .lookup(&name)
            .iter()
            .map(|&i| &index.resources[i as usize])
            .find(|r| {
                r.restype == git
                    && index
                        .source(r)
                        .module_root
                        .as_deref()
                        .is_some_and(|root| root.eq_ignore_ascii_case(module))
            })
        else {
            continue;
        };
        let bytes = read::read(index, resource)?;
        graphs.push(gff::read(&bytes, Path::new(&resource.filename()))?);
    }
    let mut matching_templates = BTreeSet::new();
    for graph in &graphs {
        for (instance_tag, template) in git_template_candidates(graph, want) {
            let Some(resource) = resolve_module_template(index, &template, want, module)? else {
                continue;
            };
            let matches = if let Some(instance_tag) = instance_tag {
                instance_tag.eq_ignore_ascii_case(tag)
            } else {
                let bytes = read::read(index, resource)?;
                let template_gff = gff::read(&bytes, Path::new(&resource.filename()))?;
                template_tag_matches(&template_gff, tag)
            };
            if matches {
                matching_templates.insert(resource.resref.to_ascii_lowercase());
            }
        }
    }

    match matching_templates.len() {
        0 => Ok(None),
        1 => {
            let template = matching_templates.first().expect("one matched template");
            resolve_module_template(index, template, want, module)
        }
        _ => bail!(
            "placed tag {tag} has multiple {want} templates: {}",
            matching_templates
                .into_iter()
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// GIT placement lists and the resource type of their templates.
/// Types are explicit because GFF does not record a schema for a list entry.
const GIT_TEMPLATE_LISTS: &[(&str, &str)] = &[
    ("Creature List", "utc"),
    ("Door List", "utd"),
    ("Encounter List", "ute"),
    ("Placeable List", "utp"),
    ("SoundList", "uts"),
    ("StoreList", "uts"),
    ("TriggerList", "utt"),
    ("WaypointList", "utw"),
];

fn git_template_candidates(git: &gff::Gff, want: ResType) -> Vec<(Option<String>, String)> {
    let mut templates = BTreeSet::new();
    for &(list_name, ext) in GIT_TEMPLATE_LISTS {
        if ResType::from_extension(ext) != Some(want) {
            continue;
        }
        let Some(gff::Value::List(entries)) = git.root.get(list_name) else {
            continue;
        };
        for entry in entries {
            let Some(gff::Value::Str(template)) = entry.get("TemplateResRef") else {
                continue;
            };
            if template.is_empty() {
                continue;
            }
            let instance_tag = match entry.get("Tag") {
                Some(gff::Value::Str(tag)) if !tag.is_empty() => Some(tag.to_ascii_lowercase()),
                _ => None,
            };
            templates.insert((instance_tag, template.to_ascii_lowercase()));
        }
    }
    templates.into_iter().collect()
}

fn template_tag_matches(template: &gff::Gff, tag: &str) -> bool {
    matches!(
        template.root.get("Tag"),
        Some(gff::Value::Str(template_tag)) if template_tag.eq_ignore_ascii_case(tag)
    )
}

fn resolve_module_template<'a>(
    index: &'a Index,
    name: &str,
    want: ResType,
    module: &str,
) -> Result<Option<&'a Resource>> {
    Ok(
        select_from_module(index, name, Some(want), module)?.or_else(|| {
            index
                .lookup(name)
                .iter()
                .map(|&i| &index.resources[i as usize])
                .find(|r| r.restype == want && index.source(r).module_root.is_none())
        }),
    )
}

/// Resolve a resource inside one logical module.
///
/// A module can be composed from `<root>.mod`, `<root>.rim`,
/// `<root>_s.rim`, and `<root>_dlg.erf`. `Index::lookup` is already ordered
/// by engine precedence, so the first same-type match is the copy that module
/// resolution would use. Different resource types still require `--type`;
/// choosing one by archive order would make an ambiguous ResRef look stable.
fn select_from_module<'a>(
    index: &'a Index,
    name: &str,
    want: Option<ResType>,
    module: &str,
) -> Result<Option<&'a Resource>> {
    let matches: Vec<&Resource> = index
        .lookup(name)
        .iter()
        .map(|&i| &index.resources[i as usize])
        .filter(|r| want.is_none_or(|t| r.restype == t))
        .filter(|r| {
            index
                .source(r)
                .module_root
                .as_deref()
                .is_some_and(|root| root.eq_ignore_ascii_case(module))
        })
        .collect();

    if want.is_none() {
        let mut types: Vec<ResType> = matches.iter().map(|r| r.restype).collect();
        types.sort_unstable();
        types.dedup();
        if types.len() > 1 {
            let names = types
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            bail!("resource {name} has multiple types in module {module}: {names}; specify --type");
        }
    }

    Ok(matches.into_iter().next())
}

fn module_git_alias<'a>(
    index: &'a Index,
    name: &str,
    want: Option<ResType>,
    module: &str,
) -> Option<&'a Resource> {
    let git = ResType::from_extension("git")?;
    if want != Some(git) || !name.eq_ignore_ascii_case(module) {
        return None;
    }
    let mut matches = index.resources.iter().filter(|r| {
        r.restype == git
            && index
                .source(r)
                .module_root
                .as_deref()
                .is_some_and(|root| root.eq_ignore_ascii_case(module))
    });
    let only = matches.next()?;
    matches.next().is_none().then_some(only)
}

/// Return the highest-precedence shared compiled script after a module miss.
///
/// Do not widen module lookup for data resources: their resrefs can be reused
/// across modules. The engine executes `.ncs` scripts, and `--type ncs`
/// removes the type ambiguity that `cat --module` otherwise reports.
fn module_script_fallback<'a>(
    index: &'a Index,
    name: &str,
    want: Option<ResType>,
) -> Option<&'a Resource> {
    let ncs = ResType::from_extension("ncs")?;
    (want == Some(ncs)).then(|| {
        index
            .lookup(name)
            .iter()
            .map(|&i| &index.resources[i as usize])
            .find(|r| r.restype == ncs && index.source(r).module_root.is_none())
    })?
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_index() -> Index {
        let ncs = ResType::from_extension("ncs").unwrap().0;
        let dlg = ResType::from_extension("dlg").unwrap().0;
        let git = ResType::from_extension("git").unwrap().0;
        let mut index: Index = serde_json::from_value(json!({
            "schema": 3,
            "root": "/game",
            "kind": "install",
            "game": "k1",
            "fingerprint": 0,
            "files": [
                "/game/Override/test.ncs",
                "/game/modules/end_m01aa.mod",
                "/game/modules/end_m01aa_s.rim",
                "/game/modules/end_m01ab.mod",
                "/game/data/scripts.bif"
            ],
            "sources": [
                {"kind":"override","label":"Override","precedence":0,"module_root":null},
                {"kind":"module-mod","label":"end_m01aa.mod","precedence":100,"module_root":"end_m01aa"},
                {"kind":"module-rim","label":"end_m01aa_s.rim","precedence":200,"module_root":"end_m01aa"},
                {"kind":"module-mod","label":"end_m01ab.mod","precedence":101,"module_root":"end_m01ab"},
                {"kind":"chitin","label":"scripts.bif","precedence":700,"module_root":null}
            ],
            "resources": [
                {"resref":"shared","restype":ncs,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"shared","restype":ncs,"file":1,"offset":0,"size":1,"source":1},
                {"resref":"shared","restype":ncs,"file":2,"offset":0,"size":1,"source":2},
                {"resref":"shared","restype":ncs,"file":3,"offset":0,"size":1,"source":3},
                {"resref":"ambiguous","restype":ncs,"file":1,"offset":0,"size":1,"source":1},
                {"resref":"ambiguous","restype":dlg,"file":2,"offset":0,"size":1,"source":2}
                ,{"resref":"m01aa","restype":git,"file":1,"offset":0,"size":1,"source":1},
                {"resref":"global_script","restype":ncs,"file":4,"offset":0,"size":1,"source":4},
                {"resref":"global_text","restype":2019,"file":4,"offset":0,"size":1,"source":4}
            ],
            "warnings": []
        }))
        .unwrap();
        index.reindex();
        index
    }

    #[test]
    fn module_selector_ignores_override_and_other_modules() {
        let index = test_index();
        let ncs = ResType::from_extension("ncs").unwrap();

        let selected = select_from_module(&index, "shared", Some(ncs), "END_M01AA")
            .unwrap()
            .unwrap();

        assert_eq!(index.source(selected).label, "end_m01aa.mod");
    }

    #[test]
    fn module_selector_uses_composite_module_precedence() {
        let index = test_index();
        let ncs = ResType::from_extension("ncs").unwrap();

        let selected = select_from_module(&index, "shared", Some(ncs), "end_m01aa")
            .unwrap()
            .unwrap();

        assert_eq!(index.source(selected).label, "end_m01aa.mod");
    }

    #[test]
    fn module_selector_reports_missing_resource() {
        let index = test_index();
        let ncs = ResType::from_extension("ncs").unwrap();

        assert!(
            select_from_module(&index, "missing", Some(ncs), "end_m01aa")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn module_selector_requires_type_for_ambiguous_resref() {
        let index = test_index();

        let error = select_from_module(&index, "ambiguous", None, "end_m01aa")
            .unwrap_err()
            .to_string();

        assert!(error.contains("multiple types in module end_m01aa"));
        assert!(error.contains("specify --type"));
    }

    #[test]
    fn module_root_is_an_alias_for_its_unique_git() {
        let index = test_index();
        let git = ResType::from_extension("git").unwrap();

        let selected = module_git_alias(&index, "end_m01aa", Some(git), "end_m01aa").unwrap();

        assert_eq!(selected.resref, "m01aa");
    }

    #[test]
    fn module_selector_falls_back_to_shared_compiled_script() {
        let index = test_index();
        let ncs = ResType::from_extension("ncs").unwrap();

        assert!(
            select_from_module(&index, "global_script", Some(ncs), "end_m01aa")
                .unwrap()
                .is_none()
        );
        let selected = module_script_fallback(&index, "global_script", Some(ncs)).unwrap();

        assert_eq!(index.source(selected).label, "scripts.bif");
    }

    #[test]
    fn module_selector_does_not_fall_back_for_other_resource_types() {
        let index = test_index();
        let txt = ResType::from_extension("txt").unwrap();

        assert!(module_script_fallback(&index, "global_text", Some(txt)).is_none());
        assert!(module_script_fallback(&index, "global_script", None).is_none());
    }

    #[test]
    fn tag_lookup_follows_typed_instance_to_template_tag() {
        let git = gff::Gff {
            file_type: "GIT".into(),
            version: "V3.2".into(),
            root: gff::Struct {
                id: 0,
                fields: vec![
                    (
                        "Door List".into(),
                        gff::Value::List(vec![gff::Struct {
                            id: 8,
                            fields: vec![(
                                "TemplateResRef".into(),
                                gff::Value::Str("sw_door_test001".into()),
                            )],
                        }]),
                    ),
                    (
                        "Placeable List".into(),
                        gff::Value::List(vec![gff::Struct {
                            id: 9,
                            fields: vec![(
                                "TemplateResRef".into(),
                                gff::Value::Str("footlker001".into()),
                            )],
                        }]),
                    ),
                ],
            },
        };
        let utd = ResType::from_extension("utd").unwrap();
        let utp = ResType::from_extension("utp").unwrap();

        let utp_candidates = git_template_candidates(&git, utp);
        assert_eq!(utp_candidates, vec![(None, "footlker001".into())]);
        assert_eq!(
            git_template_candidates(&git, utd),
            vec![(None, "sw_door_test001".into())]
        );

        let placeable = gff::Gff {
            file_type: "GIT".into(),
            version: "V3.2".into(),
            root: gff::Struct {
                id: 0,
                fields: vec![(
                    "Placeable List".into(),
                    gff::Value::List(vec![gff::Struct {
                        id: 9,
                        fields: vec![(
                            "TemplateResRef".into(),
                            gff::Value::Str("footlker001".into()),
                        )],
                    }]),
                )],
            },
        };
        let template = gff::Gff {
            file_type: "UTP".into(),
            version: "V3.2".into(),
            root: gff::Struct {
                id: 0,
                fields: vec![("Tag".into(), gff::Value::Str("end_locker01".into()))],
            },
        };

        assert_eq!(git_template_candidates(&placeable, utp), utp_candidates);
        assert!(template_tag_matches(&template, "END_LOCKER01"));
        assert!(!template_tag_matches(&template, "end_door01"));
    }
}
