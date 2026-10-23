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

Every subsystem exports `TRANSLATIONS: &[Translation]`,
`ATOMIC_CAPABLE: &[FunctionId]` and
`EXECUTION_CONTRACTS: &[(FunctionId, ExecutionClass)]`. The latter contains only
reviewed Functional/TimeAware entries, translated **or still original**. Add bodies,
tests and declarations in the owning subsystem only. Shared `SUBSYSTEMS`, the
dispatcher/context, host and generated metadata need no lane edits.
`tje_game::execution_class(id)` indexes those immutable declarations once; missing,
duplicate, cross-owner or overlapping classes fail closed. Ownership tests require
every production translation to have exactly one class, not necessarily Atomic.
A lane owning several subsystems edits each one's module.
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
explicit reviewed execution contract. `ATOMIC_CAPABLE` declares bounded no-clock
entries; `EXECUTION_CONTRACTS` declares Functional or TimeAware originals. Host
registration, translated-callee dispatch and original fallback all resolve the
same owner declaration. Generated metadata contains no execution policy. Missing
IDs fail closed, not optimistically inferred safe from W0 membership or missing
hardware annotations. Read every relevant return/wait path before adding an ID;
record its source and reasoning in the lane's evidence. `SetupYM2612`, VBlank waits and STOP/IRQ
fixtures are rejected before atomic execution. A future wait/interrupt translation
needs an explicit time-progressing platform contract, never a fabricated ready
bit or a larger instruction bound. Even a nominally bounded VDP writer needs a
live entry state in which its frozen FIFO can accept the original access.

## 3. Register and test

Put a body in production `TRANSLATIONS` only after both-revision zero-mismatch,
exercised proof. Classify it in its owner's Atomic or non-Atomic list. The host's
`HookRegistry::translated` / `filtered` registers the declared class for each
translation; check mode uses that class's fork semantics. Callers may use their
own class or a weaker one: Atomic→Atomic, Functional→Atomic/Functional,
TimeAware→all. A TimeAware caller's real-clock envelope is retained through
Functional/Atomic callees; nested original calls do not silently freeze clocks.
Scoped Rust callees restore their caller's class even when they return an error.

Keep unproved bodies in a non-test `PENDING_TRANSLATIONS: &[Translation]` table
(and a `PENDING: &[FunctionId]` blocker list), never production `TRANSLATIONS`.
An opt-in lane harness uses `HookRegistry::proof(revision, PENDING_TRANSLATIONS)`:
only pending entry hooks are checked, while pending bodies override and compose
with every proven Rust callee. `from_translations` is the deliberately isolated
registry for testing original fallback, not the pending-composition harness.
All these constructors resolve the owner classes; no metadata regeneration,
shared-table edit or incorrect Atomic approval is required.

Registries (`TRANSLATIONS`, `FRAMED_TRANSLATIONS`, `PENDING_TRANSLATIONS`,
`PENDING_FRAMED_TRANSLATIONS`, `PENDING`, `ATOMIC_CAPABLE`, `EXECUTION_CONTRACTS`)
are literal arrays written in the subsystem module, one
entry per function. No aliases to another module's static (`= templates::PENDING`),
const joins or macros: `docs/FUNCTIONS.md` is generated from these arrays, and CI
fails if `python tools/trace_functions.py --check` cannot read them. Bodies may
live in any submodule; only the registry entry is in the subsystem root. Run the
generator (without `--check`) before committing so `docs/FUNCTIONS.md` is current.

`Host::new` rejects registry/ROM revision mismatch. Authored machine-code checker
fixtures still supply explicit classes using `register_execution_contract`.
Native dispatchers reject missing translations/classes at startup and never
fall back to ROM; native contexts must implement their real execution capability.

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

`HookRegistry::proof_filtered(revision, pending, ids)` hooks only selected pending
roots while dispatching through **all** supplied pending overrides and proven
Rust callees. An empty selection is equivalent to `proof(revision, pending)`.
Unknown roots, duplicate pending IDs and missing/conflicting execution classes
are rejected, including invalid unhooked pending callees. Filtering changes hook
coverage only; it never turns an unselected pending callee into original fallback.

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

Default Atomic check mode forks live state and freezes the original/Rust function
calls; the live machine keeps original instructions/timing. It compares return
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
Functional and TimeAware contracts are explicit opt-ins with the different
comparison obligations documented below; they do not silently relax Atomic.

Raw device read observations retain every access/value. Comparisons require the
same addresses, widths and order, but compare only hardware-driven status bits:

| Device read | Word mask | Byte mask |
|---|---:|---:|
| VDP status, C00004–C00007 aliases | `03ff` | even `03`, odd `ff` |
| Z80 BUSACK, A11100 | `0100` | `01` |
| VDP data/HV, controller ports, Z80 RAM, YM status | full `ffff` | full `ff` |

The excluded status bits are open-bus/prefetch values, not device outputs.
Writes are compared in full. Additional partially driven register variants need
source-backed masks; do not insert fabricated ROM reads to mimic prefetch bits.


Differences in dead stack bytes below entry SP are counted, not silently reported
as exact RAM equivalence. Every **semantic** read-before-write in either body is
audited, including mirrored RAM, typed Bus views, the immediate hardware facade
and nested original callees. The checker independently replays the audited side
with those bytes perturbed; only unchanged contract outputs discharge the read
as informational (see section 6). A real dependency or failed replay still fails
closed. Checked Rust writes mark bytes initialized. Reading a spill left by an
earlier original call whose Rust replacement omitted it is not safe composition.
Check mode does not claim that atomic replacement is CPU-cycle-equivalent.
The 68000 bus path labels semantic, discarded and instruction-prefetch reads.
CLR's unused read-before-write and MOVEM's terminal unused word still perform
their real bus/MMIO transactions, but their values are not stack dependencies.
A real consumed read at those same addresses is still audited; no address is
whitelisted. No additional Scc bus cycle is invented where upstream models none.


## 4. Drive and report coverage honestly

