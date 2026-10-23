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

## Timeline, exceptions and devices

Base instruction charges are not elapsed time. The runtime retains the reference
order: consume refresh/FIFO/DMA/Z80 waits before CPU issue; execute one compiled
instruction's ordered accesses; advance Z80, controller/PSG, VDP and YM by the exact
master clocks; update interrupt latches; then publish deferred CPU writes. Long
port stores remain two transfers. Read compensation does not advance devices or
fabricate ready/busy values. Z80 bus ownership, YM busy state, VDP FIFO/DMA and
refresh remain live device state, not fixed instruction surcharges.

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
path; they are removed from this vendored build. This is an explicit decision
about the reference's post-chip resampling arithmetic, not a change to game
instructions, device clocks or sound-chip register streams.

ROM-free production-device goldens program authored PSG and YM2612 register
sequences at 44.1 and 48 kHz and compare sample-bit SHA-256 hashes. The same
fixture runs on native CI and in the browser proof harness. Linux/Windows native
measurements, browser measurements and the explicit macOS ARM64 CI capacity gate
cover the canonical path; unexecuted CI/platform results must not be called local
evidence. Audio receipts from the earlier feature-selected path are historical
and must be regenerated before supporting this policy's exactness claims.

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
