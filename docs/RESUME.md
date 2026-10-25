# Resuming work on the port

The entry point for anyone picking this project up. It says where things live, how to verify
them, and how changes are admitted. Current progress and the next steps are in
[STATUS](STATUS.md); the design is in [PLAN](PLAN.md).

## The two layers

- **The shipped game** is a cycle-exact engine compiled at first launch from the user's own ROM:
  Cranelift on desktop (`tje-engine-cranelift`), generated WebAssembly in the browser
  (`tje-engine-wasmgen`). Both share `tje-engine-core` and `tje-engine-runtime`, plus
  `tje-native-platform` for the VDP, YM2612, SN76489 and Z80. No 68000 interpreter ships.
  See [ENGINE](ENGINE.md) and [NATIVE](NATIVE.md).
- **The readable layer** (`tje-game`) is the faithful Rust translation of every catalogued
  decompilation function. Check mode in `tje-host` proves it function by function on real game
  states, against the original ROM running in the vendored jgenesis reference. See
  [TRANSLATION](TRANSLATION.md) for the translation contract, [PORTING](PORTING.md) for the
  admission rules and [FUNCTIONS](FUNCTIONS.md) for the generated catalogue of all 648 entries.

## Branches

- `main` is published; GitHub Pages serves the browser build from it.
- `integrate/p0` is the integration branch. Every change lands on it through one integrator after
  the full gate, then fast-forwards `main`.
- `wip/*` branches hold lane work in progress.

## Setup

- **Toolchain:** pinned in `rust-toolchain.toml`. Native dependencies and the user-local Linux,
  Windows (GNU cross) and web routes are in [TOOLCHAIN](TOOLCHAIN.md).
- **ROMs:** your own cartridge dumps at `roms/rev02.bin` and `roms/rev00.bin`. They are ignored by
  git and checked by SHA-1. Nothing ROM-derived may ever be committed; the public audit enforces
  this.
- **Decompilation:** the matching decompilation
  ([toejam-earl-decomp](https://github.com/mschienbein/toejam-earl-decomp)) is the behavioural
  source of truth. Every function row in FUNCTIONS links to its source line.

## The gate

Run all of these before any commit lands:

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo build --locked --workspace --target wasm32-unknown-unknown \
  --exclude tje-native --exclude tje-engine-cranelift --exclude tje-native-proof
uv run --python 3.12 --with-requirements tools/audit_requirements.txt python tools/trace_functions.py --check
uv run --python 3.12 --with-requirements tools/audit_requirements.txt python -m unittest discover -s tools -p 'test_*.py'
uv run --python 3.12 --with-requirements tools/audit_requirements.txt python tools/audit_public.py
```

When dependency inputs change, regenerate the third-party notices with `python3 tools/notices.py`
(see [RELEASING](RELEASING.md)). Run the history audit
(`audit_public.py --history --refs <branch>`) before every push.

The publication audit does not exempt paths or historical blobs. JSON string arrays
are identifier facts only when every member is an exact function constant, name or
inventory ID in that Git state's structurally validated 648-entry catalogue.
That catalogue's sorted unique fact set must also reproduce the SHA-256 pinned
independently in `audit_public.py`; renaming catalogue facts requires an explicit,
reviewed audit-tool pin update rather than self-authorizing metadata edits.
Only declared, unconditional metadata modules reachable from the tracked crate
root count; comments, unreferenced/untracked files and another Git state's
catalogue cannot authorize facts. Hash-shaped strings are still arbitrary encoded
data, not verified digests. Joined facts retain every format/ROM probe; only
speculative decoded-size/opacity guesses are waived. A single unknown chunk keeps
the full ordinary encoded-payload checks, including in ROM-free CI.

## How a function is admitted

1. **Write the body.** Translate it from the decompilation into its subsystem module
   (`crates/tje-game/src/<subsystem>/`), preserving behaviour and bugs.
2. **Register it as pending.** Add it to the subsystem's literal `PENDING`/`PENDING_TRANSLATIONS`
   tables, with its execution contract (`ATOMIC_CAPABLE` or a typed `EXECUTION_CONTRACTS` entry).
3. **Prove it in check mode** in both revisions, on natural session states and labelled real-state
   fixtures. Functions that take real time in the original consume engine timing through
   `tje_game::timing::TimingSource` and are proven with the strict clocked comparison.
4. **Promote it.** Move it to production (`TRANSLATIONS`) only after it passes check mode over
   **every** committed session in both revisions. A production entry that later fails any session
   goes back to pending with its exact failure recorded; failures are never masked or waived.

Whatever fails or is unproven stays pending, with an exact, evidence-backed blocker in its table
comment and in FUNCTIONS.

## Sessions and engine verification

`sessions/` holds input-only power-on recordings. Each replays exactly between two independent
reference runs; their coverage is reported over the full 648-entry denominator. Recording,
replay, comparison and the bot are covered in [SESSIONS](SESSIONS.md). The engine's acceptance
matrix (`tje-engine-verify accept`) replays every session through the native and WebAssembly
engines in both revisions and compares every frame, RAM commit, chip write and audio sample with
the reference ([PLAN](PLAN.md), "Verification").

## Releasing

[RELEASING](RELEASING.md) covers packages, checksums, notices and the tag-driven workflows.
Release workflows run only for `v*` tags or a manual dispatch on `main`.
