# jgenesis standalone Genesis subset

Source: <https://github.com/jsgroth/jgenesis>, exact commit
`d41e2d975771c78ee2efe80f8ec5768b87e3f7eb` (version 0.14.1).
Copyright and license remain those of jgenesis and its contributors. The upstream
GNU GPL version 3 license is reproduced in `LICENSE`. No upstream fork is used.

## Deterministic audio (TJE native, ER-001)

`common/dsp/src/{lib,sinc}.rs` use the same portable scalar sinc convolution
on every target. The retained scalar coefficient interpolation and tap-order
accumulation use separate multiply/add operations; CPU-dependent AVX2/AVX512
FMA kernels, lane reductions, feature dispatch and compile-time override flags
are removed, not disabled through environment variables. SIMD-only buffer
padding and tests are removed; scalar compaction/order/unfused-operation
regressions replace the former tolerance-based SIMD comparisons.

Coefficient and chip-table generation also use one implementation: registry
dependency `libm` is pinned to **0.2.16**, with default features disabled and
`force-soft-floats` enabled in the vendored workspace. That feature overrides
architecture and unstable-intrinsic selection even if another dependency
enables them through Cargo feature unification.

- `common/dsp/src/design.rs`: Butterworth poles use pure-Rust cosine/sine at
  the original angle, instead of `E.powc` and its hidden `ComplexFloat`
  hypot/atan2/log/exp path. Frequency warping uses pure-Rust tangent. Exact
  alternating polynomial signs use integer parity rather than floating powers.
- `backend/genesis-core/src/audio.rs`: configured decibel gains use pure-Rust
  power. The oracle and native runtime share this same resampler/filter code.
- `backend/genesis-components/src/ym2612.rs`: lazy log-sine and power tables
  use pure-Rust sine, log2 and power before the unchanged integer quantization.
- The corresponding three package manifests and workspace manifest select the
  same pinned software backend; no per-platform math fallback remains.

The rest of the shipped audio closure needs no transcendental implementation:
PSG amplitudes and both sinc FIR kernels are precomputed public constants;
PSG/YM oscillators and clocks are integer logic; IIR filters, scalar convolution,
gain application and mixing use ordered basic arithmetic. Sinc fixed-point
rounding/fraction extraction, table quantization and buffer-capacity ceiling
retain their specified rounding behavior. Remaining complex operators are
addition/subtraction/multiplication/division, not `ComplexFloat` functions.

This intentionally establishes a new cross-target sample-bit baseline. Audio
receipts made with the previous CPU-dispatched implementation must be rerun;
they do not prove the scalar/software-math implementation. This software-math
cutover also requires rerunning receipts made with platform coefficient/table
generation. FIR constants, resampling ratios, filter topology, chip scheduling
and sample mixing order are unchanged. Coefficient/table bits may change from
the former platform-libm results; cross-target bit proofs and any intentional
golden refresh belong to the synthetic chip-sequence and session verification
suites, not to source selection alone.

## Crates retained

- `common/dsp`, `common/jgenesis-common`, `common/jgenesis-proc-macros`
- `cpu/m68000-emu`, `cpu/z80-emu`
- `config/genesis-config`
- `backend/genesis-components`, `backend/genesis-core`, `backend/ti-sn76489`

This is **standalone cartridge Genesis only**. The upstream `s32x-core` embeds
console boot-ROM/vector images, so it must not be part of a distributable port,
not even as an unused dependency. `s32x-core`, `segacd-core`, `cdrom`, and
`sh2-emu` are not vendored or in the build graph. No firmware, ROMs, BIOSes,
extracted game assets, or frontend crates are included. The retained config crate
still declares upstream addon configuration data types; these cannot instantiate
addon hardware or pull addon code/firmware into the dependency graph.

## Modified files

Local changes are marked `TJE hybrid`, `TJE standalone cutover`, `TJE timing survey`,
`TJE native`, or `TJE reference proof` in code:

- `backend/genesis-core/Cargo.toml`: remove CD, 32X, SH-2 and addon debugger-test dependencies.
- `backend/genesis-core/src/api.rs`: standalone-only construction/reset/render/tick/save API, remove addon/channel-debugger paths, and add narrow CPU/bus borrow accessors for instruction-boundary interception.
- `backend/genesis-core/src/lib.rs`: re-export the bus type, opt-in hardware-write events and ordered 68000 device-read/write observations.
- `backend/genesis-core/src/bus.rs`: remove addon fields, memory-map dispatch, clocks and debugger coupling; expose pending-write flush; record VDP/YM2612/PSG writes at chip dispatch, including Z80 writes; optionally observe ordered byte/word 68000 device reads and writes without altering dispatch; derive equality for Z80 bus/reset signals and pending CPU writes.
- `backend/genesis-core/src/audio.rs`: remove absent CD PCM/CD-DA/32X PWM models and resamplers; retain Genesis filters/resamplers and the upstream standalone sample addition order (including zero contributions for IEEE-754 bit identity).
- `backend/genesis-components/src/vdp.rs`: immutable VRAM/CRAM/VSRAM/register accessors, public typed register definitions and allocation-free comparison of every mutable VDP port/render state field (including control address/code/latch, DMA, FIFO, pending writes, sprite state, CRAM dots, pixel buffers and latched registers).
- `backend/genesis-components/src/vdp/{registers,fifo,sprites,cramdots}.rs`: derive equality for the typed state borrowed by hybrid comparisons; no device behavior changes.
- `cpu/m68000-emu/src/core.rs`: readiness query prevents hooks before IRQs/stalls; tag instruction prefetch and value-discarding operand reads without removing any bus access or changing instruction timing.
- `backend/genesis-core/src/timing.rs`: a new, isolated `survey` module supplies opt-in hardware-access timestamps, CPU/interrupt origin, and explicit experimental write queues. Experiment-only bursts respect real VDP FIFO/DMA backpressure; no observer or policy is installed in ordinary emulation. The original clock algorithms are unchanged.
- `backend/genesis-core/src/{api,bus,lib}.rs`: additive timing-survey hooks at instruction, interrupt, device-read/write, and peripheral-advance boundaries. Existing deferred CPU-write queues are unchanged.
- `backend/genesis-components/src/vdp.rs`: immutable DMA-active query for counting actual starts, not command-shaped words.
- `backend/genesis-components/src/vdp{,/debug}.rs`: pure scalar `HybridVdpCheckpoint` inspection of FIFO count, separately deferred FIFO and DMA DATA/CONTROL port-write counts, actual DMA activity/slot-attempt latency, data-read wait and the half-written control-command latch. Exhaustive state patterns require explicit review when state fields change. This does not read MMIO, acknowledge IRQs, reset a latch, flush pending writes or advance devices.
- `backend/genesis-core/src/{bus,lib}.rs`: publicly re-exported passive `HybridBusCheckpoint` reports pending byte/word queue counts, CPU wait debt, separately retained fractional Z80 bus-stall rounding and CPU-refresh phases, bus ownership/halt/reset signals and the exact existing IRQ/halt predicates. Neither phase counter is treated as outstanding debt or required zero by inspection. No queue, state or observation stream is changed.
- `backend/genesis-components/src/memory.rs` and `backend/genesis-core/src/{bus,api}.rs`: exhaustive, borrowed development-proof Encode views retain every non-VDP/non-main-RAM machine field, including CPU, Z80, audio/resampler, controller input, cheats, pending queues, clocks and observation provenance. Physical main RAM is separately exposed by immutable word borrow, not cheat-substituted reads. Encoding does not clone, rewrite, normalize or advance a machine; callers must separately compare RAM outside their exact actual input domain and the entire VDP.
- `backend/genesis-core/src/{bus,api}.rs`: an additive borrowed
  `DiscardedReadValueVariance` probe-control Encode view excludes only the
  separately checked `native_cpu_accesses` contents, retaining its presence and
  length and every other existing control field. The ordinary full view is
  unchanged. This performs no clone, normalization, machine write or advancement;
  the host must first validate the complete original CPU journal under the named
  strict baseline-to-original probe contract.
