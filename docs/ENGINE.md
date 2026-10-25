# Cycle-exact engine

## Fidelity and ownership

The faithful game reproduces the user's cartridge as modelled by the pinned
jgenesis reference, not a new approximation of Genesis hardware. The acceptance
contract is identical CPU state and ordered accesses instruction by instruction,
then identical work RAM, VDP memories, pixels, stereo sample bits/counts and sound
register streams at every video boundary. See [PLAN.md](PLAN.md),
[TIMING.md](TIMING.md), and the private CycleProto report for the measured reason
that source-function or basic-block interrupt admission is insufficient.

There are two deliberately separate implementations of the game:

* `tje-game` is the readable, address-preserving translation of the decompilation.
  Its per-function check contract does not establish instruction timing.
* The faithful engine compiles the user's ROM at first launch. Every original
  instruction is one native continuation, carrying all original D/A registers,
  CCR/SR, user/supervisor stacks, PC and prefetch state. No generated instruction,
  immediate, code listing, table payload, RAM image, frame or sample is shipped.

The engine's maintained code lives in our crates. Vendored files retain their
upstream provenance; CycleProto's experimental semantic executor is not a
production dependency.

| Component | Responsibility |
| --- | --- |
| `tje-engine-core` | ISA decoder, metadata-constrained CFG discovery, exact timing formulas, ROM-specialized primitive SSA, common CPU/bus/result ABI |
| `tje-engine-runtime` | Continuation dispatch boundary, reset/STOP/HALT, two-phase IRQ admission, exception frames, device-step ordering |
| Native device layer | VDP, YM2612, SN76489, original Z80 sound program, master-clock progress and all reference bus/refresh waits, without a 68000 executor |
| `tje-engine-cranelift` | Desktop primitive-IR compilation, direct continuations, private machine-code cache |
| `tje-engine-wasmgen` | Runtime WebAssembly emission from the same primitive IR, asynchronous instantiation and private IndexedDB cache |
| `tje-engine-verify` | Development-only IR evaluator and reference differential/session harness; never a shipped fallback |

The runtime and both backends consume the common core ABI. The core never depends
on a backend or scheduler. The device layer does not depend on the 68000 decoder
or semantic executor. The only core dependency on the reference CPU crate is a
**dev-dependency** for ROM-free timing checks.

## Opt-in machine snapshots

`Engine<B>::snapshot()` captures a `MachineSnapshot`; `snapshot_into` reuses an
existing image allocation. `restore(&image)` keeps the immutable compiled backend
and cartridge, with no reset, MMIO read, queue drain or device-clock movement.
The canonical image contains all 25 CPU words, work/Z80 RAM, the entire VDP
(including its backing frame), YM2612, SN76489, Z80, audio resampler and exact
pending samples, deferred writes, controller latches, reset/IRQ/wait phases,
master clocks and frame counters. Optional proof journals are not game state:
restore clears them while retaining observer enablement.
Restore also rewarms instruction, chip/resampler and output-audio queue capacity,
including empty power-on images, so the next frame does not allocate merely
because serialization discarded spare capacity.

Format 2 has an eight-byte `TJESTATE` magic, little-endian version, actual ROM
SHA-1, frame count, CPU words, execution-profile SHA-1, fixed-integer little-endian
hardware encoding and a SHA-1 corruption checksum. `MachineSnapshot::from_bytes`
rejects unknown versions and corruption; restore rejects another ROM, execution
profile or compiled CPU divider before mutation. Bump `SNAPSHOT_VERSION` whenever
any encoded hardware schema changes; there is intentionally no best-effort
migration of machine images.
`as_bytes`/`into_bytes` expose the private file representation; `state_hash` is a
deterministic image digest, not peer authentication.

Ordinary capture/restore methods bind the `faithful` profile.
`snapshot_with_profile(&str)`, `snapshot_into_with_profile(&mut image, &str)` and
`restore_with_profile(&image, &str)` let stateless enhanced drivers supply a
version/config-bound identifier without changing the machine image type.
They do not capture host sidecars or authorize cross-profile migration.

Images contain copyrighted runtime state, unlike public controller sessions.
Keep them only in private local storage, never in reports, source control or
distributed caches. `RewindBuffer` evicts oldest images against an allocation
budget and removes the abandoned future branch when rewinding. Capture and the
ROM fingerprint are lazy: default faithful playback does neither. Future
enhanced modes with host-owned state must add an atomic profile/config/sidecar
envelope, not silently restore base-machine RAM into another profile.

Both frontends expose five private local save slots, rewind and 4x muted
fast-forward as independent default-off settings. Desktop controls live under
Settings → Extras (F2/F3/F4, hold Backspace/Tab); the browser's Convenience extras
panel uses F2/F3/F4, hold Backspace/backtick while the canvas is focused, leaving
Tab navigation intact. Browser slots use the separate local
`tje-private-save-states` IndexedDB database, keyed by ROM SHA-1 and slot; neither
cartridge forgetting nor a settings reset deletes intentionally saved slots.
The native `.tje-state` files are likewise retained when forgetting a cartridge.
Disable rewind to free its 64 MiB-bounded history. A load/rewind invalidates
input-only power-on reports until hard reset, rather than exporting a false
replay or adding copyrighted snapshot bytes to a report.

