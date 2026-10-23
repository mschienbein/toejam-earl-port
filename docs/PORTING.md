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
complete ABI JSON and dependency IDs. Fragments are not ordinary RTS hooks.

Generated metadata is stable input to translation lanes: **do not edit or
regenerate it to add a translation or review atomic capability**. Maintainers
regenerate only when the inventory changes; the generator reads metadata, never
ROM contents, disassembly or assets. Normal builds use committed modules, each
below the public 1-MB limit.

### Lane ownership

Each lane owns only `crates/tje-game/src/<subsystem>/` (its `mod.rs`, private child
modules and tests). The 17 inventory subsystem names have modules registered once
in `lib.rs`: `audio`, `camera`, `collision`, `core`, `cutscenes`, `earthlings`,
`elevator`, `jamout`, `map`, `platform`, `player`, `presents`, `runtime`, `ship`,
`speech`, `sprites`, `ui`. Runtime/platform assembly remains cited to G's
`src/asm`; other game code follows G's subsystem directories.

Every subsystem exports its own `TRANSLATIONS: &[Translation]` and
`ATOMIC_CAPABLE: &[FunctionId]`. Add bodies, tests and entries **in the owning
subsystem only**. Shared `SUBSYSTEMS`, the dispatcher/context, host and generated
metadata require no lane edits. `tje_game::translations()` aggregates the slices
without allocation; ownership tests reject duplicate IDs or a function registered
under another subsystem. A lane owning several subsystems edits each one's module.
If an original callee belongs to another lane, ask that owner to review/classify
it; do not add its ID to your own subsystem's list.

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
explicit reviewed atomic contract. Each subsystem's `ATOMIC_CAPABLE` slice lists
its **translated entries and any still-original callees** reviewed to return
without clock/interrupt progress. Host registration and nested original fallback
look up these local lists; generated metadata contains no atomic policy. Missing
IDs fail closed, not optimistically inferred safe from W0 membership or missing
hardware annotations. Read every relevant return/wait path before adding an ID;
record its source and reasoning in the lane's evidence. `SetupYM2612`, VBlank waits and STOP/IRQ
fixtures are rejected before atomic execution. A future wait/interrupt translation
needs an explicit time-progressing platform contract, never a fabricated ready
bit or a larger instruction bound. Even a nominally bounded VDP writer needs a
live entry state in which its frozen FIFO can accept the original access.

## 3. Register and test

Add the function ID and Rust body to the owning module's `TRANSLATIONS` slice and
classify it in that module's `ATOMIC_CAPABLE`. The host's
`HookRegistry::translated(revision)` registers all available entries using the
generated address/ABI for **both** revisions. `Host::new` rejects a registry/ROM
revision mismatch. Custom authored checker fixtures use explicit capability
arguments to `register_contract` / `call_original_with_contract`, separate from
inventory metadata; production registration always consults the owning module.

For fast lane iteration, check only your subsystem(s) or an explicit list of
canonical function names/inventory IDs (`func_...`):

```sh
cargo run --release -p tje-host -- --rom "$P/roms/rev02.bin" --mode check \
  --subsystem sprites --frames 10000 \
  --input crates/tje-host/examples/moving-player.json --require-game \
  --metrics "$P/scratch/MyLane/rev02.json"
cargo run --release -p tje-host -- --rom "$P/roms/rev00.bin" --mode check \
  --subsystem collision,camera,elevator --frames 10000 \
  --input crates/tje-host/examples/start-game.json --require-game \
  --metrics "$P/scratch/MyLane/rev00.json"
cargo run --release -p tje-host -- --rom "$P/roms/rev02.bin" --mode check \
  --function GetRandomNumber,func_004ee8 --frames 1300 \
  --input crates/tje-host/examples/start-game.json --require-game
```