- `backend/genesis-components/src/vdp.rs`: an exhaustive borrowed conservative control-superset Encode view retains ports, DMA/FIFO, mode/registers, pending/read-wait, IRQ/raster/configuration and event state while excluding explicit render/memory data caches. Full FIFO and InternalState data latches remain included, so this view may conservatively reject a data-only case rather than silently widen approval. Complete cross-language VDP equality and real VRAM/output sensitivity remain separate obligations; no ordinary device algorithm calls this view.
- `cpu/m68000-emu/src/{traits,debug,lib}.rs`: expose `ReadPurpose` and tagged byte/word reads with behavior-preserving defaults; preserve debugger callbacks for semantic and discarded reads; authored ROM-free CPU fixtures live in `lib.rs`'s test-only module.
- `cpu/m68000-emu/src/core/instructions/{bits,load}.rs`: mark CLR's read-before-write and memory-to-register MOVEM's terminal read discarded, not semantic stack dependencies; still perform the original reads.
- `cpu/m68000-emu/src/{core,traits,lib}.rs` and `core/instructions/controlflow.rs`: observe successfully entered/returned exception frames and accepted vector identities without changing CPU behavior/timing. The additive default-no-op bus callback emits the publicly re-exported `ExceptionBoundary` on successful entry and architectural RTE completion before peripheral advance; native RTE finalizers notify the same observer. A failed architectural RTE emits no return marker. A rendered frame alone is not a safe fixture-call boundary.
- `backend/genesis-core/src/bus.rs`: the reviewed zero-count DMA proof can opt into all-source DMA observations, including ROM/open-bus beats, through a default-off flag. The actual read and device effects still occur once; ordinary emulation/checks retain their existing RAM-only observer. The host restores the flag and trace across the entire root on success or error.
- `cpu/m68000-emu/src/core.rs`: opt-in actual opcode-attempt/exception counters
  stay outside physical/save-state equality. The read-only pause-state query lets
  a whole-call ledger temporarily observe real ticks without resetting an existing
  live observer. A newly installed diagnostic observer remains paused afterward;
  its presence and counts do not alter architectural state or emulated timing.

Additional `TJE hybrid` check-contract extensions (disabled by default):

- `backend/genesis-core/src/bus.rs`: an explicit check-fork-only functional mode completes VDP transaction latency after each port write and settles YM write BUSY; direct VDP status/HV observations are flagged as contract violations. BUSACK/reset semantics are unchanged. An opt-in chronological CPU RAM/device audit records timed calls without changing dispatch.
- `backend/genesis-core/src/bus.rs`: the same opt-in actual-access stream records VDP DMA work-RAM source words with an explicit `DmaRead` origin. RAM writes are recorded only at memory commit, never CPU issue; both timed ticks and check-only transfer completion use the observer. No observer/log allocation is installed for ordinary emulation, and no source read, DMA/FIFO behavior, clock or `ReadPurpose` changes.
- `backend/genesis-core/src/{bus,lib}.rs`: default-off `HybridAccess::ExceptionBoundary` indices reference a separately optional, publicly re-exported `HybridExceptionBoundary` payload, keeping architectural entry/RTE markers in the actual stream without embedding register snapshots in ordinary accesses. Distinct `HybridAccess::Z80Write` records banked Z80 RAM stores after physical commit; `HybridAccess::HostMutation` identifies externally applied timeline writes, not CPU producers. These observations never force a queue flush, synthesize a source read or change CPU/device scheduling.
- `backend/genesis-core/src/{bus,lib}.rs`: separately armed, fork-only owned DMA-start scope observations retain real entry/current exception depth and actual dispatch origin/PC/clock/address/value. Successful architectural exception markers update this observation before the optional indexed logger, including RTE-before-peripheral-advance ordering. Unknown/out-of-order scope and active/deferred DMA DATA/CONTROL port writes latch rejection even after queues drain; internal chaining is not silently attributed to a root. Default `None` performs no log allocation or algorithm/timing change. No record grants execution/IRQ authority, fabricates a bus value, flushes a queue or acknowledges an interrupt.
- `backend/genesis-components/src/vdp.rs`: `hybrid_complete_transfers` drains FIFO and progresses DMA through the original commit routines without advancing raster clocks; `hybrid_logic_state_eq` compares the explicitly documented logical projection used by time-aware function checks. Deferred CPU port writes replay the original prefix of their existing vector without a ten-entry scratch limit; writes deferred again by a chained DMA retain issue order. The physical four-entry FIFO, DMA progression and ordinary instruction timing are unchanged. The existing full-state comparator is retained for atomic/functional checks.
- `backend/genesis-components/src/vdp/{debug,fifo,sprites,cramdots}.rs`: cold, bounded read-only field diagnostics for the existing full/logical VDP comparison scopes. Register indices, control/FIFO/DMA latches and memory addresses are reported with both values; no equality, timing, FIFO, memory or device-access behavior is changed.
- `backend/genesis-components/src/ym2612.rs`: `hybrid_settle_write_busy` settles only the fixed register-write BUSY countdown for functional check forks; no timer, oscillator, status-read or sample algorithm is replaced.
- `backend/genesis-core/src/bus.rs`: expose the existing pure `Z80Signals::busack` query plus read-only `bus_requested`/`reset_asserted` field accessors for host drive-mask/seed-receipt inspection. Visibility/inlining only; the signal formula, fields, read values and normal timing are unchanged.

