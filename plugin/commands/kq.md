---
name: kq
description: Query a KotOR / TSL install with the kq CLI
argument-hint: "<question or kq arguments>"
---

Answer the user's KotOR resource question by running `kq`. Follow `AGENTS.md` if present, else the `kq-query` skill.

Arguments: $ARGUMENTS

Rules:

- `-i` is `--install`. Case-insensitive grep is `--ignore-case`.
- Do not extract KEY/BIF/ERF/RIM archives.
- Prefer `--json` or `cat -f gron` for your own parsing.
- If no install is resolved, ask for a path instead of guessing.
- Treat exit 3 as an empty result.

Workflow: resolve binary and target → `which` when names collide → `cat`/`grep` as needed → quote field paths in the answer.