Every report initializes an entry for every registered hook, including zero calls,
and every subsystem. `by_subsystem` / CLI `coverage_by_subsystem` reports registered
function count, checked/replaced calls, mismatches and explicitly unexercised
functions; an unselected subsystem has zero registered entries, not coverage of
its untranslated functions. Per-function `calls_checked`, `calls_replaced`,
`mismatches` and global `unexercised` remain available. A zero-mismatch run is not
evidence for a zero-call function. The CLI writes metrics even after a mismatch.

Corpus-only callers may explicitly select
`host.set_check_failure_policy(CheckFailurePolicy::ContinueOriginal)`.
`FailFast` remains the default; Reference/Replace reject this opt-in policy.
A failed check is still a failed attempt: every error, including side-execution
errors before comparison, increments the call/mismatch coverage exactly once.
Its entry hook is disabled for the remainder of that session, without removing
any translation from callee dispatch. The current `run_frame` loop falls through
to its one original tick; no frame retry, CPU rewrite or partial-observation
replay occurs. Other roots continue to be checked. Explicit diagnostic fixtures
and errors from the live original tick retain their existing fail-fast behavior.

`CheckReport::failures` retains one bounded item per disabled root, including
function/entry PC, frame/master clock, live-frame write position, a trace ordinal
when the comparison names one, error detail and any field-level VDP evidence.
Details are capped at 4096 UTF-8 bytes plus a truncation marker; raw device/RAM
traces are not embedded. Retaining the items in the report, rather than draining
them, preserves earlier failures even if later original execution errors.
Completed playback with nonzero `mismatches` is **not a passing check**: callers
must write the failed report and return a failing exit status. Hard reset begins
a new session, reenables the roots and clears their counts/evidence while
preserving the explicitly selected policy.

Counters are internally keyed by stable registration indices, not freshly owned
strings; serialization still presents canonical function names. Constructor and
reset initialize every registered zero-call entry and unexercised set. Realtime
replacement skips check-only stack/write logs and updates existing counters
without per-call allocation; waits move reusable audio buffers rather than copy
them. The opt-in replacement allocation test measures actual dispatched calls
after buffer warm-up in both revisions.

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
| `TimeAware` | Independent original and Rust forks run real clocks; a Rust wait suspends the current emulated frame while original IRQ handlers execute | Explicit conditions and input timeline; exact main-thread effects and return outputs; accepted ISR invocation/order and logic effects, with only the completed ISR YM BUSY spin rule below |

`ATOMIC_CAPABLE` retains its meaning; never put a Functional/TimeAware ID there.
Production declarations belong in the owner's `EXECUTION_CONTRACTS` and are
consumed automatically by hook registration and dispatch. Authored proofs may use
`HookRegistry::register_execution_contract` with an explicit class. Missing or
conflicting declarations fail closed. Atomic contexts cannot wait.
Functional **check forks** may discharge only reviewed, already-satisfied
YM BUSY/BUSACK predicates without advancing clocks;
their live replacement envelope uses real-clock waits rather than spinning on
timed hardware.

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
`WaitRequest::interrupt()` resumes only after an unmasked interrupt has actually
entered and its original handler has returned. A STOP translation first applies
the source instruction's SR, then requests that wait; the primitive never lowers
the interrupt mask to make progress. Trace mode is rejected, not mistaken for an
external interrupt.
Functional check-fork discharge admits only byte BUSY-bit predicates at YM ports
`0xA04000`/`0xA04002` and word BUSACK bit `0x0100` at `0xA11100`, with ready value
zero. It performs the actual modeled read, retaining its trace/latch effects, and
errors if not already ready. In YM2612 cached-status mode, a cached busy value at
port 2 is not cleared artificially. RAM, interrupt, VDP/raster/DMA-status and
unreviewed predicates cannot use this clock-free discharge path.

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
Replacement hosts move the existing audio allocation through a wait rather than
copying the generated sample stream; unchecked play disables chronological RAM
auditing. Recorded session timelines also apply their per-frame RAM recipes
before the first instruction of each crossed frame.
The session runner installs the same immutable indexed recording on both check
forks, including recipes. Replacement replay does not apply consumed frames a
second time. A whole-session timing comparison refuses mismatched video boundaries
if a synchronous replacement has no intermediate snapshots; it never calls that
partial observation stream a passing timing proof.

### Exactly what a time-aware check proves

The original runs to return with clocks active. Rust starts from an identical
fork and advances real clocks through its platform waits. Return PC/SP, banked
stacks, declared register/SR outputs and live RAM at return remain obligations.
Logical VDP memories/registers/port latches compare after pending transactions
complete on comparison forks. Actual audits/traces are captured first; endpoint
normalization then runs while each fork still owns its cartridge, so pending
ROM-sourced DMA reads real ROM bytes. Comparison itself is pure/idempotent.
Execution never enables functional devices, rewrites readiness or advances an
unfinished handler to a synthetic return merely to make a check pass.

The readable layer has no original-instruction timing. Its equality projection
separates the checked function's **main thread** from interrupt work:

- Main-thread live-RAM byte stores and **all device accesses**, including
  discarded/prefetch MMIO reads, compare in exact issue order, including address,
  width and driven read bits. Explicit source-backed device operands compare
  their full actual values. No main-thread device read is removed, including
  readiness, status and H/V reads. Ordinary main-thread RAM read ordinals/counts
  need not match Rust local/predicate lowering; their values flow into the exact
  effects, live RAM, ABI outputs and independent residual-stack dependency probes.
- Successful CPU exception-frame entries/returns identify each ISR, not a guessed
  PC range, video phase, or change in interrupt mask. The accepted vector, handler
  target, invocation count, nesting and entry/return order compare exactly,
  relative to main-thread effects and other ISRs. Only clocks and the interrupted
  instruction address are absent from equality. Unbalanced invocations and
  unreviewed exception vectors fail closed. A declared root RTE is the checked
  entry's own exit, not a newly invoked child ISR; it must reach its exact
  reference boundary as that successful original frame returns.