No constructor or ordinary timing path enables functional mode. The live machine
and time-aware waits use unmodified FIFO/DMA/BUSY timing. Function-level logical
projections do not substitute for the unmodified-upstream whole-session comparison.

Additional `TJE native` device-only interfaces:

- `Cargo.toml` and `backend/{genesis-core,genesis-components}/Cargo.toml`:
  default-on `emulator` / `m68000-timing` features preserve existing builds.
  Disabling them removes the 68000 dependency; Z80, VDP, FM, PSG, controllers,
  resampling and clock counters remain available.
- `backend/genesis-core/src/{lib,api}.rs`: runtime-only peripheral captures
  contain cloned devices, clock phases, work/Z80 RAM, register values and the
  resampler, never cartridge bytes or a 68000 execution object.
- `backend/genesis-core/src/{lib,timing}.rs`: expose the unchanged clock counters
  and retained input model without the emulator-only survey module.
- `backend/genesis-core/src/{audio,bus}.rs`: move the chip sample sink trait to
  the CPU-independent audio module; share the existing filters/resampler.
  Expose the existing peripheral tick for differential proofs. Optional actual
  chip-dispatch timestamps retain CPU origin and PC from the passive survey;
  no observer is installed in ordinary emulation.
- `backend/genesis-components/src/vdp.rs`: explicit status/HV read offsets let
  native continuations preserve reference read look-ahead without linking an
  opcode decoder. Zero-offset access samples the scheduler's current clock.
  The emulator's existing opcode-specific adjustment and read effects are unchanged.
- `backend/genesis-core/src/{lib,bus}.rs`: an optional CPU-issued journal covers
  all physical addresses, stable semantic/prefetch/discarded purposes, prequeue
  stores and interrupt acknowledgements. Z80 bank reads retain their existing
  effects but bypass this main-CPU-only observation.
- `cpu/m68000-emu/src/core.rs`: a read-only 25-word instruction-boundary state
  accessor exposes the register file, banked stacks, prefetch, pending IRQ and
  STOP/fault bookkeeping for compiled-native differential checks.
- `backend/genesis-core/src/{lib,bus}.rs`: a separate optional actual RAM-commit
  journal includes CPU and Z80 stores, canonicalizes mirrors/word alignment and
  records only after writes commit. It is disabled by default like the CPU journal.
- `backend/genesis-core/src/api.rs`: timing-only check regions publish input and
  opcode attribution before Rust issues effects, then use the same extracted
  device/queued-write/audio/render/save tail as ordinary CPU ticks. The additive
  completion API never executes a main 68000 opcode or invents physical bus
  microcycles; existing issue/advance/commit ordering is retained.
- `cpu/m68000-emu/src/core.rs`: a pipeline-only completion setter updates real
  PC/prefetch and instruction bookkeeping after acknowledged Rust effects,
  without fetching again or writing game registers, stacks, flags or memory.
- `backend/genesis-components/src/{vdp,ym2612}.rs`,
  `backend/genesis-core/src/audio.rs`, and `common/dsp/src/sinc.rs`: additive
  startup/import capacity reservation only. Both sprite scratch lists reserve
  the hardware's 80-entry SAT maximum, also covering disabled sprite limits.
  The existing VDP `<1250`-master-clock tick bound is exposed unchanged.
  `cpu/z80-emu/src/core/instructions.rs::parse_opcode` increments its 16-bit PC
  for each DD/FD prefix. The native read map's unconditional `$6000 = $FF`
  prevents a second bank traversal; at most 32768 prefix reads charge the 68k
  bank bus. The separate 32-byte VDP streak stops at `$7F20 = $FF`. The largest
  non-prefix body is six accesses: ED/subopcode, address word, data word
  (`instructions/load.rs::{ld_direct_dd,ld_dd_direct}`). CALL, stack exchange
  and indexed CB need at most five; IRQ/NMI entry writes two bytes in the pinned
  core (including its explicit mode-1 fallback for IM2), and block-repeat
  handlers perform one iteration. Three trace-mode reads in `execute` are
  included even when logging is disabled. Each charged access adds `3*15`
  clocks and the enclosing Z80 tick adds 15. Adding the 3420-clock YM lag,
  strict VDP tick bound and 42-clock YM remainder gives 1479692 clocks, or at
  most `ceil(ceil(1479692/42)/24) = 1468` newly buffered YM samples.
  This is the pinned reference/native-map contract, not a new physical Z80
  timing claim. Independent real-core opcode/branch/prefix/IRQ tests, and actual
  native banked exchange/maximal-prefix bursts near the YM synchronization
  threshold, exercise the sizing without changing CPU or clock algorithms.
  The two sinc output queues reserve the sum of their actual filter-window,
  source-phase and bounded synchronization-burst output maxima at current
  rates. Existing queued samples are retained in addition to that free capacity.
  Reservation changes no vector length, contents, ring index, filter history,
  sample arithmetic, clock, FIFO/DMA state or serialized physical state, and
  performs no bus read, tick, flush or drain. The native adapter invokes it after
  both construction and imported state, and on a cold output-rate change, never
  on source entry. All four paths already carry `modified: true` provenance in
  `UPSTREAM.json`; their original upstream paths/blobs are unchanged.

