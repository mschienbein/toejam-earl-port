# Porting one function batch

The behavioral authority is the matching [decompilation][decomp] (**G**), not a
modernized interpretation of its C. Read [TRANSLATION.md](TRANSLATION.md),
[PLAN.md](PLAN.md), and [TOOLCHAIN.md](TOOLCHAIN.md) before translating.

## 1. Choose a closed batch

Use the private inventory's `WAVES.md` and `functions.json`: start with W0 runtime
helpers, then independent leaf batches. A wave is a dependency ordering, not
coverage evidence. Keep shared bodies together (notably `ulmodt`/`uldivt`). Inspect
both revisions' source branches, ABI, dependencies, indirect targets and hardware
uses. The committed `tje_game::functions()` metadata covers all 648 catalogue
entries (645 functions and three fragments), with stable inventory identities,
revision addresses/sizes, source path/line, classification, wave, signature,
complete ABI JSON, dependency IDs and a reviewed `atomic_capable` flag. Fragments are not ordinary RTS hooks.

The metadata generator reads the inventory, never ROM contents, disassembly or
assets. Normal builds use committed modules, each below the public 1-MB limit:

```sh
python3 crates/tje-game/tools/generate_metadata.py --inventory "$P/scratch/Inventory/functions.json"
python3 crates/tje-game/tools/generate_metadata.py --inventory "$P/scratch/Inventory/functions.json" --check
```

## 2. Translate into `tje-game`

Put game bodies in `crates/tje-game`, which depends only on `tje-mem`. Cite the G
source path and function/line in every translation's Rust doc comment. Use the
existing typed RAM/ROM views, emulated `Address`/`Ptr`, wrapping arithmetic,
MCC68K promotions, and the actual evaluation/store order. Do not copy assets into
Rust, create native pointers from ROM addresses, or silently fix original bugs.

A `Context` combines:

- `tje_mem::Bus` for side-effect-free ROM/RAM reads and typed views;
- `tje_game::Platform` (extending `tje_mem::Platform`) for width-preserving VDP
  ports, YM2612, PSG, Z80 RAM/bus request/reset and controller ports; its primitive
  device operations use the real platform;
- original residual data registers and a dispatcher.

Each translation receives raw four-byte argument slots and returns `Option<u32>`
for D0 (`None` for a void C result). Narrow parameters at the callee's declared
width. Do not zero/sign extend a narrow return unless the matching implementation
does: preserve residual D0 bits when required. Register-runtime entries take
D0/D1, not invented stack arguments. Division by zero propagates an explicit
`ArithmeticTrap::DivideByZero` (68000 vector 5), never a fabricated result.

Call another game function with `tje_game::call(context, ids::FUNCTION, arguments)`.
The dispatcher runs available Rust translations. A hybrid missing callee calls
`Context::call_original` using that revision's original address; the host pushes
real MCC68K slots and waits for both the sentinel PC and matching SP. No emulator
or host dependency enters the game crate. A native platform must construct
`Dispatcher::native(roots)` at startup: it validates the entire static dependency
closure and lists all missing translations. Even an out-of-closure call on that
dispatcher returns `Error::Untranslated`, never a ROM fallback. Callback roots
must be included in the closure through the inventory's indirect dependencies.

Do not translate time-dependent waits using the atomic bridge: peripheral clocks
are frozen inside check calls. Registration and nested original calls require an
explicit `atomic_capable` contract. The generator's reviewed allow-list contains
the pilot plus the nonwaiting reset-register/interrupt-control fixtures; unknown
entries fail closed, not optimistically inferred safe from W0 membership or
missing hardware annotations. Classify a new batch and its original callees only
after inspecting their return paths. `SetupYM2612`, VBlank waits and STOP/IRQ
fixtures are rejected before atomic execution. A future wait/interrupt translation
needs an explicit time-progressing platform contract, never a fabricated ready
bit or a larger instruction bound. Even a nominally bounded VDP writer needs a
live entry state in which its frozen FIFO can accept the original access.

## 3. Register and test

Add the function ID and its Rust entry to `tje_game::TRANSLATIONS`. The host's
`HookRegistry::translated(revision)` registers every available translation using
the generated address/ABI for **both** revisions; there is no separate REV 02-only
demo table. `Host::new` rejects a registry/ROM revision mismatch. The CLI identifies
the user's cartridge automatically. Custom host instrumentation hooks remain
explicit; they do not hold game implementations. `register_contract` owns authored
synthetic contracts (with ABI and atomic capability) for checker regression tests;
production registration borrows generated metadata without allocating.

Run ROM-free checks first, then the private differential tests. From the repo root
(`P` is this checkout; all generated evidence stays in ignored scratch):

