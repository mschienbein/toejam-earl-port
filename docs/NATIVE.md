# Native devices and execution boundary

## Fidelity decision

The native target is **cycle-exact to the pinned jgenesis reference**. An earlier
logic-exact baseline is not the shipped schedule: the cycle prototype established
that even basic-block interrupt delivery can change RAM and sound. Native game
execution supplies an instruction continuation, its exact base CPU charge and its
originally ordered bus effects. No 68000 emulator object or original-call fallback
is present in `tje-native-platform`'s normal dependency graph.

This is reference exactness, not a new claim of transistor-/physical-console
accuracy. The retained reference approximates refresh and quantizes device effects
at instruction boundaries; [TIMING.md](TIMING.md) records those limits.

The cycle-exact device API builds by default. The separate readable `NativePlatform`
adapter is enabled with `--features game-adapter`. This feature is temporarily
non-default for the first engine integration while its shared host-exit and
GamePilot frame-validation APIs land; enable it by default in the corresponding
follow-up only after its explicit feature gate and readable proofs pass. Device,
runtime and backend proofs never require this readable adapter.

## Two consumers, one device state

`crates/tje-native-platform` has two public entry points:

- **`DeviceRuntime` / `ClockedDevices`**: cycle-driven devices for generated native
  instruction continuations. The CPU/continuation runtime owns PC, CCR, banked
  stacks, exceptions, STOP and the instruction-boundary interrupt-admission rule.
- **`NativePlatform`**: an implementation of `tje_game::Context`, `tje_game::Platform`
  and `tje_mem::Bus` for the existing readable Rust translations and closed-subsystem
  differential fixtures. Its immediate port facade is deliberately not presented
  as cycle-exact execution of an ordinary C-to-Rust function.

Both store the only work-RAM image in `tje_mem::AddressSpace`: 64 KiB, big-endian,
with the original E00000–FFFFFF mirrors. ROM pointers remain emulated addresses.
The caller supplies its cartridge bytes at runtime. Z80 RAM, VDP, YM2612, SN76489,
Z80 CPU and controller pin state are the retained jgenesis implementations, not
register stand-ins. The Z80 executes the original driver uploaded by game startup
from that user's ROM. No sound driver or extracted asset is compiled into the port.

## Exact instruction/device contract

`ClockedDevices` is the timing interface. The caller:

1. Publishes the instruction PC with `set_instruction_pc`, then calls
   `begin_instruction(opcode, read_offset_mclk)`. The continuation can update the
   read offset before a status/HV read. This preserves the reference's explicit
   instruction-read look-ahead without a 68000 opcode decoder in the device crate.
2. Calls `take_wait_step(pc)` before executing semantics. A returned step consumes
   the reference's pending refresh, DMA or bus-steal wait; no instruction executes.
   If there is no pending wait, it checks the actual interrupt/halt lines and
   executes one instruction continuation or the appropriate CPU-owned exception step.
3. Performs byte/word reads and queues byte/word writes through the bus interface.
   Long accesses are two ordered word operations. Stores, including RAM stores,
   become visible at the original post-instruction publication point.
4. Calls `finish_instruction(pc, base_cpu_cycles)`. Base charges must include the
   operand-dependent instruction timing; the device crate does not invent it.

The retained `genesis_core::timing::CycleCounters` accounts for divide-by-seven
CPU clocks, ROM/RAM refresh, one-cycle main-to-Z80 penalties, Z80-to-main bus steals,
VDP ownership and optimized long-DMA waits. Each step preserves the reference order:

1. Increment the master timeline and refresh/wait ledger.
2. Run due Z80 T-cycles at divide-by-15, preserving banked ROM access and bus stalls.
3. Advance controller timeouts and divide-by-15 PSG clocks.
4. Advance the VDP, FIFO/DMA, rendering and interrupt pending state.
5. Synchronize divide-by-42 YM clocks and collect chip samples at the reference's
   drain points, using the retained audio filters and sinc resamplers.
6. Apply interrupt-enable latches on non-wait steps, then commit queued CPU writes.
7. Drain ready stereo output, preserving upstream floating-point addition order.

`pending_interrupt()` returns the real VDP level. `acknowledge_interrupt()` performs
its real pending-latch acknowledge. R10 counter/reload, enable latches, VBlank and
line IRQ generation therefore remain device behavior, not hard-coded frame or split
callbacks. The continuation scheduler chooses the eligible original-instruction
boundary, pushes the real exception frame and dispatches the generated handler.
The device layer never accepts an IRQ while the CPU mask forbids it.

An exact noninstruction interval can be supplied through `advance_master_clocks`.
It preserves fractional device phases and splits only to respect the retained VDP's
maximum tick size. It does **not** accept interrupts or replace the instruction
runtime's STOP/exception state machine. Readable-layer waits use it with actual
RAM/device predicates and fail if their explicit bound expires; they never force
BUSACK, YM BUSY or VDP DMA-ready bits.

## Platform ports

The native bus retains the standalone Genesis mappings used by the game:

- VDP data, control/status, HV and debug aliases; byte duplication and ordered words.
- DMA reads from runtime ROM and the same `tje-mem` RAM image, plus VDP fill/copy.
- Z80 BUSREQ, active-low reset, BUSACK-gated RAM and YM access; word accesses retain
  the original single-byte write / duplicated-byte read behavior. Reset also resets
  the YM chip on its original assertion edge.