- Each invocation compares its ordered device writes, non-timing device reads,
  live-RAM stores and semantic live-RAM reads. ISR writes cannot disappear behind
  a global trace filter, nor can restoring a main-thread value later hide a value
  observed by an ISR. Matching stores from identical entry RAM establish matching
  effects at handler returns; final live-RAM comparison covers the checked return.
  The existing dead-stack/ABI rule and independent dependency audit still apply.

**The sole ISR readiness stutter rule:** a semantic **byte** read of YM status at
`0xA04000` or `0xA04002` is eligible only when the executing user-memory
instructions are verified to be `BTST #7,<memory>; BNE <same BTST>`. The effective
address mode has no postincrement/predecrement or other side effect; the loop
contains no counter, timeout, write, callback or other instruction. Zero or more
real busy observations followed by one real ready observation become one
completed-poll token containing the original poll PC and port. The terminal
read must actually have bit 7 clear. A missing ready read, changed site,
unfinished loop or other effect before ready is an error. BTST consumes only
bit 7 and leaves the same Z after the terminal ready test; other status bits
remain raw but are not operands of this reviewed loop. Explicit full operands,
arbitrary YM reads, BUSACK/VDP loops and main-thread polls are not eligible.

Actual values, PCs and master clocks survive in raw `LogicEvent` traces.
`CheckReport::time_aware` counts ISR invocations and busy/total poll reads on both
sides, and calls with differing raw device/chip-write streams.
`CheckReport::last_time_aware` retains the latest complete raw pair in memory;
these private runtime observations must not be committed or distributed.
The quotient changes comparison only, never the executed device protocol.

VDP equality failures also retain a bounded `CheckReport::last_vdp_mismatch`
before the failing check fork is dropped. It identifies the compared scope
(`logical` for TimeAware, `full` for Atomic/Functional), the function/frame and
whether this was the cross-language check or a particular stack-perturbation
replay. `state.differences[0]` is the first differing field; at most fifteen more
follow, with a `truncated` flag if further differences exist. Locations name the
register index or indices, command/FIFO/latch/DMA field, or memory kind and byte
address; both values are retained. Individual values are capped at 256 characters.
Logical diagnostics do not present excluded FIFO/raster/cache differences as a
cause. Collection is read-only: it neither completes transactions nor changes
the selected equality comparison. The error string includes the first
difference, and CLI metrics include the structured report. These runtime values
are private evidence, not distributable cartridge-derived fixtures.

Raster/HV phase, FIFO occupancy/latency, render caches/pixels, independently
progressing Z80 state and audio phase are **not** readable TimeAware outputs.
The asynchronous Z80 chip-write stream is recorded rather than compared as one
interleaved 68000/Z80 sequence. Every main-thread/per-ISR **68000** VDP/YM/PSG
transaction still compares through its owned trace; checked-function sound
transactions may not be discarded.

`WaitForNextVBlank` does not draw idle random numbers: it waits on the
IRQ-incremented counter, and each original ISR must actually finish. Do not add
`GetRandomNumber` draws to that primitive. The main loop's separate idle draws
still require exact recorded counts; lag and interrupt phase still require
logic-tick/session evidence. A passing function-level logic check is not a
timing-equivalence claim for a native scheduler or for replacement play.

Named projection regressions separately exercise the live frame-492/2437 windows
and a GenerateSpeech fixture reached by **original-only** advancement to frame
10000. They are not a continuous 10000-frame readable-layer proof. A previous
continuous REV 02 run rejected UpdateSpeechAndDialogue at frame 7353 on strict
logical VDP state; its independent diagnosis/approval remains pending, not masked
or attributed to a particular field without evidence.


## 6. Differential dead-stack dependency checks

A semantic bus read is not necessarily an output dependency: for example,
`PrepareEntitySpriteFrame` REV 02 PC `0xCA36` reads a word whose high byte is
unwritten, then `EXT.W` discards that byte. `ReadPurpose` does not change to
accommodate this value flow.

For each side with audited bytes, check mode creates fresh forks of the same
entry state and perturbs **only** those canonical byte addresses. Two distinct
XOR masks (`55` and `aa`) change every candidate byte relative to its entry value.
With multiple candidates, additional two-pattern single-byte probes prevent
correlated changes from hiding dependencies such as `old_a - old_b`. If a replay
exposes more unwritten bytes, their union is probed before approval.

Nested atomic originals borrow the suspended root's entry-SP boundary and
initialized-byte bitmap, processing reads and writes in instruction order.
Incoming callee arguments can still be unwritten caller locals; moving below a
new callee SP does not make them safe. Real callee source PCs are retained, and
a later write never retroactively initializes an earlier read. Clocked originals
merge their complete ordered accesses through the same root scope.

Each replay uses the same hook, dispatcher, execution class, input timeline,
clock entry and stack-audit initialization as that side's baseline. It compares
every output of that execution contract: return/control state, declared
registers/SR, RAM outside the dead window, device state and ordered device
reads/writes, plus live RAM at each device write. The other side agreeing with
the baseline never substitutes for this independent dependency check. Any
difference or a replay that fails to complete is a check mismatch.

Accepted read sites are recorded in `nondependent_stack_reads`; rejected sites
remain in `uninitialized_stack_reads`. `stack_dependency_reruns` counts the extra
side executions, not checked calls, nested-call coverage or gameplay wait frames.
These are per-entry differential observations, not proof of independence for
every possible stack value or future game state. A later call is checked again.
Finite probes are **not** a formal independence or sequential-composition proof.
A read gated on an untested value can evade them: baseline `ff` with XOR
`55`/`aa` never tests an `old_byte == 00` branch. Review discharged sites against
static value flow where possible (for example, `EXT.W` kills the loaded high
byte before any use, or an exact `AND`/`MOVE.B` mask removes it). Otherwise flag
the site for a conditional-read mutation test rather than treating discharge as
universal proof.

