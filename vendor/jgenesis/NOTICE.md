# jgenesis standalone Genesis subset

Source: <https://github.com/jsgroth/jgenesis>, exact commit
`d41e2d975771c78ee2efe80f8ec5768b87e3f7eb` (version 0.14.1).
Copyright and license remain those of jgenesis and its contributors. The upstream
GNU GPL version 3 license is reproduced in `LICENSE`. No upstream fork is used.

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

Local changes are marked `TJE hybrid`, `TJE standalone cutover`, or `TJE timing survey` in code:

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
- `cpu/m68000-emu/src/{traits,debug,lib}.rs`: expose `ReadPurpose` and tagged byte/word reads with behavior-preserving defaults; preserve debugger callbacks for semantic and discarded reads; authored ROM-free CPU fixtures live in `lib.rs`'s test-only module.
- `cpu/m68000-emu/src/core/instructions/{bits,load}.rs`: mark CLR's read-before-write and memory-to-register MOVEM's terminal read discarded, not semantic stack dependencies; still perform the original reads.
- `cpu/m68000-emu/src/core.rs` and `core/instructions/controlflow.rs`: observe successfully entered/returned exception frames and accepted vector identities without changing CPU behavior/timing; native RTE finalizers notify the same observer. A rendered frame alone is not a safe fixture-call boundary.

Additional `TJE hybrid` check-contract extensions (disabled by default):

- `backend/genesis-core/src/bus.rs`: an explicit check-fork-only functional mode completes VDP transaction latency after each port write and settles YM write BUSY; direct VDP status/HV observations are flagged as contract violations. BUSACK/reset semantics are unchanged. An opt-in chronological CPU RAM/device audit records timed calls without changing dispatch.
- `backend/genesis-components/src/vdp.rs`: `hybrid_complete_transfers` drains FIFO and progresses DMA through the original commit routines without advancing raster clocks; `hybrid_logic_state_eq` compares the explicitly documented logical projection used by time-aware function checks. Deferred CPU port writes replay the original prefix of their existing vector without a ten-entry scratch limit; writes deferred again by a chained DMA retain issue order. The physical four-entry FIFO, DMA progression and ordinary instruction timing are unchanged. The existing full-state comparator is retained for atomic/functional checks.
- `backend/genesis-components/src/vdp/{debug,fifo,sprites,cramdots}.rs`: cold, bounded read-only field diagnostics for the existing full/logical VDP comparison scopes. Register indices, control/FIFO/DMA latches and memory addresses are reported with both values; no equality, timing, FIFO, memory or device-access behavior is changed.
- `backend/genesis-components/src/ym2612.rs`: `hybrid_settle_write_busy` settles only the fixed register-write BUSY countdown for functional check forks; no timer, oscillator, status-read or sample algorithm is replaced.

No constructor or ordinary timing path enables functional mode. The live machine
and time-aware waits use unmodified FIFO/DMA/BUSY timing. Function-level logical
projections do not substitute for the unmodified-upstream whole-session comparison.

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