`--subsystem` and `--function` are repeatable/comma-separated and combine by
**union**. Unknown subsystems/functions and requested untranslated IDs are errors,
not silent omissions. Selection is check-mode-only. It limits entry hooks, **not
callee dispatch**: a selected Rust caller still invokes every available Rust
callee, even outside the selection. `HookRegistry::filtered(revision, subsystems,
ids)` provides the same behavior for lane-owned fixture tests. Only checked hook
entries contribute per-function coverage; a Rust callee invoked inside one is not
independently counted as checked.

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
as exact RAM equivalence. Any read-before-write of such bytes **in either the
original or Rust body** fails approval, including mirrored RAM, typed Bus views,
the immediate hardware facade and nested original callees. Checked Rust writes
mark bytes initialized; a wrong Rust body cannot pass by reading a spill left by
an earlier original call whose Rust replacement omitted it. Residual stack state
must be modeled before replacement is safe. Check mode does not claim that
atomic replacement is CPU-cycle-equivalent.

## 4. Drive and report coverage honestly

Every report initializes an entry for every registered hook, including zero calls,
and every subsystem. `by_subsystem` / CLI `coverage_by_subsystem` reports registered
function count, checked/replaced calls, mismatches and explicitly unexercised
functions; an unselected subsystem has zero registered entries, not coverage of
its untranslated functions. Per-function `calls_checked`, `calls_replaced`,
`mismatches` and global `unexercised` remain available. A zero-mismatch run is not
evidence for a zero-call function. The CLI writes metrics even after a mismatch.

Drive unexercised paths with recorded controller inputs that select the relevant
menu/player/present/level. For rare and boundary paths, use `Host::check_fixture` on a
fork of live state after reaching gameplay, with correct stack arguments or D0/D1
register inputs. Label these **live-state fixtures**, separately from natural
session calls; do not claim the normal game generated their synthetic inputs.
Put lane-specific live-state fixture tests inside your owned subsystem, using a
separate opt-in host harness when necessary; do not add fixtures to another lane's
module or rewrite the shared pilot acceptance test. Report natural session calls
and added fixture calls separately. A filtered 10,000-frame session plus labeled
live-state fixtures must exercise every delivered hook in both revisions; leave
any zero-call entry explicitly unexercised rather than claiming approval.
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

- commit, owned paths, translated IDs, wave/subsystems, decomp sources and ABI decisions;
- atomic-capability additions with source-backed no-wait reasoning and original callees;
- exact commands and hook filters, toolchain, frames, revision and gameplay milestones;
- per-subsystem summary plus per-function natural/fixture calls, mismatches and unexercised IDs;
- whole-session comparison results, dead-stack differences/audited dependencies,
  nested fallback counts, exceptional or time-dependent paths not exercised;
- native startup closure result, tests/build/lint/public-audit results, and open
  issues clearly distinguished from observed evidence.

Never commit ROM bytes, extracted assets, recorded RAM, screenshots or audio.
Only metadata, original Rust translations, synthetic tests and textual coverage
counts belong in the public repository.

## 5. Non-atomic logic checks and time-progressing waits

The atomic-only restriction in section 2 is the default, not a reason to label a
hardware wait atomic. A reviewed execution contract uses
`tje_game::execution::ExecutionClass`:

| Class | Execution on check forks | What must be reviewed |
| --- | --- | --- |
| `Atomic` | Original CPU instructions and Rust, peripheral clocks frozen; existing full state/trace comparison | Every return path and original callee returns without time or interrupt progress; the live FIFO can accept all accesses |
| `Functional` | Both forks use immediate device transaction completion, clocks otherwise frozen; existing full state/trace comparison | No raster/status/HV/FIFO/DMA-busy observation, timing-dependent concurrent observer, unstable DMA source, or Z80-mailbox retry requiring the Z80 to run |
| `TimeAware` | Independent original and Rust forks run real clocks; a Rust wait suspends the current emulated frame while original IRQ handlers execute | Explicit conditions and input timeline; identical ordered transactions, RAM and ABI outputs at return; every observed read still compares exactly |

Existing subsystem `ATOMIC_CAPABLE` lists retain their meaning. Do not put a
functional or time-aware ID there to bypass registration. Authored proofs use
`HookRegistry::register_execution_contract` with an explicit class. New production
registrations likewise need an explicit reviewed class; missing declarations fail
closed. An atomic or functional context cannot invoke a time-progressing wait.

### Functional device semantics and their limits