The opt-in `tje-host` `semantic_reads` tests cover the real REV 02 `0xCA36`
case and REV 00 counterpart, discarded high bytes on either side, true
RAM/register/device dependencies, RAM/device ordering, correlated cancellation,
newly exposed candidates and nonreturning probes. `review_regressions` retains
the earlier sequential/Rust-only proofs and adds the prepared-original hole:
correct stores pass with original/Rust callees, omitted stores and
read-before-later-write fail in Atomic/TimeAware roots, and genuinely unused
eager argument loads remain informational rather than false dependencies.

### Opt-in equivalence for genuinely dependent residuals

A source-backed frame does not by itself discharge a real dependence. Such a
function may use the separate, reviewed dependent-equivalence mode:

```rust,ignore
use tje_host::dependent_equivalence::DependentProofConfig;
registry.enable_dependent_equivalence(
    &[tje_game::ids::EARTHLING_STEER_TOWARD_NEAREST_TARGET],
    DependentProofConfig { seed: 1234, seeded_probes: 16 },
)?;
```

Every named root must already be registered and have a descriptor in
`dependent_equivalence::REVIEWED_CONTRACTS`: its FunctionId, cited FrameLayout,
bounded entry-SP-relative residual ranges, and compiled producer/caller frames.
Empty roots enable nothing. Fewer than 16 seeded probes are rejected before
execution. Continuation hooks preserve the same descriptor/configuration.
This marker neither registers an unproven translation nor authorizes another ID,
address, branch, producer or missing frame.

The mode requires the original and Rust to perform the same residual reads,
through actual emulated stack RAM: exact ordered raw 24-bit **and** canonical
addresses, bus width, ReadPurpose and value. A long read is two ordered words,
not four artificial byte accesses. An opt-in journal records physical-stack
reads and their preceding-write counts; each side replays its own writes to
identify bytes read before initialization. Nested originals and real waits
append to the same root journal. The write-count indices are not compared across
languages. Discarded/prefetch-only reads do not establish dependence, but their
accesses to a genuinely dependent byte retain their actual purpose in the
compared trace. Extra semantic residual bytes expand the probe union, and any
byte outside the reviewed ranges fails closed.

Fresh coupled original/Rust forks receive at least 16 deterministic seeded
assignments and, separately, `00`, `ff`, `7f` and `80` for **each** dependent byte.
The entire expanded union is reprobed if another path exposes more bytes. Each
pair must match standard check-mode outputs and ordered device/RAM-at-device
obligations, the full RAM write/value **multiset including dead stack**, and
full SR/CCR even for compiler C. This is a write/value footprint contract, **not**
literal ordered all-RAM bus writes: low-word-first predecrement/MOVEM bus order
belongs to the cycle-exact engine. It is not silently normalized into a stronger
readable-layer timing claim.

The original's actual machine outputs must change under at least one completed
probe, and Rust must track every result exactly. A Rust-declared original exit
boundary is not an original return-value observation; actual D0/ABI registers,
SR, RAM and writes provide that evidence. Merely changing the perturbed input
bytes themselves is not output sensitivity. No zeroing, cached value, constant
or original/Rust agreement on just the baseline can substitute for this proof.

Successful genuine reads are recorded separately in `dependent_stack_reads`
and `dependent_equivalence`; they never become informational
`nondependent_stack_reads`. Evidence counts actual completed seeded/byte probe
pairs and side executions, matched pairs and original-sensitive pairs, including
partial work on a rejection. A marked call with no residual reads earns no
genuine-dependence claim. Unmarked hooks retain the independent two-mask checks
above, unchanged.

The nearest-target regression executes the real original `SetEarthlingState`
producer at an authored same-SP caller frame, then checks the complete pending
`EarthlingSteerTowardNearestTarget` body in both revisions. Its no-candidate path
reads selected Y at entry SP-6 **before** X at entry SP-10. Wrong-offset and
stored-value mutants must fail. This is labelled authored producer evidence,
not an observation of the last writer in a natural gameplay session.
It also does **not** prove replacement composition with a Rust producer. Before
consumer promotion, the named producer must preserve those actual residual
writes, with source-backed frames/callsites and a joint original/Rust sequence
proof. A legacy producer that elides its MOVEM save area is not discharged by
the consumer tracking arbitrary old bytes correctly. Keep the consumer pending
until that producer/caller cohort is migrated and proven.

## 7. Exceptions, terminal entries and persistent control flow

Execution classes describe clocks, not an implied RTS.
`tje_game::exits::contract(id)` supplies the separately reviewed `ExitContract`;
generated inventory remains unchanged. C handlers `HandleVBlank` and
`HandleHBlank` still RTS to assembly wrappers; only `Level6Isr` / `Level4Isr`
consume the exception frame with RTE.

| Entry | Finite reference boundary / native outcome |
| --- | --- |
| `Level4Isr`, `Level6Isr` | `ExceptionReturn`: restore saved SR/PC from the six-byte supervisor frame |
| `RestartFromResetEntry` | `Tail` to `_start`, after masking interrupts and reloading active SP from cartridge vector zero |
| `_start` | `Tail` to `GameProgram`; cold startup clears USP, warm startup preserves its original skip |
| `GameProgram` | `Call` to `VDPInitialisation`, pushing actual fragment-end return PC (`entry + 38`) |
| `VDPInitialisation` | `Call` to `FurtherInitialisation`, pushing actual return PC `entry + 8` |
| `ErrorTrap` | `Terminal`: at least one original self-branch, without a return or stack pop |
| `FurtherInitialisation` | Modal startup calls, then persistent `Begin` / `Idle` continuations |

Registries apply both policies. `GameProgram`'s fragment is admitted only because
it has an explicit non-RTS contract; undeclared fragments still fail closed.
Authored fixtures use `register_exit_contract(symbol, class, exit_contract, body)`
and retain their own addresses. Registration alone is not proof of a body.