`snapshot_sessions` (native Cranelift example) runs complete recorded sessions,
captures three checkpoints, compares each restored continuation's complete
machine/frame/audio hashes and measures 100 warm captures/restores. Its private
metadata-only receipt feeds `web/verify-snapshots.mjs`, which checks browser
straight runs against those native hashes, restored continuations, rejection
without mutation, rewind branches, five persisted slots across reload and the
actual frontend controls. Build the observation-only browser package with
`web/build-replay.sh`; generated Wasm and all evidence belong under `scratch/`.

## First launch and metadata

After the ordinary ROM importer identifies REV 02 or REV 00, call
`metadata::game_metadata(revision)` and
`metadata::compile_program(rom_bytes, &metadata)`. Configurations with another
68000 divider must use `compile_program_with_divider`; the default is seven.
The compiler produces a sorted `Program` of instruction continuations. Backends
may discard the ROM-specific IR after compilation; it is not an instruction
interpreter or an executable fallback.

Function names, entry points and half-open extents reuse `tje-game`'s generated
metadata. The supplemental switch metadata contains only jump-site/table
locations, element counts and relative bases. Actual signed word offsets and
resulting targets are read from the user's ROM. Table payload ranges are removed
from executable ranges. Discovery follows direct control flow from every
catalogued function/fragment and runtime-read switch targets, rather than
linearly interpreting embedded data as instructions. Indirect calls dispatch to
the compiled PC table. A missing continuation is an error, not permission to run
an interpreter. Odd/truncated instructions, control flow outside the executable
metadata, overlapping instruction extents and code/data crossings fail closed.

The decoder covers the 68000 encoding space using authored ISA rules adapted
under GPL-3.0 from jgenesis, not game-derived opcode tables. An architecturally
illegal encoding is a real ILLEGAL/line-A/line-F exception, not an unsupported
operation silently replaced by a NOP. Decoding and lowering happen only during
compilation. A browser is never asked to invoke a Rust compiler or download a
predecoded cartridge.

Desktop code is cached only in the user's application-data directory, keyed by
ROM identity, engine/IR identity and target ISA/features. Cache files are private
ROM derivatives. A stale or damaged artifact is regenerated from the user's ROM.
The native continuation mapping is writable while loading and executable only
after publication. Browser modules are compiled at runtime and cached in
IndexedDB with the corresponding identity. Neither cache is a distributable
build artifact; possession of a cache does not relax ROM ownership requirements.

## Instruction IR and state ABI

`CpuState` is `repr(C)`, 25 consecutive `u32` words:

```
D[8], A[7], USP, SSP, PC, SR, prefetch, pending_interrupt,
stopped, frozen, address_error, last_muldiv
```

There is no redundant A7: SR.S selects USP or SSP. Word address-register writes
sign-extend to 32 bits; data-register byte/word writes preserve upper bits. SR
writes retain only `0xA71F`, as in the reference. An SR.S transition simply changes
the selected canonical stack; it cannot overwrite the inactive stack. This also
preserves the compiler's word-sized indexed-address arithmetic, including wrap
before ADDA.W sign extension. Intermediate register, flag and stack states are
not projected to source-level C values.

Each `InstructionIr` is linear SSA with explicit `Value` definitions:

* 32-bit wrapping arithmetic/bit operations, signed/unsigned comparisons,
  selections and masked shifts;
* state loads/stores using fixed `StateField` offsets;
* byte/word bus reads and writes, tagged semantic, discarded or prefetch;
* a conditional read for **non-speculative** taken-branch prefetch;
* the reference's VDP status/HV read compensation in **master clocks**;
* guarded exception exits carrying complete base charge, vector, saved PC and
  address-error metadata;
* `Finish`, the original instruction's interrupt-acceptance boundary.

There is no operation that evaluates a decoded opcode or hides an EA-resolution
interpreter. Both backends lower these primitives. Memory/MMIO operations cannot
be eliminated, reordered or widened. A long access is two explicit word accesses;
MOVE.L predecrement and extend-operation predecrement preserve their reversed
word order and intermediate address-register updates. MOVEM preserves mask order,
self-register behavior, postincrement timing and its final discarded read.
CLR's discarded read remains a real bus transaction. Fetching an extension word
also performs the reference's next-word prefetch at the correct point in the
access sequence. Branches do not speculatively fetch their untaken target.

Arithmetic lowering preserves X independently from NZVC when required, sticky Z
for ADDX/SUBX and BCD, all size truncations, signed overflow, counts zero through
63 for shifts/rotates, and operand-dependent multiply/divide timing. Divide by
zero and CHK set the reference's pre-exception flags before exiting. Privileged
SR/USP/RTE/STOP paths guard before their operand accesses. Illegal instructions,
TRAP/TRAPV, CHK, divide by zero, privilege and odd-address exits delegate frame
construction to the runtime, not to the original CPU implementation.

Reference-specific behavior is intentional: Genesis TAS suppresses memory writes;
RESET follows the pinned executor's long-NOP behavior; trace is retained in SR
without inventing trace-exception behavior absent from that executor. This engine
claims equivalence to that model, not independent physical-console accuracy.