```sh
export CARGO_TARGET_DIR="$P/scratch/Porting/target"
export CARGO_BUILD_JOBS=8
cargo fmt --all --check
cargo clippy -p tje-game -p tje-host --all-targets --no-deps -- -D warnings
cargo test --workspace
cargo check -p tje-game -p tje-host --all-targets --target wasm32-unknown-unknown
TJE_REV02_ROM="$P/roms/rev02.bin" TJE_REV00_ROM="$P/roms/rev00.bin" \
  cargo test -p tje-host --release --test rom -- --include-ignored --nocapture
python3 tools/audit_public.py
```

Reproducible natural-coverage sessions (the existing script skips the introduction,
selects one-player ToeJam/random world, then leaves the game running):

```sh
cargo run --release -p tje-host -- --rom "$P/roms/rev02.bin" --mode check \
  --frames 10000 --input crates/tje-host/examples/start-game.json \
  --require-title --require-game --metrics "$P/scratch/Porting/rev02.json"
cargo run --release -p tje-host -- --rom "$P/roms/rev00.bin" --mode check \
  --frames 10000 --input crates/tje-host/examples/start-game.json \
  --require-title --require-game --metrics "$P/scratch/Porting/rev00.json"
```

Check mode forks live state, executes the original and Rust atomically, and leaves
the live machine running the original instructions/timing. It compares return
PC/SP, D0 where defined, per-inventory preserved registers, SR control bits, live
RAM, Z80 RAM, all mutable VDP state (including command latch/address/code, FIFO,
DMA/pending writes, rendering caches and pixels), ordered hardware writes, and
ordered device reads/writes. Handwritten assembly compares all D/A registers and
CCR too; compiler C/runtime scratch registers and CCR are explicitly ABI-clobbered.
The RAM state at each device write is compared, so moving a RAM mirror before a
VDP write cannot pass merely because the final bytes match. Production translated
device accesses use immediate `Platform` methods or `HookContext::bus()`'s
immediate facade. Both preserve mixed byte/word order, flush before reads and
nested execution, and do not expose a mutable raw deferred bus. The upstream
instruction queues themselves are unchanged.

Differences in dead stack bytes below entry SP are counted, not silently reported
as exact RAM equivalence. Any read-before-write of such bytes, including mirrored
RAM and nested original callees, **fails approval**: the function depends on
residual stack state that has to be modeled before replacement is safe. Check
mode does not claim that atomic replacement is CPU-cycle-equivalent.

## 4. Drive and report coverage honestly

Every report initializes an entry for every registered hook, including zero calls.
It reports `calls_checked`, `calls_replaced`, `mismatches`, and the explicit
`unexercised` list. A zero-mismatch run is not evidence for a zero-call function.
The CLI writes its metrics even if a mismatch aborts the session.

Drive unexercised paths with recorded controller inputs that select the relevant
menu/player/present/level. For rare and boundary paths, use `Host::check_fixture` on a
fork of live state after reaching gameplay, with correct stack arguments or D0/D1
register inputs. Label these **live-state fixtures**, separately from natural
session calls; do not claim the normal game generated their synthetic inputs.
`check_fixture` restores CPU, RAM and hardware afterward while retaining coverage,
so repeated diagnostic writes cannot fill the live machine's frozen VDP FIFO.
Use the explicitly state-mutating `check_call` only when the fixture deliberately
needs the original's post-call state; do not chain frozen FIFO writes without
clock progress.
The committed `gameplay_10000_frames_both_revisions_and_every_pilot_hook` test
compares each session to a separate timed reference (whole RAM including stack,
VDP/pixels, audio bits and write traces), prints natural coverage, then fills
remaining pilot coverage using explicitly labeled live-state fixtures.

When a mismatch occurs, retain the revision, function ID/address, entry inputs,
exact command and first differing state/trace detail privately. Inspect the
matching compiler output and both revision sources before changing the Rust.
Add synthetic boundary/regression tests and a differential mutation test when a
checker blind spot is involved. Do not waive mismatches, mask unexplained bytes,
or count a helper skipped by the script as verified.

A batch handoff must state:

- commit, translated IDs, wave/subsystems, decomp sources and ABI decisions;
- exact commands, toolchain, frames, revision and title/menu/game milestones;
- per-function natural and fixture calls, mismatches and remaining unexercised IDs;
- whole-session comparison results, dead-stack differences/audited dependencies,
  nested fallback counts, exceptional or time-dependent paths not exercised;
- native startup closure result, tests/build/lint/public-audit results, and open
  issues clearly distinguished from observed evidence.

Never commit ROM bytes, extracted assets, recorded RAM, screenshots or audio.
Only metadata, original Rust translations, synthetic tests and textual coverage
counts belong in the public repository.

[decomp]: https://github.com/mschienbein/toejam-earl-decomp