`TranslationFn` retains `Result<Option<u32>>`. Non-RTS bodies end with typed
`exits::{exception_return,tail,call_transfer,terminal,main_loop}` helpers.
These produce `Error::ControlTransfer(...)` events, never a fake D0 or `Ok(None)`.
Callers propagate them with `?`; framed calls do not run RTS cleanup, restore an
abandoned caller frame or pop outgoing arguments afterward. Execution-class
scope is restored even when the frame is abandoned. Nested dispatch validates
each callee before extracting D0, so an invalid normal return cannot let a caller
continue and mutate state. Wrong sources/targets/JSR return PCs fail closed.

`exits::enter(context, entry, arguments)` executes a prepared entry without
inventing a JSR and returns `EntryOutcome::{Returned,Transfer}` for a native
scheduler to consume. Missing non-RTS bodies cannot use the ordinary RTS-only
original-call bridge.

`Context::{stack_banks,set_stack_banks}` expose both USP and SSP, rejecting
unsupported contexts by default. `exits::restore_exception` requires supervisor
mode, reads SR at SSP and PC at SSP+2, advances **SSP** by six, then restores SR.
User mode selects untouched USP; supervisor mode selects advanced SSP.
All D/A registers, full SR, PC, both banks and accepted exception depth compare.
The host records exactly one successful native RTE; wrapper register saves and
callee effects remain real materialized-frame obligations.

Rust supplies a validated outcome before original execution. RTS compares the
real return PC and SP; RTE reaches the exception return; tail/JSR boundaries must
originate in the declared source entry. Terminal checks execute at least one
original branch. Non-RTS comparison includes the still-live frame below entry SP
(dead end is no higher than the lower of entry/exit SP), not all historical
popped spills. RTE also conservatively includes any valid RAM USP, including a
suspended user caller beneath a supervisor return. Without declared disjoint
stack bounds a lower USP can require extra SSP-spill equality; it cannot waive
user-frame corruption. Device/RAM ordering and dependency probes retain the same
execution class and outcome boundary; no instruction bound is raised.

Use `check_entry_fixture(address)` for deliberately prepared frames, without a
synthetic return word or implicit settling. `check_continuation_fixture(entry,
stage, body, saved_frame)` checks a prepared persistent stage without restarting
startup; its explicit `FrameState` is the original parent's, not the lower SP.
Both restore machine/scheduler state afterward and retain coverage. These are
labelled fixtures, not claims that natural gameplay generated the inputs.

`FurtherInitialisation` retains a twenty-byte D2/D3/A2/A3/A4 save frame:

| Stage | Offset from entry | Retained state |
| --- | ---: | --- |
| Intro return / `AfterIntro` | `0xe0` | Save frame plus eight outgoing starfield argument bytes |
| Menu return / `AfterMenu` | `0xe6` | Same frame/arguments; cleanup only after menu return |
| `Begin` | `0x260` | Persistent frame; D2.low phase and D3.low frame-counter snapshot |
| `Idle` | `0x2fc` | Recurrent counter comparison; same retained frame/high halves |

The real JSR returns select `MainLoopContinuation::from_return`; the owner supplies
`MainLoopBodies` for the four stages. `resume` selects a real body, not a suspended
Rust caller. The uninitialized C phase is original D2 residue, not a new zero.
A native scheduler's modal Call record retains the parent's `FrameState` before
entering Intro/Menu and restores it before the matching `AfterIntro`/`AfterMenu`
stage. `from_return` maps only PC to stage; it does not recover that saved frame
from the current SP, which still includes twenty saved bytes and eight arguments.
`Begin` is one source-scheduled logic iteration, not a rendered video frame;
`Idle` draws RNG only while the counter equals D3.low and resumes only after a
real interrupt changes it. Recorded idle-draw and lag/IRQ policies remain separate
native-scheduler obligations.

Terminal stops game logic, not devices/IRQs. Hybrid dispatch consumes it once and
lets the original park loop clock devices rather than repeatedly dispatching a
zero-time hook. Hard reset clears it; native platforms advance devices without
68000 instructions. Function checks do not claim whole-session timing or a
complete native dependency closure.

## 8. Settle a live boundary before seeding RTS fixtures

End-of-video-frame state may contain a pending IRQ, bus/FIFO/DMA halt, refresh
wait or unfinished handler. Before the **first fixture mutation**, call
`host.settle_fixture_boundary(script.at(host.frame_count()))?`. Record its real
ticks/master clocks, start/end PC/frame, completed handlers and every returned
`BoundaryAdvance::completed_frames` snapshot.

Settling advances only original instructions/devices, even in Check/Replace,
to the first instruction-ready boundary with no asserted/pending IRQ, CPU wait,
bus halt or outstanding exception frame. Successful entries/RTEs, including
nested traps, are observed by the CPU, not inferred from SR masks/PC ranges.
It never rewrites PC/prefetch, SP, SR, stopped/IRQ flags or registers to manufacture
readiness. Already settled state returns zero work. Its fixed bound never raises
the function call bound.

Supply current-frame inputs and apply any current-frame recipe beforehand.
Crossed frames use the same `WaitInputPoller` timeline as waits; missing samples
are errors. Completed snapshots retain audio/hardware writes and normal frame
counts. Partial-frame audio/write/idle/lag observations remain for the next
`run_frame`. No translation runs or earns coverage during settling.

`check_call`/`check_fixture` reject unsettled entries **before** injecting slots
or a sentinel. They never settle after seeding inputs, when gameplay could consume
the synthetic mutation. Do not clear IRQs, lift the mask, choose another session,
replace caller state or increase bounds to evade the original work. Ordinary
fixture isolation restores all machine bytes; recorded settling progress remains.
Explicitly prepared non-RTS `check_entry_fixture` frames use section 7 instead.

`fixture::tests::player_entity_fixture_after_recorded_10000_frames` runs both
revisions' committed one-player recordings for exactly 10,000 natural frames,
then settles before Player's original level/x/y writes and `EntityIsNearPosition`
call. Its source-cited local predicate keeps this host regression independent of
translation-lane promotion.

## 6. Observed open-bus bits and explicit prefetch sources

