//! NCS → NSS decompiler (DeNCS algorithm port).

mod fallback;

pub use kq_index::Game;

#[derive(Clone, Debug)]
pub struct Decompiled {
    pub source: String,
    pub complete: bool,
    pub warnings: Vec<Warning>,
    pub subs: Vec<SubReport>,
}

#[derive(Clone, Debug)]
pub struct Warning {
    pub sub: Option<u16>,
    pub pos: Option<u32>,
    pub severity: Severity,
    pub msg: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug)]
pub struct SubReport {
    pub id: SubId,
    pub start: u32,
    pub end: u32,
    pub status: SubStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubId {
    Header,
    Globals,
    Main,
    User(u16),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubStatus {
    Ok,
    Fallback(String),
}

/// Never panics. Never returns Err.
pub fn decompile(ncs: &kq_format::ncs::Ncs, _game: Game) -> Decompiled {
    let disasm = fallback::disasm_lines(ncs);
    let mut indented = String::new();
    for line in disasm.lines() {
        indented.push_str("     ");
        indented.push_str(line);
        indented.push('\n');
    }
    let source = format!(
        "void main() {{\n\t/* kq: decompiler stub — full pipeline not yet wired.\n\t   Disassembly:\n{indented}\t*/\n}}\n"
    );
    let (start, end) = ncs
        .instructions
        .first()
        .map(|first| {
            let end = ncs
                .instructions
                .last()
                .map(|last| last.offset)
                .unwrap_or(first.offset);
            (first.offset, end)
        })
        .unwrap_or((0, 0));
    Decompiled {
        source,
        complete: false,
        warnings: Vec::new(),
        subs: vec![SubReport {
            id: SubId::Main,
            start,
            end,
            status: SubStatus::Fallback("stub disassembly".into()),
        }],
    }
}

pub fn disasm_comment(ncs: &kq_format::ncs::Ncs) -> String {
    fallback::disasm_lines(ncs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kq_format::ncs::{self, Ncs};
    use std::path::Path;

    fn minimal_main_ncs() -> Ncs {
        // NCS V1.0 + magic + size, then: JSR ->21, RETN, RETN (empty main)
        let mut data = b"NCS V1.0".to_vec();
        data.push(0x42);
        let body = [
            0x1E, 0x00, 0x00, 0x00, 0x00, 0x08, // JSR +8 → offset 21
            0x20, 0x00,                         // RETN header
            0x20, 0x00,                         // RETN main
        ];
        let size = (13 + body.len()) as u32;
        data.extend_from_slice(&size.to_be_bytes());
        data.extend_from_slice(&body);
        ncs::read(&data, Path::new("t.ncs")).unwrap()
    }

    #[test]
    fn decompile_never_empty_and_never_panics() {
        let d = decompile(&minimal_main_ncs(), Game::K1);
        assert!(!d.source.is_empty());
        assert!(d.source.contains("/*") || d.source.contains("void") || d.source.contains("RETN"));
    }
}