The relocation-free desktop ABI is
`step(state_ptr, bus_context, callbacks, exception_out) -> u64`: low 32 bits are
complete base CPU cycles; high 32 bits are the exception vector (zero normally).
`ExceptionOut` holds saved PC, fault address and status word. `StepResult` is the
safe Rust counterpart. The bus callbacks are indirect so persisted code contains
no process-specific function addresses. They must not unwind across native code.

The WebAssembly entry is `step(state_ptr, bus_context, exception_out) -> u64`.
It imports the runtime's numeric bus functions directly and inserts its dispatcher
into the application's function table. The runtime calls that table entry without
crossing JavaScript at instruction boundaries. JavaScript is used for asynchronous
compilation/instantiation and IndexedDB, not register or bus-operation dispatch.

### ISA register dependency analysis

`tje-engine-core::dependencies` analyzes the **same compiled instruction SSA**,
not an independent opcode dependency table. `analyze_instruction` accepts cached
`InstructionIr`; `analyze_rom_instruction` decodes the user's actual opcode and
extension words through the existing compiler. Reusable scratch/report buffers
retain their largest analyzed workload. No bus call, device advance or published
CPU update occurs.

`StateMask` covers all **25** canonical domains: D0–D7, A0–A6, USP/SSP, SR and
every PC/prefetch/pending/halt/fault/timing word. A7 is represented by its physical
USP/SSP banks and SR.S selection, not an invented independent register slot.
CCR is the low five SR bits. Non-register domains require explicit consumer
handling; `registers_only` is an explicit projection, never the default.

The report separates live entry reads, physical stores, possible non-identity
writes, proved new definitions and preserved bits. Byte/word upper halves,
unchanged CCR bits and inactive stack banks remain distinguishable from new
definitions. The default read/fold includes identity-passthrough uses; the
effect-only/provenance views do not license importing preserved oracle outputs.
New-definition masks are conservative sufficient proofs, intersected across
**all permitted exits**. An early `ExitIf` cannot grant a later normal-path
definition. RMW/conditional cases may conservatively prove fewer definitions.

Three input modes have distinct authority:

* `Conservative` unions possible inputs and intersects exit definitions.
* `Entry` refines choices from the actual before-state while unresolved
  memory-derived choices remain conservative and flagged.
* `Executed` additionally consumes actual recorded reads exactly once, in order,
  matching the explicit address domain, width/purpose and value width. `Raw32`
  matches logical SSA EAs exactly; `M68000Bus24` requires a journal address with
  no high bits and matches only the low 24 address lines. This never normalizes
  RAM mirrors, register values or data, and the domain remains in `PathBinding`.
  Missing, extra or mismatched observations fail closed. The caller must bind
  those values to its accepted original trace and compare controlling entry-input
  bits before using outputs; the API does not attest caller-authored observations.
  Path-refined results
  cannot be cached under opcode/PC alone.

Per-operation provenance separates memory address and write-value roots,
register outputs, passthrough, selection controls, timing and exception uses.
Root selection always retains controlling inputs. Classifying private compiler
frames and proving memory causality/identity restores remain Host obligations:
never subtract a saved register globally when it may also feed algorithm uses.

After a successful `Executed` analysis with one actual exit,
`DependencyScratch::output_reads(field, bits)` follows the retained SSA binding
for exactly those output bits. The result identifies its PC/opcode and path
binding, includes every actual path control, and explicitly flags demanded
memory values or memory-selected controls. An observed memory sample is never
treated as a constant or a memory-causality proof. For example, the low bit of
an even stack adjustment depends only on the old low bit, not a coarse union
of all SSP carry inputs; this follows the existing SSA walker, not an opcode
allowlist. All 25 canonical output fields can be queried.

Every new analysis invalidates the prior output binding before ROM compilation
or validation, including truncated/odd-ROM errors and non-executed inputs.
Invalid register indices fail through the existing canonical-field validator
instead of aliasing another domain or panicking. Conservative and partial Entry
queries fail closed rather than select only normal-final bindings. An executed
early exit retains its state at that exit, not unreachable later stores;
multiple permitted exits are unsupported by this query. These results are
analysis data only, never source, timing, interrupt or replacement authority.

`DependencyFold` accumulates read-before-write and written masks, killing only
proved definitions. `observe_outputs` includes pass-through declared outputs as
inputs when a consumer intends to import them rather than preserve its own
values. `Finish` IRQ/trace admission and exception frame/vector effects lie
outside instruction SSA; explicit scheduler obligations must be rejected or
separately accounted for, not mistaken for a complete empty dependency set.

## Timeline, exceptions and devices

Base instruction charges are not elapsed time. The runtime retains the reference
order: consume refresh/FIFO/DMA/Z80 waits before CPU issue; execute one compiled
instruction's ordered accesses; advance Z80, controller/PSG, VDP and YM by the exact
master clocks; update interrupt latches; then publish deferred CPU writes. Long
port stores remain two transfers. Read compensation does not advance devices or
fabricate ready/busy values. Z80 bus ownership, YM busy state, VDP FIFO/DMA and
refresh remain live device state, not fixed instruction surcharges.