An unused device bit is not necessarily unobservable software state. The
68000's bus latch is deterministic: a partially driven read can retain bits from
the original instruction's prefetched ROM word. If the program stores, returns,
branches on, or performs arithmetic with those bits, a defined-bit-only device
comparison is insufficient. Keep the strict RAM/register/CCR contract.

Use the `OpenBusSource` type from `tje_game::open_bus`, binding its
`prefetch(address)` source to `Platform::read_device_u8_with_open_bus` or
`read_device_u16_with_open_bus`. The
source is an **address in the user's ROM**, never an embedded opcode or literal
bus word. Cite the matching instruction layout and derive it separately for each
revision. For `LoadSoundDriverToZ80`, G `src/asm/z80bus.s:17-18` places the BUSACK
MOVE at entry + `0x14` and the prefetched following BTST at entry + `0x1a`:

```rust
use tje_game::{ids, open_bus::OpenBusSource};

let function = ids::LOAD_SOUND_DRIVER_TO_Z80.metadata();
let entry = function.address(context.revision());
let source = OpenBusSource::prefetch(entry + 0x1a);
let status = context.read_device_u16_with_open_bus(0xa11100, source)?;
```

The corresponding source addresses are REV 02 `0x03f594` and REV 00 `0x03f524`.
The upload consumes the full BUSACK word in D1 and two MULU operations; masking
the undefined bits or preserving an unrelated function-entry latch changes its
D0/D1 residues.

The same contract applies to value-returning convenience reads, not only the
generic MMIO methods. `GameProgram`'s initial status TST is at entry `0x2fc`;
the following JSR opcode is its prefetch source at entry + 6 (`0x302`) in both
revisions. Its `read_vdp_status(source)?` retains the full observed status and
real read side effects before the interrupt-mask call. Any test harness that
includes the canonical body must expose the same `open_bus` type, rather than
substitute the old source-less helper.

Ordinary, explicit-source and discarded reads share the host's central tagged
bus reader and observation path. Each actual access is dispatched once and
retains its width, purpose, value, dependent-read journal position and readable
logic event. A missing-source semantic read still performs and records the
real access before poisoning the contract; failure never erases device effects.

The hybrid platform validates an aligned, present ROM word, performs the declared
`ReadPurpose::Prefetch` read on the real bus, then dispatches the actual MMIO
read. Hardware-defined bits still come from the real device; the ROM cannot grant
BUSACK, clear YM BUSY, replace controller pins, or alter VDP flags. Normal device
read side effects and the resulting bus latch are retained. Explicit-source reads
are marked for **full raw operand equality** in check mode, in addition to the
ordinary ordered transaction comparison; a wrong prefetch source cannot pass
merely because the readiness bit or final ABI outputs happen to match.

A native platform resolves the same big-endian word with
`OpenBusSource::word_from_rom(user_rom)`, then uses its device model's driven-bit
mask. A missing/unaligned/non-ROM source is an error, never a fabricated zero or
ready value. The default Platform methods reject unsupported implementations.
Ordinary value-returning read helpers are for fully driven values only.
`read_vdp_status(source)` and `read_z80_bus_request(source)` require the explicit
source too. A raw partially driven read still performs its actual access, then
poisons the execution context; returning the right registers cannot conceal the
missing declaration. Invalid-source failures are sticky even if Rust catches
their immediate error. Entry/return validation covers nested value and framed
calls, original bridges and live returns, restoring scope/frame bookkeeping on
failure. Control-transfer mint/finish paths must validate the same contract
before treating a transfer as success.

The narrow `discard_z80_bus_request_read()` operation preserves CLR.W's real
read-before-write without exposing a value. It is accepted only against an
original discarded read; a semantic BUSACK operand cannot be relabelled
discarded to evade the source requirement. Reviewed readiness predicates remain
narrow mask-only operations, not a global guard toggle. Wait masks that observe
undefined bits (including a denied Z80 memory window) fail instead of reading
the synthetic idle loop's unrelated bus value.

### Which reads use the CPU bus latch?

This table is the current vendored standalone model's contract
(`genesis-core/src/bus.rs`), not a claim that every unused electrical pin on every
Genesis revision behaves identically:

| Read | Driven bits / source |
| --- | --- |
| BUSACK word `0xa11100` | Bit `0x0100` is real active-low grant; all other bits are CPU open bus |
| BUSACK byte `0xa11100` | Bit `0x01` is grant; remaining bits use the prefetched word's high byte |
| Byte `0xa11101` | Unmapped in this decoder: prefetched low byte, not another grant bit |
| VDP status words `0xc00004..07` | `0x03ff` comes from VDP; `0xfc00` from CPU open bus |
| VDP status byte, even / odd alias | Even: low two bits driven, high six from prefetched high byte; odd: all eight status bits driven |
| VDP data / HV words `0xc00000..03`, `08..0b` | No CPU-open-bus substitution; preserve actual VDP FIFO/read-buffer/counter semantics |
| PSG/write-only/unused VDP reads `0xc0000c..1f` | Whole word is CPU open bus; byte accesses select the addressed byte |
| Controller/version/control/I/O block `0xa10000..1f` | I/O model supplies the entire byte; word reads mirror it into both lanes. Pin defaults and reserved-register defaults are not replaced with ROM bits |
| Z80 window `0xa00000..a0ffff` | Granted: real Z80 memory/device byte, mirrored for word reads. Denied: CPU open bus |
| Other unmapped/write-only addresses, e.g. reset read `0xa11200` | CPU open bus, subject to the normal cartridge/device decoder |

Byte reads retain the upstream bus-latch update rule too. VDP-internal undefined
data/read-buffer bits and YM cached status are device-model state, not permission
to merge in arbitrary CPU-ROM bits. Expansion/cartridge register branches keep
their actual decoder behavior.

This API specifies an observable operand, **not** CPU timing, instruction-cycle
charges, IRQ admission phase, or a replacement for the frame scheduler. It does
not bypass Functional's status/HV restriction or solve the separate
`WaitForNextVBlank` full-music ISR-phase issue.