- YM address/data latches and real BUSY/timer state; PSG byte-lane gating.
- Controller data/direction/serial registers, including the actual TH protocol,
  pull-up delay and unplugged pin behavior from the retained `InputState`.

The readable context's dispatcher is constructed with `Dispatcher::native` over the
chosen roots **and installed IRQ roots**. Startup reports the complete missing
translation list. An unexpected untranslated call returns `Error::Untranslated`;
there is no hybrid fallback. A closed subsystem is not a claim that power-on game
startup is already a fully translated closure.

The readable context persists real `FrameState` and banked USP/SSP. An IRQ entry
installs its exception-frame SP, dispatches the validated translated wrapper and
consumes only its explicit `ExceptionReturn` event. RTE pops the six-byte SSP frame,
restores SR/PC and then restores the interrupted caller's `FrameState`; a normal
RTS, unrelated control transfer or missing handler is not silently treated as RTE.
Other root transfers remain explicit `EntryOutcome` values for a caller to consume.
Prepared ordinary `invoke` entries reject an odd entry SP before execution and
validate the actual RAM return PC before reporting success. Odd control targets
fail closed through `FrameError`; the readable adapter does not fabricate a CPU
address-error exception or silently align the target.

## Capture and differential proof

`GenesisEmulator::capture_native_state` clones peripheral state, CPU register values,
work/Z80 RAM, exact clock counters and the resampler. `DeviceRuntime::from_state`
imports it into `tje-mem`. Captures live only in memory/private scratch and contain
no cartridge bytes; they are not a public save-state or distribution format.

Two opt-in private-ROM tests provide the first native/reference paths:

- `translated_subsystems_match_original_from_live_captures`: boot the real game,
  prepare original MCC68K entries and execute closed readable translations with
  **no 68000 in the native candidate**. Compare live RAM, Z80 RAM, full VDP/FM/PSG
  state and ordered actual chip writes. Original compiler dead-stack differences
  are counted explicitly, not reported as whole-RAM or cycle-exact equivalence.
- `live_sound_driver_and_every_device_match_reference_on_exact_cycle_steps`:
  import the original game's live sound driver and clock phase, then drive native
  and reference devices through identical instruction/wait charges. Compare clock
  ledgers, full mutable VDP/chip/Z80 state, actual timestamped chip dispatches and
  every stereo sample bit/count. This validates the retained-device boundary,
  not yet every generated game continuation.

Tracing is opt-in. `enable_trace` records main/Z80 origin, PC, integer master time,
widths and actual VDP/YM/PSG dispatches; `clear_trace` retains capacity and
`disable_trace` drops the proof buffers. The reference's
`GenesisBus::enable_native_write_trace` captures the same actual-dispatch types,
using the passive survey's origin/PC (rather than mistaking attempted MMIO stores
for accepted chip writes). Observers never alter device time.

For instruction-level proofs, `GenesisBus::native_cpu_accesses` is a separate
default-off journal of **CPU-issued** reads, writes and interrupt acknowledges,
including ROM prefetch. Addresses are normalized to 24 bits; read purpose is
explicitly 0=semantic, 1=prefetch, 2=discarded. Writes are recorded before queuing,
not again at commit. Z80 bank reads preserve their original bus effects but do not
appear as main-CPU issues. `M68000::hybrid_state_words()` exposes the 25-word
instruction-boundary register/pending/STOP/fault state consumed by the compiled
engine verifier. These reference-only observations do not enter the device-only
normal dependency graph.

`native::RamWrite` and the separate optional RAM-commit journal cover actual
work-RAM stores from **both processors**, after device advancement and queued
CPU write publication. Canonical FFxxxx addresses and word alignment make mirrors
comparable. Equal initial RAM, identical explicit recipes and equal ordered
all-origin commits establish RAM equality at every instruction boundary without
copying 64 KiB per step; the CPU-issued journal alone would miss Z80 stores.
`enable_ram_write_trace`, `clear_ram_write_trace`, `disable_ram_write_trace` and
`ram_write_trace` provide the native side without allocation when disabled.

Playback borrows the VDP frame and `audio_samples(): &[[f64; 2]]`; `clear_audio`
reuses capacity. Sample-rate configuration reserves a complete frame before the
realtime loop. Presentation and audio callback cadence do not change device clocks.
The NTSC period remains 3420 × 262 master clocks, not a rounded 1/60 second.

## Build and reproduce

The default `genesis-core/emulator` and `genesis-components/m68000-timing` features
preserve existing hybrid builds. The native device crate disables both. The
68000 appears only in its development/reference-test graph, not normal builds.
Browser entropy uses `getrandom/wasm_js`; no native-only device path is required.

```sh
cargo build --locked -p tje-native-platform
cargo build --locked -p tje-native-platform --target wasm32-unknown-unknown
cargo test --locked -p tje-native-platform
TJE_REV02_ROM=/private/rev02.bin TJE_REV00_ROM=/private/rev00.bin \
  cargo test --locked -p tje-native-platform --test differential -- --ignored --nocapture
cargo tree --locked -p tje-native-platform --edges normal
```

The full workspace release gate still applies. Commands and observed results for
this implementation are recorded in the private lane report; build instructions
alone are not evidence that a target or scene was executed.