**Reference phases, not invented physical microcycles.** In this pinned model,
all CPU reads, discarded accesses, prefetches and deferred-write *issues* occur
at the instruction's starting master clock, in explicit instruction-specific
order. Only after these issues does the complete base-cycle device advance run;
the existing byte/word queues are then committed with their original width
grouping and stall consequences. Two words are distinct effects but are not
assigned separate physical bus subcycle times absent from the oracle. A timing
bridge must reproduce these exact phases, not improve the hardware model
silently and thereby diverge from the compiled engine/reference.

Wait/IRQ/reset/STOP/HALT/frozen and exceptional-entry paths initialize the same
read compensation from the prefetched opcode classifier and the actual CPU
divider before bus accesses or device progress. This matters even with no main
CPU instruction: the Z80 can read banked VDP status/HV ports during those steps.
Normal compiled instructions set the phase directly through IR, with no redundant
runtime decode on their successful fast path. CMPA's classifier intentionally uses
the reference's CMP look-ahead rather than CMPA's execution charge.

IRQ recognition is two separate scheduler steps: the reference's ten-cycle
admission delay latches a level, then acknowledgement and exception entry cost
44 cycles. Instructions cannot execute between those steps. Interrupts are
recognized at every original boundary, subject to SR.IPL. STOP polls in four-cycle
steps while admitting interrupts; HALT/frozen polls in one-cycle steps. Reset and
exception entry preserve the reference's stack-word order, vector reads and
prefetch. An address error during address-error entry freezes the CPU until reset.
The engine retains the reference's existing level-seven and interrupt-acknowledge
approximations rather than silently claiming E-clock physical accuracy.

An exception exit already contains its **complete** base charge. The runtime
must not add exception-entry cycles twice. Failed exception stack/vector accesses
instead take the reference's address-error path. Hardware bus waits remain
separate scheduler steps in every case.

## Canonical audio arithmetic

The accepted reference uses the **portable scalar resampler on every target**,
including the reference oracle, desktop engines and browser. Runtime CPU-feature
selection must not change tap order, addition grouping or FMA contraction.
The original x86 SIMD/FMA paths produced different sample bits from the portable
path; they are removed from this vendored build. Scalar accumulation alone is
insufficient: platform C math libraries also differed on the ARM64 CI runner.
Filter poles/warping, configured volume gain, and YM sine/log/power table
initialization therefore use pinned pure-Rust `libm` with `force-soft-floats`.
This disables architecture/intrinsic specializations even when dependency
feature unification enables `arch`. Butterworth unit-circle poles use explicit
deterministic sine/cosine rather than a hidden `num_complex::powc` math chain.

PSG volumes and sinc kernels are committed mathematical constants, not runtime
transcendental calculations. IIR filtering and sinc accumulation retain ordered
basic arithmetic; exact alternating signs use integer parity. This is an
explicit reference-audio arithmetic policy, not a change to game instructions,
device clocks or sound-chip register streams.

ROM-free production-device goldens program authored PSG and YM2612 register
sequences at 44.1 and 48 kHz and compare sample-bit SHA-256 hashes. The same
fixture runs on native CI and in the browser proof harness. Linux/Windows native
measurements, browser measurements and the explicit macOS ARM64 CI capacity gate
cover the canonical path; unexecuted CI/platform results must not be called local
evidence. Audio receipts from the earlier feature-selected path are historical
and must be regenerated before supporting this policy's exactness claims.

## Timing-only regions for readable Rust

`tje-engine-core::timing_region` compiles an instruction's **timing dependency
slice**, not another semantic executor. Roots are effective addresses, access
conditions/order/width/ReadKind, exact charge, prefetch/PC and exception metadata.
Instruction-local arithmetic needed for those roots is permitted; dead game
results are removed. No game `StoreState` survives, and no memory write payload
is calculated or published. The slice has no bus object and cannot read RAM/MMIO,
execute a game effect or return a game value.

Rust supplies the real full entry register/flag/stack state, performs every
requested bus effect once and acknowledges the **actual** observed read or
Rust-owned write value. Prefetch/discarded accesses remain explicit real effects.
Opaque move-only tokens prohibit dropped, reordered, cross-cursor or replayed
acknowledgements. The next event cannot issue until its predecessor is complete.
A source-derived `RustPathChoice` is required at every original boundary; the
dependency slice validates it rather than privately deciding the game's branch.
Only then is an unforgeable advance/commit grant emitted.

`TimingScratch` reserves reusable dependency/read buffers before execution. A
process-unique owner is assigned once per scratch, and each admitted cursor has
an independently advancing checked `u64` generation. There is no globally
contended per-instruction atomic or 32-bit instruction lifetime. Completion
grants bind the admitted plan, owner, generation and acknowledged effect counts;
a grant compiled from another IR cannot authorize a runtime boundary.

