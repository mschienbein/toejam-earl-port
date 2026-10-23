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
python tools/audit_public.py --history --no-rom
```

The audit examines tracked worktree files, every staged blob and HEAD; `--history`
also checks all trees reachable from every Git ref and requires a non-shallow clone.
It rejects private/generated paths, binary/assets/disassembly and long encoded data.
ROM-free CI skips only comparisons with private cartridge bytes, not these checks.
No path- or historical-blob exceptions are inherited from the decompilation.

Before publishing, put your own unmodified images at `roms/rev02.bin` and
`roms/rev00.bin` (ignored by Git), then run the stronger private checks:

```sh
cargo test --locked -p tje-rom --test roms -- --nocapture
python tools/audit_public.py --history
```

The ROM tests print a skip message if a private image is absent; other read errors
and incorrect images fail. The private audit requires both pinned 1 MiB ROMs and
checks their whole-image SHA-1 before comparing exact raw and decoded ROM windows.
