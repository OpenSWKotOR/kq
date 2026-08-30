//! Structural compare, patch, and three-way merge of JSON values.
//!
//! Ops are RFC 6902 JSON Patch (`add` / `remove` / `replace`) plus an `old`
//! field so a delta can be inverted and printed. Paths are JSON Pointers
//! (RFC 6901). Arrays of objects keyed by `_row`, `strref`, `name`, or `Tag`
//! are aligned by that field; other arrays are compared by index.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value as J};

const ALIGN_KEYS: &[&str] = &["_row", "strref", "name", "Tag"];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Op {
    Add {
        path: String,
        value: J,
    },
    Remove {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        old: Option<J>,
    },
    Replace {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        old: Option<J>,
        value: J,
    },
}

impl Op {
    pub fn path(&self) -> &str {
        match self {
            Self::Add { path, .. } | Self::Remove { path, .. } | Self::Replace { path, .. } => path,
        }
    }
}

/// Compare `left` to `right`. Empty means the trees are equal.
pub fn diff(left: &J, right: &J) -> Vec<Op> {
    let mut ops = Vec::new();
    diff_at(left, right, "", &mut ops);
    ops
}

fn diff_at(left: &J, right: &J, prefix: &str, ops: &mut Vec<Op>) {
    if left == right {
        return;
    }
    match (left, right) {
        (J::Object(a), J::Object(b)) => {
            for (k, lv) in a {
                let p = join(prefix, k);
                match b.get(k) {
                    Some(rv) => diff_at(lv, rv, &p, ops),
                    None => ops.push(Op::Remove {
                        path: p,
                        old: Some(lv.clone()),
                    }),
                }
            }
            for (k, rv) in b {
                if !a.contains_key(k) {
                    ops.push(Op::Add {
                        path: join(prefix, k),
                        value: rv.clone(),
                    });
                }
            }
        }
        (J::Array(a), J::Array(b)) => diff_arrays(a, b, prefix, ops),
        _ => ops.push(Op::Replace {
            path: prefix.to_string(),
            old: Some(left.clone()),
            value: right.clone(),
        }),
    }
}

fn diff_arrays(left: &[J], right: &[J], prefix: &str, ops: &mut Vec<Op>) {
    if let Some(key) = align_key(left).and_then(|k| align_key(right).filter(|r| *r == k)) {
        diff_aligned(left, right, key, prefix, ops);
        return;
    }
    let n = left.len().min(right.len());
    for i in 0..n {
        diff_at(&left[i], &right[i], &join_index(prefix, i), ops);
    }
    // Remove extra left tail from the end so later apply stays aligned.
    for i in (n..left.len()).rev() {
        ops.push(Op::Remove {
            path: join_index(prefix, i),
            old: Some(left[i].clone()),
        });
    }
    for item in right.iter().skip(n) {
        ops.push(Op::Add {
            path: join(prefix, "-"),
            value: item.clone(),
        });
    }
}

fn diff_aligned(left: &[J], right: &[J], key: &str, prefix: &str, ops: &mut Vec<Op>) {
    let right_by: std::collections::BTreeMap<String, &J> = right
        .iter()
        .filter_map(|v| key_of(v, key).map(|k| (k, v)))
        .collect();
    let left_keys: std::collections::BTreeSet<String> =
        left.iter().filter_map(|v| key_of(v, key)).collect();

    let mut removals = Vec::new();
    for (i, lv) in left.iter().enumerate() {
        let Some(k) = key_of(lv, key) else {
            continue;
        };
        match right_by.get(&k) {
            Some(rv) => diff_at(lv, rv, &join_index(prefix, i), ops),
            None => removals.push(i),
        }
    }
    for i in removals.into_iter().rev() {
        ops.push(Op::Remove {
            path: join_index(prefix, i),
            old: Some(left[i].clone()),
        });
    }
    for rv in right {
        let Some(k) = key_of(rv, key) else {
            continue;
        };
        if !left_keys.contains(&k) {
            ops.push(Op::Add {
                path: join(prefix, "-"),
                value: rv.clone(),
            });
        }
    }
}

fn align_key(arr: &[J]) -> Option<&'static str> {
    if arr.is_empty() {
        return None;
    }
    for key in ALIGN_KEYS {
        let mut seen = std::collections::BTreeSet::new();
        let ok = arr.iter().all(|v| match key_of(v, key) {
            Some(k) => seen.insert(k),
            None => false,
        });
        if ok {
            return Some(key);
        }
    }
    None
}