`tje-engine-runtime::timing_region` admits a reviewed finite region with explicit
entry, instruction identities, allowed edges, return boundary and diagnostic
step ceiling. The ceiling is not a semantic timeout or termination proof.
Admission rejects a pending address fault or incorrect PC/prefetch and drains
waits before original legal IRQ boundaries. A main region is suspended without
consuming its next bus effect; resumption requires the actual RTE-returned
PC/prefetch, SR, **both** banked stacks and interrupt bookkeeping, never a
repaired old snapshot. Original ISR execution is allowed only in the explicitly
labelled hybrid oracle path, not as a shipped fallback.

Native startup requires a concrete `ScheduledIsrClosure`, an immutable
`NativeIsrProgram` freshly compiled from the user's ROM, the required interrupt
vectors and the complete reviewed ISR/callee closure. An implementation-present
boolean is not a capability. The controller admits the real pending IRQ,
performs its real exception-frame entry, and gives the source callback an
`IsrSourceContext`. This exposes read-only live CPU state and controlled
`IsrEffects` on the **same** device object. Actual reads are performed and
acknowledged together through the current cursor; the provider cannot submit
invented read values, bypass effects or substitute a foreign/zero-cycle grant.
The callback still owns its independently proved Rust game effects and paths.

`ExceptionReturn` is only a request. At an actual prefetched supervisor RTE and
an owned exception frame, the controller's exception-return primitive performs
the ordered stack reads, SR/stack-bank selection and target prefetch, then the
real device advance. Only exact bank/SR/PC/prefetch restoration and actual
scheduled progress mint the move-only `RteReceipt`. No provider-supplied
`rte_observed` flag or copied CPU snapshot can complete service. Each
`step_native_interrupt` call borrows devices only for one real hardware/source
step and yields any actual `FrameRendered` before a subsequent step. The caller
therefore samples its existing frame timeline once without retained device
borrows, a second clock, or hidden input publication.

`tje-host::timing_region` is the check integration point. Its `RustInstruction`
callbacks own effects and source register/flag/stack updates; `TimingLedger`
hooks each actual source-instruction, wait and original-ISR step so an outer
call budget cannot be renewed by a region. It reuses the ordinary `WaitClock`
input/recipe policy and the reference's shared device/commit/output tail, with
no original main-opcode execution, synthetic parking loop, eager pipeline read
or aggregate block charge. Unsupported exception/nonlocal exits fail closed
instead of pretending to return normally.

### Portable source bridge and ordinary return ownership

`tje-game::timing::TimingSource` is an object-safe capability used through `Context`.
It depends on neither the host, the compiler nor jgenesis. Its default context
implementation fails closed before consulting the source. A source declares
reviewed function/role identities and permitted paths, receives the current
register/stack state, requests each game read/write, acknowledges its actual
result, and commits its own registers and next role. Instruction prefetch is an
adapter-owned, separately ordered real effect. The engine independently checks
the address, width, purpose, ordering and path against the ROM-bound dependency
slice; source-supplied cycles, ROM opcodes, interrupt authority and new budgets
are not part of this protocol.

The portable entry must have been prepared for the exact function by the framed
dispatcher. Its actual entry PC, stack pointer, physical frame, revision and
prefetch must already agree. Neither the bridge nor the body repairs an entry
snapshot, pushes a sentinel return, or patches arguments. Descriptors alone do
not activate translations or change their execution class.
Each portable program owns exactly one reviewed function scope; foreign bodies
are never inlined into it. `Program.calls` declares separately scoped JSR/BSR
sites, finite FunctionId targets and the parent return Location. The source's
`Path::Call` selects that target/return role, while the adapter validates the
actual resolved EA, real call stack word, PC/prefetch and caller-owned fallthrough
against the user's ROM. Every source keeps its literal execution class, inherited
callee permissions and physical clock; ClockedStrict is a proof envelope, not a
class escalation or an original-execution capability.

Startup validates immutable source descriptors against the verified revision,
ROM, device divider and interrupt policy. It lowers each instruction once and
pre-reserves reusable timing scratch and interrupt storage. A normal entry does
not decode or allocate a plan, and warmed interrupt nesting reuses its storage.
Deeper actual nesting may grow the reusable stack: priority levels are not an
artificial depth limit because an ISR can lower IPL. Dynamic return permission
checks only the aligned physical-ROM address, not an ahead-of-time caller
opcode read. The actual RTS stack read and target prefetch still happen once
on the real bus.
Native device authentication additionally binds the previously checked ROM to
an opaque process-local `AddressSpace::rom_identity()`: clones preserve that
identity, new address spaces get a new one, and RAM mutation does not alter it.
The identity contains no ROM-derived bytes and is not serialized into evidence.
At native construction/import, capacity-only helpers restore hardware-bounded
sprite, chip-output and sinc-output storage lost by cloning/decoding. The
80-entry SAT, strict VDP tick limit, synchronization lag and the pinned native
map's whole-Z80-instruction bank-access bound determine the capacities. That
bound includes the entire 32 KiB DD/FD prefix bank, ED word transfers and optional
trace lookaheads, not merely one Z80 T-cycle. Reservation clears no samples,
alters no filter history and moves no clocks. `vendor/jgenesis/NOTICE.md` records
the derivation and unchanged upstream provenance.