`GenesisBus::hybrid_set_functional_mode(true)` is enabled on check forks only.
Normal reference/check-live/replace emulation keeps the upstream timing. VDP
transactions use the upstream FIFO commit and DMA progression routines without
waiting for raster access slots. Existing pending transfers are completed on
entry; each later write completes before the next transaction. DMA source wrapping,
zero-length meaning 65,536 words, length/source/address updates, sprite-cache
updates and command latches use those same routines, not a second DMA algorithm.

BUSACK is **not** forced ready. The vendored model derives it combinationally as
`busreq && !reset`, with active-low bit 8 and the original open-bus unused bits.
Requesting the bus while reset remains asserted still does not grant access.
YM register writes already commit synchronously in this model; only the fixed
32-internal-cycle BUSY countdown delays readiness. Functional mode settles that
countdown, without changing timer flags, status-latch/port-read semantics,
oscillators or audio samples. This is an eventual-ready quotient, not an invented
status value. Code which observes elapsed YM timer/sample time is not eligible.

For a closed, stable-source transaction stream with no time-dependent observation,
FIFO stalls and DMA/BUSY latency insert only stuttering steps: deleting those steps
preserves transaction order and the committed logical state. This proves **logic**
equivalence for that stream. It does not prove scanline-relative writes, rendered
pixels, sound timing, interrupt interleaving, DMA-source races, mailbox retries or
even the number of idle random draws. Those are separate session/scheduler
obligations. Direct VDP status/HV reads in functional mode are recorded as contract
violations, not masked or given fabricated ready bits.

### Wait API and hybrid suspension

`WaitRequest::vblank_frame(address, previous)` expresses a RAM-word change;
`WaitRequest::{dma_done,z80_busack,ym2612_ready}` express explicit device masks.
`Platform::wait_until(request)` and `wait_vblank_frame(address, previous)` return
`Result<()>`. Their default implementation rejects unsupported scheduling.
`max_ticks` is an error bound, never a synthetic cycle charge or permission to
resume with an unsatisfied condition. A native implementation uses its frame and
device scheduler; no emulator type or ROM instruction address enters this trait.

The hybrid implementation parks the CPU in a synthetic idle loop in disjoint,
temporarily reserved stack space, retaining the suspended frame's SP and original
SR interrupt mask. It advances the real emulator, including DMA, Z80, sound, video
and original interrupt handlers. Rust resumes only after the condition holds and
the CPU has returned from all handlers to the parked SP/PC. Parking bytes are
restored on success and failure. IRQ reads/writes are merged into the same
chronological RAM/device trace and initialized-stack audit as ordinary execution;
an interrupt does not reset or replace the current frame's audit bitmap.

A wait spanning a video-frame boundary requires a `WaitInputPoller`; the host
polls the timeline for each crossed frame. `InputScript` implements that trait.
Missing samples/pollers are errors in check/replay, not silent reuse of the entry
frame's controller state. Generated audio and crossed-frame counts are retained;
they are not discarded just because Rust is suspended.

### Exactly what a time-aware check proves

The original runs to return with clocks active. Rust starts from an identical
fork and advances real clocks through its platform waits. Return PC/SP, banked
stacks, declared register/SR outputs, live RAM, ordered device reads/writes and RAM
at each device write remain obligations. Logical VDP memories/registers/port
latches are compared after pending transactions complete on comparison forks.
Raster/HV phase, FIFO occupancy/latency, render caches/pixels, independently
progressing Z80 state and audio phase are **not** time-aware function outputs.
These exclusions never hide an observed read: status/HV/BUSACK/YM operands and
polling counts remain in the transaction trace. A generic device wait whose
polling sequence differs therefore fails closed; only a separately reviewed
whole-poll-loop primitive may define a narrower observation contract.

`WaitForNextVBlank` does not draw idle random numbers: it waits on the
IRQ-incremented counter, and each original ISR must actually finish. Do not add
`GetRandomNumber` draws to that primitive. The main loop's separate idle draws
still require exact recorded counts; lag and interrupt phase still require
logic-tick/session evidence. A passing function-level logic check is not a
timing-equivalence claim for a native scheduler or for replacement play.


[decomp]: https://github.com/mschienbein/toejam-earl-decomp
