# Working on this repository

- Nothing from the game goes into this repository: no ROM bytes, extracted assets or
  disassembly. The port reads game data from the user's ROM at runtime.
- The [decompilation](https://github.com/mschienbein/toejam-earl-decomp) (checkpoint c038-final)
  is the source of truth for behaviour. Every translated function names its decomp source file
  and function and keeps the decomp's name in snake_case. It preserves the original behaviour
  exactly, including integer widths and wraparound, evaluation order and original bugs; keep the
  decomp's `ORIGINAL BUG:` notes.
- Faithfulness is shown by tests against the original ROM, not by reading the code.
- Keep `cargo fmt`, `cargo clippy` and the tests clean for native targets and wasm32.

## Checks

Run the same workspace-wide checks as CI (the toolchain and wasm target are pinned):

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo build --locked --workspace --target wasm32-unknown-unknown
python -m unittest discover -s tools -p 'test_*.py' -v
python tools/audit_public.py --history --no-rom
```

The audit examines tracked worktree files, every staged blob and HEAD; `--history`
also checks all trees reachable from every Git ref and requires a non-shallow clone.
It rejects private/generated paths, binary/assets/disassembly and long encoded data.
Hex, numeric/byte atoms, escaped Rust/Python strings, Base64 and escaped JSON are
decoded: non-text payloads of 64 bytes or more are rejected by size/shape without an
entropy exemption. Shorter ROM matches and additional opaque-format heuristics
remain checked. ROM-free CI skips only comparisons with private cartridge bytes.
Tests generate both synthetic revision images and cover worktree, staged-only and
deleted historical payloads; optional real-ROM probes stay entirely in memory.

Compile-time `include_bytes!` / `include_str!` may name only a direct literal path
to a regular, tracked public input in the same audited Git state. Private, missing,
untracked and symlink inputs are rejected, as are aliased builtins and unresolved
`concat!`/`env!` expressions. Private ROM tests must continue to read files at
runtime, never embed them in test executables.

Vendored jgenesis files are governed by `vendor/jgenesis/UPSTREAM.json`, generated
from the pinned public upstream Git tree:

```sh
python tools/generate_vendor_manifest.py
```

The audit fetches the pinned upstream revision into ignored `.cache/public-audit/`
and verifies its Git objects and every manifest path/blob id. An unavailable,
uncached upstream, an unlisted vendor file or unverifiable metadata fails closed.
Exact upstream blobs are exempt from content checks; modified files are audited
only on added/replaced lines, with changed include expressions checked in their
complete source context. The local `NOTICE.md` has no upstream counterpart and
is audited in full. Path/type/size restrictions still apply to all vendored files.
The verified current manifest also covers historical trees predating its addition;
generation retains historical vendor paths. Do not hand-add content exemptions.
No exceptions are inherited from the decompilation.

Before publishing, put your own unmodified images at `roms/rev02.bin` and
`roms/rev00.bin` (ignored by Git), then run the stronger private checks:

```sh
cargo test --locked -p tje-rom --test roms -- --nocapture
python tools/audit_public.py --history
```

The ROM tests print a skip message if a private image is absent; other read errors
and incorrect images fail. The private audit requires both pinned 1 MiB ROMs and
checks their whole-image SHA-1 before comparing exact raw and decoded ROM windows.