- `backend/{genesis-core,genesis-components}/Cargo.toml` and
  `backend/genesis-core/src/{lib,api,bus,timing}.rs`: default-off
  `reference-proof-hooks` feature and opt-in original-execution/physical-commit/
  actual-Z80-tick-tail bridge. The shared observer lives in the existing optional
  timing survey, so strict full-control encodings retain its provenance without
  weakening exhaustive bus-field bindings. Ordinary construction retains the
  original reset execution and ignored charge; there is no added device advance.
- `backend/genesis-core/src/reference_proof.rs` and its ROM-free tests are local
  additions. One explicitly authored, non-natural audio fixture may write only
  physical Z80 byte `0x41` once at the authenticated original owner BUSREQ take
  and restore its saved zero once after exactly 112 eligible actual Z80 ticks
  following the first original retry release. There is no persistent override,
  CPU/request edit, inferred delay, readiness substitution or natural admission.
  Typed commit/tick/byte receipts, sticky failures and nonserializable live-origin
  authority prevent failed, decoded or speculative evidence from being upgraded.
- `backend/genesis-components/src/memory.rs`: feature-gated physical target-byte
  writer journal observes aliases and same-value writes, preserves unexpected
  writes, and invalidates opaque mutable RAM access. No watch is installed in
  normal operation. The feature and its optional dependencies are excluded from
  shipping frontend graphs; the package shipping-graph test guards both native
  and actual WebAssembly/browser dependency resolution. See the core README.
- `backend/genesis-core/src/reference_proof/{reset,reset_tests}.rs` are local
  additions to the same proof-only observer namespace. The independent passive
  lane captures actual create/soft/hard-reset raw line, vector/prefetch bus
  accesses and CPU/bus/clock transitions. It retains the actual reset return
  charge separately from actual device progress; the existing ignored charge
  remains ignored. No CPU/device/time change or `_start` source credit occurs.
  Fresh input is opaque, nonserializable and one-shot, separate from archival
  facts and audio authority. Progress/escape/import/copy guards prevent stale,
  same-value, decoded or analysis-reset revival. Hard-reset reconstruction moves
  only independently trusted reset lineage. Capture overflow/fault fails closed.
  The existing proof-only feature and shipping-graph guard cover this lane.

Removed upstream source modules, rather than leaving unsupported feature paths:

- `backend/genesis-core/src/api/debug.rs`
- `backend/genesis-core/src/api/debug/{s32x,tests}.rs`
- `backend/genesis-core/src/bus/debug.rs`

These channel-debugger modules were coupled to Sega CD/32X; `tje-host` implements
its own synchronous function-boundary observation. The standalone 68000/Z80/VDP/
YM2612/PSG timing, DMA and memory-access branches otherwise retain upstream logic.
The constructor is now `GenesisEmulator::create(cartridge_rom, config, saves)` and
is infallible because standalone cartridge construction needs no firmware/disc I/O.
Upstream multi-hardware save-state encodings and debugger commands are not a
compatibility interface of this subset.

`Cargo.toml` is a new nine-crate workspace manifest retaining the required upstream
dependency versions and lint settings. `NOTICE.md` is new. All other copied files
are unmodified. The parent workspace excludes this nested workspace; `tje-host`
depends on it by path. Browser entropy is selected by `tje-host`'s wasm-only
`getrandom/wasm_js` feature, matching upstream's web frontend. No atomics or nightly
compiler configuration is required by this subset.
