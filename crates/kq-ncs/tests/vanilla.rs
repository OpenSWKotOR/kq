//! Ignored K1 corpus smoke. Needs `KQ_INSTALL` (and PyKotor for fixpoint).

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::process::Command;

use kq_index::{Index, Resource};

/// Laptop-sized default. `KQ_NCS_CORPUS=all` walks every NCS.
const CORPUS_CAP: usize = 50;
const FIXPOINT_SAMPLE: usize = 3;

fn corpus_limit() -> usize {
    match std::env::var("KQ_NCS_CORPUS") {
        Ok(v) if v.eq_ignore_ascii_case("all") => usize::MAX,
        _ => CORPUS_CAP,
    }
}

/// Same per-resource read as `kq-cli` (`File` + seek + `read_exact`).
fn read_resource_bytes(index: &Index, r: &Resource) -> std::io::Result<Vec<u8>> {
    let path = index.file(r);
    let mut file = std::fs::File::open(path)?;
    if r.offset > 0 {
        file.seek(SeekFrom::Start(r.offset))?;
    }
    let mut buf = vec![0u8; r.size as usize];
    file.read_exact(&mut buf)?;
    Ok(buf)
}

fn ncs_resources(index: &Index) -> impl Iterator<Item = &Resource> {
    index
        .resources
        .iter()
        .filter(|r| r.restype.extension() == Some("ncs"))
}

fn pykotor_available() -> bool {
    Command::new("python3")
        .args([
            "-c",
            "from pykotor.resource.formats.ncs import compile_nss, bytes_ncs",
        ])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[test]
#[ignore = "requires KQ_INSTALL"]
fn k1_corpus_no_panic_nonempty() {
    let root = std::env::var("KQ_INSTALL").expect("KQ_INSTALL");
    let (_install, index, _) = kq_index::open(Path::new(&root), true, false).unwrap();
    let limit = corpus_limit();
    let mut n = 0usize;
    let mut complete = 0usize;
    for r in ncs_resources(&index) {
        let Ok(bytes) = read_resource_bytes(&index, r) else {
            continue;
        };
        let Ok(ncs) = kq_format::ncs::read(&bytes, Path::new("x.ncs")) else {
            continue;
        };
        let d = kq_ncs::decompile(&ncs, index.game);
        assert!(!d.source.is_empty(), "empty decompile for {}", r.filename());
        n += 1;
        if d.complete {
            complete += 1;
        }
        if n >= limit {
            break;
        }
    }
    assert!(n > 0, "no readable NCS under KQ_INSTALL");
    eprintln!("complete {complete}/{n}");
}

#[test]
#[ignore = "requires PyKotor + KQ_INSTALL"]
fn fixpoint_sample() {
    let Ok(root) = std::env::var("KQ_INSTALL") else {
        eprintln!("skipping fixpoint_sample: KQ_INSTALL unset");
        return;
    };
    if !pykotor_available() {
        eprintln!("skipping fixpoint_sample: PyKotor/PYTHONPATH unavailable");
        return;
    }

    let (_install, index, _) = kq_index::open(Path::new(&root), true, false).unwrap();
    let work = std::env::temp_dir().join(format!("kq-ncs-fixpoint-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).unwrap();

    let mut nss_paths = Vec::new();
    let mut sources = Vec::new();
    for r in ncs_resources(&index) {
        if nss_paths.len() >= FIXPOINT_SAMPLE {
            break;
        }
        let Ok(bytes) = read_resource_bytes(&index, r) else {
            continue;
        };
        let Ok(ncs) = kq_format::ncs::read(&bytes, Path::new("x.ncs")) else {
            continue;
        };
        let d = kq_ncs::decompile(&ncs, index.game);
        if d.source.is_empty() {
            continue;
        }
        let path = work.join(format!("{}.nss", r.resref));
        std::fs::write(&path, d.source.as_bytes()).unwrap();
        nss_paths.push(path);
        sources.push(d.source);
    }
    assert!(!nss_paths.is_empty(), "no readable NCS under KQ_INSTALL");

    let game = match index.game {
        kq_index::Game::K1 => "Game.K1",
        kq_index::Game::K2 => "Game.K2",
    };
    let mut args = vec![
        "-c".into(),
        format!(
            "from pykotor.resource.formats.ncs import compile_nss, bytes_ncs\n\
             from pykotor.common.misc import Game\n\
             import sys, pathlib\n\
             for p in sys.argv[1:]:\n\
             \tsrc = pathlib.Path(p).read_text(encoding='windows-1252', errors='replace')\n\
             \tncs = compile_nss(src, {game}, library_lookup=[pathlib.Path(p).parent])\n\
             \tpathlib.Path(p).with_suffix('.ncs').write_bytes(bytes_ncs(ncs))\n"
        ),
    ];
    args.extend(nss_paths.iter().map(|p| p.to_string_lossy().into_owned()));
    let status = Command::new("python3").args(&args).status().unwrap();
    assert!(status.success(), "PyKotor compile_nss failed");

    for (nss_path, s1) in nss_paths.iter().zip(sources.iter()) {
        let compiled = nss_path.with_extension("ncs");
        let bytes = std::fs::read(&compiled).unwrap();
        let ncs = kq_format::ncs::read(&bytes, &compiled).unwrap();
        let s2 = kq_ncs::decompile(&ncs, index.game).source;
        assert_eq!(s1, &s2, "fixpoint mismatch for {}", nss_path.display());
    }

    let _ = std::fs::remove_dir_all(&work);
}