Native entry borrows the caller's frame-qualified input latch. Only an actual
renderer event polls the next sample and validates that the shared latch now
contains it, then applies the recipe and emits output once. Repeated entries
in one frame do not poll again, and a stale/wrong updated latch fails closed.

The native portable adapter runs the source on the same `Machine` and devices
as the compiled engine. Main-region game effects belong only to the Rust
source. Actual admitted interrupts may use the already published, complete,
ROM/IR-authenticated native continuation table; these are explicitly counted as
**compiled-engine ISR instructions**, not as proved readable ISR roles or as an
original interpreter. The independent scheduled-Rust-ISR capability above
remains a distinct ownership profile. Unknown code fails closed without an
interpreter fallback.

After the source's real RTS, the adapter drains actual bus debt and eligible
interrupts at that boundary without executing the caller opcode. Only a
quiescent controller with the real return PC/prefetch, result, both stack banks,
SR and completed region can certify a return. The certificate is private,
move-only, and bound to its ledger, local call scope and unique region
invocation. Both the host root finalizer and nested prepared-call finalizer
consume it exactly once instead of issuing another RTS. Duplicate, foreign,
stale, wrong-kind and altered-live-boundary attempts reject without restoring
an old state. Absence preserves the ordinary untimed finalizer; a failed timing
attempt cannot manufacture that absence as a successful return. The ordinary
portable capability admits RTS exits only, not readable RTE wrappers.

Live completion authority remains in the reusable source state/service lease
across Context lifetimes. Each new HookContext opens one checked root scope at
its first ceiling before any legacy or framed body; prepared/nested frame
changes retain their own brands. Identity initializes once per live trajectory,
and pending/failed authority cannot be erased by constructing another Context.
Discarded check/probe/fixture trajectories instead get distinct speculative
ledgers; the owner moves live authority aside and restores it on every Result
path before propagating errors. Cache/scratch reuse is not authority cloning.
Hosts with no source descriptors keep timing services absent: ordinary roots do
not create completion identities or advance timing-ledger scopes.
Actual hardware reset completes before live completion/clock authority is
replaced; plans may remain cached. A failed speculative probe neither poisons
live state nor reactivates a ContinueOriginal-disabled root.

An admitted invocation's execution/callback failure, or any real receipt
protocol failure, returns `TimingExecutionFailure { entry, cause }`. The
owner's actual in-flight/pending/failed ledger phase also protects errors raised
by a readable wrapper or MMIO/clock checks after the source completed but before
consumption. Protection latches owned failure metadata with a relaxed atomic;
it preserves Host's existing Send/Sync contract without sharing fork authority.
Primary failures are classified before secondary checks and retained unchanged.
Callback errors retain their owned game-error type, reason and standard error
chain through the runtime adapter; transport does not replace them with a
generic IR-validation diagnostic. Already-fatal callback/wrapper causes still
poison outstanding authority before being propagated unchanged.
The typed failure is never an approved transfer: callers return it before any
SP/frame/register restoration or argument release, leaving actual CPU/device
state exactly at failure. Clean consumption ends this protection for unrelated
ordinary errors; a failed or pending invocation still cannot be rebound.

### Nested calls, substitution profiles and authority

Caller source owns the complete JSR/BSR instruction: argument/local preparation,
real return-address writes, register updates and its exact instruction/device
charge. The child is already physically prepared. The bridge must not push
another argument/RA, call an ordinary setup helper that uses `set_pc`, pre-read
or substitute its caller opcode, or release outgoing slots on return. Post-JSR
debt and legal IRQ admission occur in the child's inherited scope before its
first opcode. Child post-RTS waits/IRQs drain before its single sealed receipt
can resume the parent's authenticated return role. Actual callee registers and
CCR remain visible; no pre-call CPU image is restored.

Plans and buffers are not invocation authority. Immutable bindings are shared,
but every active depth owns distinct prepared scratch and its own affine
completion ledger. A parked non-Clone continuation keeps its storage leased,
so recursive A→B→A cannot alias an active parent's scratch. Per-function pools
reuse those buffers. A child's public frame changes bind only its new owner,
never `change_scope` on an in-flight parent. A consumed child does not consume,
renew or unpoison the suspended parent.
After validated RTS consumption, the controller closes the child ceiling
**before** finishing/popping its owner, resuming a cursor or restoring parent
frame/class metadata. An orphan/LIFO ceiling failure is still a child-finalizing
failure, even though its source ledger's RTS was consumed. The active child
binding retains protection until that transition succeeds; failure preserves
the child metadata, actual returned CPU/debt/queues and first cause without
caller argument cleanup.

The root initialized-byte map and chronological audit remain shared across
all nested paths, including original execution. The active root CallClock is
also shared: class and child ceilings tighten its absolute existing deadlines;
no region gets a fresh root allowance. Device steps charge it once. Inclusive
ancestor progress/call counters are observations, not aggregate cycle grants.
Any descendant admission/execution/receipt failure poisons the admitted chain,
retains the first cause and actual failed frame/CPU/device queues, and performs
no caller epilogue or argument cleanup. Discarded worlds move the entire nested
authority aside and restore live authority on every Result path. Their scratch
may be recycled only after discard; live debt clears only after actual reset.