[decomp]: https://github.com/mschienbein/toejam-earl-decomp

## Source-backed frame-sensitive calls

The readable layer may keep proven value-only `tje_game::call` / Dispatcher calls
when compiler stack bytes have no semantic effect. **Use the explicit frame
contract** when an original function mutates an argument slot, reuses an outgoing
slot, passes a local's emulated address, reads a residual local, or exposes frame
bytes through an unchecked array/DMA. The cycle-exact engine owns exact stack
timing; this contract owns logically observed addresses/bytes, not instruction
cycle charges. Never zero missing arguments or unwritten local bytes.

`Context::frame_state()` identifies the current entry SP; its `argument_slot(i)`
is `Address<u32>` at entry SP + 4 + 4*i. `argument_word(i)` and
`argument_byte(i)` address the low signed word/byte of the same four-byte slot.
Unsigned parameters may cast the view. `stack_pointer` / `set_stack_pointer`
expose the actual emulated SP, **not** `address_register(7)` (the address-register
API is A0–A6). Native contexts implement FrameState/SP in owned emulated state;
contexts without that capability fail explicitly, never invent an SP.

Translate the emitted prologue, with a source citation:

```rust
use tje_game::call_frame::{FrameLayout, LocalFrame};
// G src/sprites/sprite_slots.c:10; matching c_sprite_slots.s:3-5.
let frame = LocalFrame::enter(context, FrameLayout::link(32, 0x7c, 0))?;
let free_slots = frame.local::<[i8; 12]>(-20);
let secondary_slots = frame.local::<[i8; 12]>(-32);
let output_slot = frame.argument_slot(3); // original entry SP+16
// ... use typed views, including original unchecked offset arithmetic ...
frame.leave(context)?;
```

`link(n,d,a)` performs LINK A6,#-n then predecrement MOVEM of the selected registers.
`movem(n,d,a)` omits LINK and allocates n local bytes before MOVEM. Masks are
ordinary D0–D7/A0–A6 bitsets, not reversed opcode masks. Saves are written to real
big-endian RAM; leave reads them back, so an overwritten save slot changes the
restored register. LINK's old A6 is also real RAM. Neither entering nor leaving
initializes/clears local storage. `local<T>(offset)` is A6-relative after LINK, or
relative to pre-local-allocation SP without LINK; it intentionally bounds-checks
no emulated C array. The helpers model MOVEM/UNLK values, not a single-register
MOVE restore's CCR side effects: reproduce those explicitly when source requires.

`local<T>(offset)` and `frame_pointer()` use the frame's **cached entry base**;
the returned Address is a numeric pointer snapshot, not a live A6-relative
reference. Keep it across a call only when the compiled source captured that
pointer before the call. A buggy callee such as overflowing BuildViewportSpriteLinks
can restore a corrupted A6. If the next source instruction loads `-2(A6)`,
recompute the address from `context.address_register(6)` **at that use point**
and access the resulting address through the appropriate bus. Do not read a
previously cached local view or force A6 back to its old value.

The same distinction applies to late parameter loads: FrameState argument views
identify the original ABI slots, not locations relative to a subsequently
corrupted A6. For source A6-relative arguments, recompute live A6 + 8 + 4*i
(plus the original low-word/byte offset). A pointer explicitly captured earlier
must instead retain its earlier address. `LocalFrame::leave` already uses live
A6 for linked restores/UNLK; this does not make earlier cached views dynamic.

The predecrement MOVEM saves each long's **low word first**, then its high word,
while transferring selected registers in descending register order. LINK and
JSR return-address pushes retain their separate high-word-first rule. Do not
infer all stack stores from one generic predecrement: a source `MOVE.L Dn,-(SP)`
has its own word ordering. If an outgoing store's bus order is observable,
perform those source-ordered writes and adopt the area with
`OutgoingArguments::existing`, rather than assuming the value-only push helper
decodes the caller's instructions.

For an exact outgoing call, cite the original JSR/BSR return address in each
revision and use `CallSite::new(pc)` or `CallSite::relative(caller, revision,
offset)`. The caller must have the original SP/layout/residual registers at that
point. `Dispatcher::call_at` pushes four-byte arguments right-to-left and the
actual return PC, installs the callee FrameState, runs the Rust or original body,
and performs RTS/caller cleanup without changing argument/local bytes. Hybrid
original execution uses the **already prepared** entry; it cannot push a second
sentinel frame. Class scopes and real wait envelopes remain enforced.

When the original caller examines or reuses an outgoing slot, retain it explicitly:

```rust
use tje_game::call_frame::{CallSite, OutgoingArguments};
let outgoing = OutgoingArguments::push(context, &[count, mode, secondary, output])?;
let dispatcher = context.dispatcher();
outgoing.call(context, dispatcher, ids::ALLOCATE_SPRITE_SLOTS, first_site)?;
let advanced_output = outgoing.argument_slot(3).read(context);
// Reuse the same slots, including the callee's mutation, at another real site:
outgoing.call(context, dispatcher, ids::ALLOCATE_SPRITE_SLOTS, second_site)?;
outgoing.release(context)?;
```

`OutgoingArguments::existing(context, slot_count)` adopts an area the compiler
already reserved without writing it. Passing fewer slots deliberately exposes
existing adjacent RAM for omitted-argument bugs; consumed slots are read from
emulated memory, never padded with zero. Numeric local pointers remain valid
emulated addresses after return; no host reference/lifetime is manufactured.

### Frame-consuming entries and lazy arguments

A frame-sensitive body has signature `FramedTranslationFn`:
`fn(&mut dyn Context) -> Result<Option<u32>>`. Register
`FramedTranslation { id, function }` in the owner's `FRAMED_TRANSLATIONS`, or
`PENDING_FRAMED_TRANSLATIONS` until its proof and caller cutover are complete.
Bind every owner's production table to its Subsystem's `framed_translations`
slice in `SUBSYSTEMS`, even while that table is empty. Merely exporting a module
static does not register its functions for dispatch or hooks.
An id has exactly one effective body form; conflicting legacy/framed declarations
fail closed. Both forms require the same unique reviewed execution class.