fn key_of(v: &J, field: &str) -> Option<String> {
    match v.get(field)? {
        J::String(s) => Some(s.clone()),
        J::Number(n) => Some(n.to_string()),
        J::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Apply ops to `target` in order. `old` fields are ignored.
pub fn apply(target: &mut J, ops: &[Op]) -> Result<(), String> {
    for op in ops {
        match op {
            Op::Add { path, value } => add(target, path, value.clone())?,
            Op::Remove { path, .. } => {
                remove(target, path)?;
            }
            Op::Replace { path, value, .. } => {
                let slot = target
                    .pointer_mut(path)
                    .ok_or_else(|| format!("replace: no value at {path}"))?;
                *slot = value.clone();
            }
        }
    }
    Ok(())
}

fn add(root: &mut J, path: &str, value: J) -> Result<(), String> {
    if path.is_empty() {
        *root = value;
        return Ok(());
    }
    let (parent_ptr, token) = split_parent(path)?;
    let parent = if parent_ptr.is_empty() {
        root
    } else {
        root.pointer_mut(parent_ptr)
            .ok_or_else(|| format!("add: no parent at {parent_ptr}"))?
    };
    match parent {
        J::Object(map) => {
            map.insert(token, value);
        }
        J::Array(arr) => {
            if token == "-" {
                arr.push(value);
            } else {
                let i: usize = token
                    .parse()
                    .map_err(|_| format!("add: bad array index {token}"))?;
                if i > arr.len() {
                    return Err(format!("add: index {i} past end of array"));
                }
                arr.insert(i, value);
            }
        }
        _ => return Err(format!("add: parent at {parent_ptr} is not a container")),
    }
    Ok(())
}

fn remove(root: &mut J, path: &str) -> Result<J, String> {
    if path.is_empty() {
        return Err("remove: cannot remove root".into());
    }
    let (parent_ptr, token) = split_parent(path)?;
    let parent = if parent_ptr.is_empty() {
        root
    } else {
        root.pointer_mut(parent_ptr)
            .ok_or_else(|| format!("remove: no parent at {parent_ptr}"))?
    };
    match parent {
        J::Object(map) => map
            .remove(&token)
            .ok_or_else(|| format!("remove: missing key {token}")),
        J::Array(arr) => {
            let i: usize = token
                .parse()
                .map_err(|_| format!("remove: bad array index {token}"))?;
            if i >= arr.len() {
                return Err(format!("remove: index {i} past end of array"));
            }
            Ok(arr.remove(i))
        }
        _ => Err(format!("remove: parent at {parent_ptr} is not a container")),
    }
}

/// Three-way merge. Conflicts become `{ "_conflict": true, "base", "ours", "theirs" }`.
pub fn merge(base: &J, ours: &J, theirs: &J) -> (J, usize) {
    let mut conflicts = 0;
    let value = merge_at(base, ours, theirs, &mut conflicts);
    (value, conflicts)
}

fn merge_at(base: &J, ours: &J, theirs: &J, conflicts: &mut usize) -> J {
    if ours == theirs {
        return ours.clone();
    }
    if ours == base {
        return theirs.clone();
    }
    if theirs == base {
        return ours.clone();
    }
    match (base, ours, theirs) {
        (J::Object(b), J::Object(o), J::Object(t)) => {
            let mut out = Map::new();
            let mut keys: Vec<&String> = b.keys().chain(o.keys()).chain(t.keys()).collect();
            keys.sort();
            keys.dedup();
            for k in keys {
                let empty = J::Null;
                let bv = b.get(k).unwrap_or(&empty);
                let ov = o.get(k).unwrap_or(&empty);
                let tv = t.get(k).unwrap_or(&empty);
                if b.get(k).is_none() && o.get(k).is_none() {
                    out.insert(k.clone(), tv.clone());
                    continue;
                }
                if b.get(k).is_none() && t.get(k).is_none() {
                    out.insert(k.clone(), ov.clone());
                    continue;
                }
                if o.get(k).is_none() && t.get(k).is_none() {
                    continue;
                }
                out.insert(k.clone(), merge_at(bv, ov, tv, conflicts));
            }
            J::Object(out)
        }
        (J::Array(b), J::Array(o), J::Array(t)) if b.len() == o.len() && o.len() == t.len() => {
            let items: Vec<J> = b
                .iter()
                .zip(o.iter())
                .zip(t.iter())
                .map(|((bv, ov), tv)| merge_at(bv, ov, tv, conflicts))
                .collect();
            J::Array(items)
        }
        _ => {
            *conflicts += 1;
            json!({
                "_conflict": true,
                "base": base,
                "ours": ours,
                "theirs": theirs,
            })
        }
    }
}

/// Prefer one side at every conflict node (`_conflict: true`).
pub fn resolve_conflicts(value: &J, take: ConflictSide) -> J {
    match value {
        J::Object(map) if map.get("_conflict") == Some(&J::Bool(true)) => {
            let key = match take {
                ConflictSide::Ours => "ours",
                ConflictSide::Theirs => "theirs",
                ConflictSide::Base => "base",
            };
            map.get(key).cloned().unwrap_or(J::Null)
        }
        J::Object(map) => J::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), resolve_conflicts(v, take)))
                .collect(),
        ),
        J::Array(items) => J::Array(items.iter().map(|v| resolve_conflicts(v, take)).collect()),
        other => other.clone(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictSide {
    Ours,
    Theirs,
    Base,
}

/// JSON Pointer → gron-style path (`/ClassList/0/Class` → `ClassList[0].Class`).
pub fn pointer_to_gron(ptr: &str) -> String {
    if ptr.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for token in ptr.split('/').skip(1) {
        let token = unescape(token);
        if token.chars().all(|c| c.is_ascii_digit()) && !token.is_empty() {
            out.push('[');
            out.push_str(&token);
            out.push(']');
        } else if token.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !token.is_empty()
        {
            if !out.is_empty() {
                out.push('.');
            }
            out.push_str(&token);
        } else {
            out.push('[');
            out.push_str(&J::String(token).to_string());
            out.push(']');
        }
    }
    out
}

pub fn join(prefix: &str, key: &str) -> String {
    format!("{prefix}/{}", escape(key))
}

fn join_index(prefix: &str, i: usize) -> String {
    format!("{prefix}/{i}")
}

fn split_parent(path: &str) -> Result<(&str, String), String> {
    let slash = path
        .rfind('/')
        .ok_or_else(|| format!("bad pointer: {path}"))?;
    Ok((&path[..slash], unescape(&path[slash + 1..])))
}

fn escape(s: &str) -> String {
    s.replace('~', "~0").replace('/', "~1")
}

fn unescape(s: &str) -> String {
    s.replace("~1", "/").replace("~0", "~")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_and_apply() {
        let left = json!({"Tag": "a", "HP": 10});
        let right = json!({"Tag": "b", "HP": 10});
        let ops = diff(&left, &right);
        assert_eq!(ops.len(), 1);
        let mut got = left;
        apply(&mut got, &ops).unwrap();
        assert_eq!(got, right);
    }

    #[test]
    fn add_remove_roundtrip() {
        let left = json!({"a": 1});
        let right = json!({"b": 2});
        let ops = diff(&left, &right);
        let mut got = left;
        apply(&mut got, &ops).unwrap();
        assert_eq!(got, right);
    }

    #[test]
    fn twoda_aligns_on_row_label() {
        let left = json!([{"_row": "0", "label": "a"}, {"_row": "2", "label": "gone"}]);
        let right = json!([{"_row": "0", "label": "A"}, {"_row": "3", "label": "new"}]);
        let ops = diff(&left, &right);
        let mut got = left;
        apply(&mut got, &ops).unwrap();
        assert_eq!(got.as_array().unwrap().len(), 2);
        assert_eq!(got[0]["label"], "A");
        assert_eq!(got[1]["_row"], "3");
    }

    #[test]
    fn merge_takes_changed_side() {
        let base = json!({"Tag": "x", "HP": 10});
        let ours = json!({"Tag": "x", "HP": 12});
        let theirs = json!({"Tag": "y", "HP": 10});
        let (out, n) = merge(&base, &ours, &theirs);
        assert_eq!(n, 0);
        assert_eq!(out["Tag"], "y");
        assert_eq!(out["HP"], 12);
    }

    #[test]
    fn merge_conflict_is_marked() {
        let base = json!({"Tag": "x"});
        let ours = json!({"Tag": "a"});
        let theirs = json!({"Tag": "b"});
        let (out, n) = merge(&base, &ours, &theirs);
        assert_eq!(n, 1);
        assert_eq!(out["Tag"]["_conflict"], true);
        let resolved = resolve_conflicts(&out, ConflictSide::Ours);
        assert_eq!(resolved["Tag"], json!("a"));
    }

    #[test]
    fn pointer_gron() {
        assert_eq!(pointer_to_gron("/ClassList/0/Class"), "ClassList[0].Class");
        assert_eq!(pointer_to_gron("/Tag"), "Tag");
    }
}
