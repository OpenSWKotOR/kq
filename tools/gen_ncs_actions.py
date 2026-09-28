#!/usr/bin/env python3
"""Generate crates/kq-ncs/src/actions_gen.rs from TSL nwscript + ncs_actions names.

Does not use DeNCS's header regex. A `// N` line is a header only when:
- the number is not a float literal (`0.0f`), and
- no prototype is already pending (the `// 0.0f` trap), and
- binding never overwrites an occupied slot (duplicate `// 771` → 772).
Names 0..=875 are cross-checked against kq_format::ncs_actions::ACTIONS.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

PROTO_RE = re.compile(r"^\s*(\w+)\s+(\w+)\s*\((.*)\)\s*;", re.M)
HEADER_RE = re.compile(r"^\s*//\s*(\d+)(.*)$")
NAME_RE = re.compile(r'"([^"]*)"')

TY = {
    "void": "Void",
    "int": "Int",
    "float": "Float",
    "string": "Str",
    "object": "Object",
    "effect": "Effect",
    "event": "Event",
    "location": "Location",
    "talent": "Talent",
    "vector": "Vector",
    "action": "Action",
}

TABLE_LEN = 877  # ids 0..=876
K1_COUNT = 772


def rust_escape(s: str) -> str:
    return s.replace("\\", "\\\\").replace('"', '\\"')


def split_params(inner: str) -> list[str]:
    inner = inner.strip()
    if not inner:
        return []
    parts: list[str] = []
    buf: list[str] = []
    depth = 0
    in_str = False
    for ch in inner:
        if in_str:
            buf.append(ch)
            if ch == '"':
                in_str = False
            continue
        if ch == '"':
            in_str = True
            buf.append(ch)
        elif ch == "[":
            depth += 1
            buf.append(ch)
        elif ch == "]":
            depth = max(0, depth - 1)
            buf.append(ch)
        elif ch == "," and depth == 0:
            part = "".join(buf).strip()
            if part:
                parts.append(part)
            buf = []
        else:
            buf.append(ch)
    part = "".join(buf).strip()
    if part:
        parts.append(part)
    return parts


def parse_param(part: str) -> tuple[str, str | None]:
    m = re.match(r"(\w+)\s+\w+(?:\s*=\s*(.+))?$", part.strip())
    if not m:
        raise SystemExit(f"unparseable param: {part!r}")
    ty = m.group(1).lower()
    if ty not in TY:
        raise SystemExit(f"unknown param type {ty!r} in {part!r}")
    default = m.group(2).strip() if m.group(2) is not None else None
    return ty, default


def is_header(line: str, pending: int | None) -> int | None:
    if pending is not None:
        return None
    m = HEADER_RE.match(line)
    if not m:
        return None
    rest = m.group(2)
    if re.match(r"\.\d", rest):
        return None
    if rest != "" and rest[0] not in ".: \t":
        return None
    return int(m.group(1))


def parse_nss(text: str) -> dict[int, tuple[str, str, str]]:
    """id -> (ret, name, raw_params)."""
    bound: dict[int, tuple[str, str, str]] = {}
    occupied: set[int] = set()
    pending: int | None = None
    for line in text.splitlines():
        hdr = is_header(line, pending)
        if hdr is not None:
            pending = hdr
            continue
        if pending is None:
            continue
        stripped = line.strip()
        if not stripped or stripped.startswith("//"):
            continue
        pm = PROTO_RE.match(line)
        if not pm:
            pending = None
            continue
        ret, name, raw = pm.group(1), pm.group(2), pm.group(3)
        idx = pending
        pending = None
        if idx in occupied:
            while idx in occupied:
                idx += 1
        occupied.add(idx)
        bound[idx] = (ret, name, raw)
    return bound


def parse_names(text: str) -> list[str]:
    body = text.split("= &[", 1)[1].rsplit("];", 1)[0]
    return NAME_RE.findall(body)


def collect_protos(text: str) -> dict[str, tuple[str, str]]:
    out: dict[str, tuple[str, str]] = {}
    for m in PROTO_RE.finditer(text):
        out[m.group(2)] = (m.group(1), m.group(3))
    return out


def emit_sig(ret: str, name: str, raw_params: str) -> str:
    ret_key = ret.lower()
    if ret_key not in TY:
        raise SystemExit(f"unknown return type {ret!r} for {name}")
    params = split_params(raw_params)
    parsed = [parse_param(p) for p in params]
    if not parsed:
        param_src = "&[]"
    else:
        pieces = []
        for ty, default in parsed:
            if default is None:
                def_src = "None"
            else:
                def_src = f'Some("{rust_escape(default)}")'
            pieces.append(f"ParamSig {{ ty: Ty::{TY[ty]}, default: {def_src} }}")
        inner = ", ".join(pieces)
        param_src = f"&[{inner}]"
    return (
        f'Some(ActionSig {{ name: "{rust_escape(name)}", '
        f"ret: Ty::{TY[ret_key]}, params: {param_src} }})"
    )


def generate(tsl: str, names_rs: str) -> str:
    bound = parse_nss(tsl)
    names = parse_names(names_rs)
    protos = collect_protos(tsl)

    mismatches = []
    for i, name in enumerate(names):
        got = bound.get(i)
        if got is None or got[1] != name:
            mismatches.append((i, None if got is None else got[1], name))
    if mismatches:
        preview = mismatches[:10]
        raise SystemExit(f"name cross-check failed ({len(mismatches)}): {preview}")

    slots: list[tuple[str, str, str] | None] = [None] * TABLE_LEN
    for idx, triple in bound.items():
        if idx >= TABLE_LEN:
            raise SystemExit(f"action id {idx} exceeds table length {TABLE_LEN}")
        slots[idx] = triple

    # Unnumbered trailing prototype (RebuildPartyTable) → next free id.
    bound_names = {t[1] for t in bound.values()}
    for name, (ret, raw) in protos.items():
        if name in bound_names or ret.lower() not in TY:
            continue
        idx = next((i for i, s in enumerate(slots) if s is None), None)
        if idx is None:
            raise SystemExit(f"no free slot for extra prototype {name}")
        slots[idx] = (ret, name, raw)
        bound_names.add(name)

    lines = [
        "// @generated by tools/gen_ncs_actions.py — do not edit.",
        "use crate::actions::{ActionSig, ParamSig};",
        "use crate::ty::Ty;",
        "",
        f"pub(crate) static ACTIONS_GEN: [Option<ActionSig>; {TABLE_LEN}] = [",
    ]
    for slot in slots:
        if slot is None:
            lines.append("    None,")
        else:
            ret, name, raw = slot
            lines.append(f"    {emit_sig(ret, name, raw)},")
    lines.append("];")
    lines.append("")
    return "\n".join(lines)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument(
        "--nss",
        type=Path,
        default=Path("vendor/nwscript/tsl_nwscript.nss"),
    )
    ap.add_argument(
        "--names",
        type=Path,
        default=Path("crates/kq-format/src/ncs_actions.rs"),
    )
    ap.add_argument(
        "--out",
        type=Path,
        default=Path("crates/kq-ncs/src/actions_gen.rs"),
    )
    args = ap.parse_args()
    src = generate(args.nss.read_text(encoding="utf-8"), args.names.read_text(encoding="utf-8"))
    args.out.write_text(src, encoding="utf-8")
    print(f"wrote {args.out} ({TABLE_LEN} slots, K1 prefix {K1_COUNT})", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