Root hooks in check/replacement mode and prepared Rust/native dispatch invoke
the **same** framed body. No argument snapshot is created. Read each slot/local
through FrameState/LocalFrame and the audited Bus at the point where the original
consumes it, using its actual byte/word/long width and order. In particular, an
unused output pointer must not be read on a failure branch. Re-read a slot after
a callee mutates it instead of retaining a stale host snapshot.

Legacy value-only signatures remain valid for proven value calls **and prepared
calls**. Prepared legacy dispatch reads its formal slots through the audited bus
into a value snapshot, as before. These additional ROM/RAM stack reads have no
device side effects: they may produce a conservative rejection for an unused
uninitialized slot, but cannot hide a different observed output or an argument
slot mutation that the caller consumes. The active root audit includes incoming
slots read by original callees too. Discharge of any extra read is finite,
per-state evidence, not a formal independence proof (see section 6).

Opt into a framed entry where conditional slot consumption or caller-observed
slot mutation requires exact source use sites; never silence its audit by
zero-padding or unaudited preloading. A value-only `call` targeting a framed
body fails with `PreparedEntryRequired`, even for zero-argument bodies: the caller
must supply a real child SP/return PC through `call_at`, not reuse its own
FrameState. Typed exits may explicitly invoke an already prepared framed root
with an empty value-argument slice; nonempty values are rejected before its body.

For a mixed proof overlay, use
`HookRegistry::proof_with_frames(revision, legacy, framed)` or
`proof_filtered_with_frames(revision, legacy, framed, root_ids)`. Tables are
borrowed statics; selecting entry hooks never removes pending/proven callees.
The bounded counterpart is `from_entries(revision, legacy, framed)`. Native
startup uses `Dispatcher::native_with_frames(roots, legacy, framed)` and checks
both forms throughout the complete dependency closure; it never falls back to
the original. Do not use a proof-only wrapper that changes the entry contract.

Promote a framed callee only with every affected production caller migrated and
proven in the same cutover. Until then the complete framed body remains pending,
even if isolated fixtures pass. No hidden original fallback or legacy shim may
paper over a value-call edge. This applies to AllocateSpriteSlots and its
AllocateEntityViewportSprites caller, and BuildViewportSpriteLinks and its map,
elevator and sprite-queue callers.

When the ID is **already production in legacy form**, stage its alternative as
a public, canonical framed function referenced only by a private or opt-in proof
harness's static `FramedTranslation` array. Do **not** put that ID in a public
`PENDING_FRAMED_TRANSLATIONS` table while its legacy record remains in
`TRANSLATIONS`: production/pending overlap correctly fails the trace generator.
`FUNCTIONS.md` continues to show the ID as production (legacy). No parser waiver,
table renaming or proof-only body adapter is allowed. The atomic promotion range
removes the legacy entry, adds the same canonical body to `FRAMED_TRANSLATIONS`,
and migrates/proves all affected callers together. For an ID with no production
entry, the ordinary public `PENDING_FRAMED_TRANSLATIONS` table is appropriate.

Ordinary errors restore suspended caller bookkeeping while remaining errors.
Non-returning typed control transfers retain their live frame for the exit
contract, with no fake RTS/cleanup. Unexpected SP, a corrupted return PC or an
odd return PC fails closed instead of silently accepting an impossible normal
68000 return. Odd call sites are rejected before any outgoing frame is written.

The ROM-backed `tests/call_frames.rs` runs an authored caller against retail
AllocateSpriteSlots and the **canonical pending framed body** in both revisions,
proving SP+16 mutation, shared outgoing slots and a failure branch that never
reads the omitted fourth argument. Its CPU-free native context also proves
omitted-argument aliasing of a saved D2 and a writable local passed by emulated
address. There is no copied selector or proof-only adapter. These entries remain
pending until the sprites lane completes the all-caller promotion; successful
isolated proofs do not change the production table.

## Wait-resume interrupt quiescence

A true wait condition and matching parked PC/SP are necessary, not sufficient,
for resuming Rust. The pinned 68000 first recognizes an eligible IRQ in a
ten-cycle step, then enters the original handler on its next step. After an RTE,
a lower-priority IRQ can already be eligible under the restored SR. Rust must
not run ahead of that handler.

The hybrid wait resumes only when the condition holds, the CPU has returned to
its parked PC/SP, no real bus/refresh wait remains, and the CPU's existing
`hybrid_instruction_ready` predicate is true. Otherwise real ticks deliver the
pending recognition/handler sequence. This does not alter SR or acknowledge an
IRQ by hand. An interrupt at or below the restored mask remains pending; a
same-level request cannot re-enter its own masked handler.

Ordinary timed original returns apply the same ready boundary before the Rust
caller continues. Their actual return PC/prefetch and exception-frame contents
are retained: only the CPU's interrupt/stall paths execute, never a caller opcode
or synthetic replacement return stub. Typed RTE, terminal and main-loop
boundaries still end at their separately declared transfer; they are not
redefined as ordinary RTS calls.

The first ordinary return boundary is latched while eligible handlers drain.
At the original entry exception depth, a handler returning to a different PC/SP
is an error before any unrelated caller instruction can run. The parked wait
enforces the same depth/PC/SP contract. An unfinished handler cannot impersonate
a returned continuation merely by using its PC/SP. Failure retains the actual
CPU state; it does not restore a plausible return address or stack bank.

The regression is source-visible game behavior, not a rendering waiver:
InventoryRefreshContentsAndWait can finish VBlank with a level-4 HBlank request
already pending. Its original HBlank handler must execute before continuation,
including the final H-int-counter disable write. This fix neither drains masked
interrupts nor aligns arbitrary original/Rust instruction cadence. Raw IRQ and
device observations remain recorded.