Callee ownership is explicit and independent from both class and proof envelope:

| Runtime profile | Callee with selected timing roles | Callee without timing roles |
| --- | --- | --- |
| `HostCheckOriginalSubstitution` | Canonical selected timed Rust, including Atomic/Functional | Exact original, including a ready production Rust body, only under actual Host `Mode::Check` |
| `HostHybridReference` | Selected timed Rust | Existing unselected original hybrid path; never substitute behind selected ready untimed Rust |
| `NativeStrictReadable` | Every callee must be selected timed Rust | Startup rejection |
| `NativeCompiledOriginalCallees` (opt-in transitional) | Selected timed Rust | Only unselected/pending original continuations; selected ready untimed Rust rejects at startup |

Host CHECK substitution is labelled verbatim **“original-callee substitution
(callee proven separately)”**, counted separately, and never composition
evidence. Its receipt retains a separate-callee-proof obligation, so the label
does not certify an unproven/pending body. Actual immutable Host mode is carried
by trusted context/fork factories; neither audit presence, proof registry,
ClockedStrict nor fixture status can mint that permission. Reference/replacement
contexts do not inherit CHECK's ready-untimed substitution.

The native transitional profile executes authenticated compiled original
instructions, never an interpreter. Its original-callee driver intercepts
internal **actual** JSR/BSR boundaries too: selected timed targets hand off to
their Rust entry; selected untimed targets reject the startup closure. There is
no opaque-original-subtree exemption or original-root directive. These
**compiled-original CALLEE** counts are neither Rust nor ISR counts and grant
no readable/native-composition coverage. Unknown targets, ambiguous/uncovered
edges and unsupported nonlocal/terminal/RTE exits fail closed.

Receipt profile/kind and the separate-proof flag survive the handoff. Startup
closure caches bind the immutable dispatcher data-table selection, descriptor
set, runtime profile/whitelist and ROM/revision/divider/backend authority.
Table identity is not canonical body approval; independent diagnostic-origin
and resolved-body admission guards still apply. Changed selection/profile
requires explicit startup revalidation rather than a hidden warm-entry walk.

The nonleaf candidate is the public
`cutscenes::timed_clear_lower::clear_lower_vram_tilemap_framed`
(`G/src/cutscenes/title_and_transition.c:99`). Its source owns the saved loop
register, real WaitForNextVBlank call and caller cleanup, all 32×64 VDP writes,
actual saved-local reload and RTS. It activates no registry identity.

#### Proof-only structural call window

The non-default runtime feature `call-window-proofs` exposes a separate
`CallWindow` constructor for structural evidence, not a source-root entry.
It authenticates the **live settled** compiled JSR/BSR, opcode, pure control EA,
SP, immutable ROM/backend identity, actual committed caller extent and callee
edge. Only that authenticated callsite derives the additional transitive
selection root; no public arbitrary whitelist/seed is accepted. Class, exit,
ceiling ownership and transitive selected-body/role checks precede every call
effect. Callee bindings are genuine; the window has **no root BoundProgram**.

Rust owns that one actual call instruction's ordered prefetch/RA/SP effects.
The ordinary child controllers then execute original B and redispatch selected
timed C, retaining real RTS, prefetch debt and boundary IRQ ownership. The
returned observation is labelled **“structural call window (not whole-root
translation)”**. It certifies no whole caller, grants no readable coverage and
contains no reusable root invocation/completion authority. There is no original
root directive, snapshot import, fake source/root receipt or state repair.
The constructor, window and observation types do not exist without the feature;
both shipping frontend dependency guards explicitly forbid enabling it.
An admitted/fatal window error invalidates the existing shared clock without
repairing physical state or renewing private authority. Ordinary pre-admission
missing/stale input-latch errors do neither: the unchanged world and caller
clock scope remain retryable after correcting the caller-owned latch.
Its optional failure tap is diagnostic structural-window observation only:
copied frame/class and borrowed CPU/devices/first cause, after fatal protection
and poisoning. It receives no Context, mutable state, token/ledger or completion
authority and cannot replace the outcome. Pre-admission ordinary errors do not
run this admitted-failure tap.



The first reviewed source exemplar is `WaitForNextVBlank`: eleven source roles
validated against the user's revision at runtime, including the original prefix,
counter reads/poll branches, decrement/test and RTS. It preserves D0/D1 low-word
semantics and flags, and services actual counter-changing ISRs at the real
instruction boundaries. Both revision proofs compare every actual scheduler
step's CPU words, cycles, ordered bus accesses and all-origin RAM commits, then
the complete serialized CPU/device/audio state. The reported controller-edge
witnesses cross input frame 837 to 838 with 7,855 Rust source instructions, 313
explicitly original ISR instructions and 23,556 effects per revision; the
separate frame-482 fixture has 3,305/267/9,906 respectively. A seeded two-cycle
prefix mutation rejects in both fixtures and revisions.

