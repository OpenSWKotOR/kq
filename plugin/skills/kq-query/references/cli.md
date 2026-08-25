# kq CLI reference

Confirm live flags with `kq --help` and `kq <command> --help`. This file
matches kq 0.3.2.

## Source kinds (`-s`)

`override`, `module-mod`, `module-rim`, `lips`, `texturepack`, `rims`,
`stream`, `chitin`, `talktable`, `loose`.

Resolve order (lower precedence number wins): Override, module `.mod`,
module rim trio, lips, texture packs, `rims/`, streams, chitin BIFs,
`dialog.tlk`. Standalone targets are all `loose`.

## Recipes

```bash
kq -i "$KQ_INSTALL" info --json
kq -i "$KQ_INSTALL" which appearance.2da
kq -i "$KQ_INSTALL" ls -t utc -m danm13 --winners
kq -i "$KQ_INSTALL" cat n_bastila.utc --json
kq -i "$KQ_INSTALL" cat appearance.2da --from 'data/2da.bif' -f gron
kq -i "$KQ_INSTALL" grep Bastila -t dlg -n 10 --json
kq -i "$KQ_INSTALL" grep --ignore-case GetObjectByTag -t ncs
kq -i ./modules/danm13.mod ls -t git
kq -i "$KQ_INSTALL" unused -q
kq -i "$KQ_INSTALL" unused --summary
kq -i "$KQ_INSTALL" unused -t utc -n 40
kq -i "$KQ_INSTALL" leftovers --summary
kq -i "$KQ_INSTALL" leftovers --what strings -n 40
```

## Environment

| Variable | Role |
|----------|------|
| `KQ_INSTALL` | default `--install` |
| `KQ_CACHE_DIR` | index cache directory |

`--refresh` rebuilds and **writes** the cache. `--no-cache` neither reads nor writes.
Standalone file/folder/capsule targets skip the cache.

## Broken pipes

`kq ls | head` is success, not an error.