Authored ROM-free native tests independently cover actual VDP IRQ4/IRQ6
nesting, both stack banks, real RTE, and the renderer's actual frame event
(which is not VBlank admission). Negative cases reject a missing ISR body or
callee closure, no progress, false RTE, foreign plan, fabricated zero-cycle
grant, fake bus acknowledgements and corruption of either stack bank. These protocol
proofs do not prove arbitrary callback game effects, all ISA/ISR source bodies,
or promote the exemplar into the production registry. Busy controller loops
and ISR wrappers retain their own complete source-region/closure obligations.

## Frontend playback

`Engine<B>::from_devices(DeviceRuntime, B)` reads the reset vectors and initial
prefetch without advancing device clocks, as the reference constructor does.
The frontend uses the same playback shape as `tje-host`:

* `play_frame(GenesisInputs) -> Result<&[[f64; 2]], B::Error>` borrows exact stereo
  samples until the next mutable operation; no frame/RAM snapshot is collected.
* `frame_buffer() -> (&[Color], FrameSize)` borrows the corresponding image.
* `target_fps()` retains the console's fractional refresh rate and interlace mode.
* `set_audio_sample_rate(u32)` accepts 8,000–192,000 Hz and reserves audio storage
  outside the realtime loop. The frontend must flush its output queue on changes.
* `reset()` recreates power-on devices/CPU, retains compiled continuations and the
  output rate, and resets `frame_count()` to zero.

`Machine<D, B>` is the generic scheduler used by synthetic tests. Snapshot and
instruction-proof tools use `cpu()`/`devices()` and opt-in trace buffers; normal
playback leaves those allocating observers disabled. Backend errors are returned
without inventing cycles or executing an interpreter fallback.

## Verification boundary

The test evaluator lives only in the development verification crate. It evaluates
primitive IR, not the production decoder, and is unavailable as a shipped backend
or a missing-PC fallback. Start oracle and IR from identical architectural state
and the same memory/bus responses. Compare D/A, both stacks, SR, PC/prefetch,
ordered byte/word reads/writes (including discarded/prefetch transactions), memory
and base cycles after each instruction. The runtime/device fork additionally
compares IRQ/noninstruction/wait steps and every video-boundary output listed
above. Synthetic streams cover illegal and privileged operations, odd addresses,
BCD, all sizes/EAs, carry/overflow boundaries, MUL/DIV operands, DBcc outcomes and
shift counts, not only the opcodes naturally reached by the game.

`--instruction-proof` compares all 25 CPU words, charged cycles, ordered tagged
68000 reads/issued writes, interrupt acknowledgements and actual RAM commits from
both CPUs after every scheduler step. Initial full-state/RAM equality plus equal
ordered actual commits establishes boundary RAM equality by induction without
copying 64 KiB per instruction. CPU-issued stores alone would miss Z80 writes.
Receipts distinguish scheduler steps, instructions, bus accesses and RAM commits;
frame-only replay is not reported as instruction equivalence.

The required retail campaign is the same six controller-only sessions as
CycleProto: stationary 1P, moving 1P, 2P separation, elevator/navigation, menu and
Jam Out; four 10,000-frame and two 12,000-frame sessions total **64,000 frames per
revision**. Report exact executed-instruction denominators and zero/nonzero
mismatches separately for each revision. Prototype timing receipts are not receipts
for this new IR. Whole-device native/WebAssembly claims require their own receipts;
a passing decoder test or a passing short session must not be substituted.

Committed tests and metadata contain no ROM-derived payload. Runtime listings,
caches, session snapshots and evidence that includes private state stay in
ignored scratch/user storage. The ordinary workspace gates remain ROM-free;
long private campaigns use the user's own ROMs.

## Event-horizon optimisation: proof obligation, not an enabled shortcut

The baseline deliberately returns after every instruction. No event-horizon
batching is enabled. An optimised region may suppress scheduler calls only if it
proves equivalence to the baseline for **every** original boundary inside it:

1. At entry there is no pending admitted IRQ, reset, HALT/STOP transition or
   unconsumed wait. SR/stack/exception effects and control flow are known.
2. An exclusive master-clock horizon bounds the earliest possible interrupt-line
   change, Z80 bus-ownership change, refresh/DMA/FIFO wait, input transition or
   other device event. The cumulative **worst-case** charge, including all
   operand-dependent instruction costs and bus waits, is strictly before that
   horizon. Equality is not sufficient: the event's admission ordering matters.
3. Every access is proven immutable ROM or private work RAM, never MMIO, banked
   memory, code modification, a faulting address, or memory that the Z80/devices
   can observe or mutate during the interval. Unknown EAs fail this proof.
4. Device bulk advancement is itself proven to produce the identical ordered
   events, sample bits, frame boundaries and state as the omitted per-instruction
   advances. A final-clock total alone is not such a proof. Deferred stores retain
   their original visibility times, and every instruction's boundary state remains
   reconstructible if an event invalidates the region before execution.

Only after these obligations hold can the region's accumulated charge replace
its individual scheduler calls. Otherwise dispatch the same already-compiled
single-instruction continuations; never fall back to an opcode interpreter or
coarsen IRQ delivery. CycleProto's block/function experiments disprove unconditional
batching, and its performance numbers do not prove this fast path or predict its
speed. Until an independently exercised proof and differential campaign exist,
leaving the optimisation disabled is the exact implementation.
