# Plan: the faithful port

## Decisions (user, 2026-10-23)

- Rust; WebAssembly for the browser; GPU rendering where it makes sense.
- A new repository that links to the decompilation. GPL-3.0, so components of
  [jgenesis](https://github.com/jsgroth/jgenesis) (GPL-3.0) can be used.
- Faithful first. The extras (widescreen, online co-op, quality of life, saves, enhanced
  graphics, new content and modes, modding) come after the faithful port works.
- **Cycle-exact.** The faithful port reproduces the cartridge bit for bit: identical frames
  and identical audio samples, not just identical game state. Chosen after the timing survey
  ([TIMING.md](TIMING.md)) and a prototype showed that anything coarser than original
  instruction boundaries changes split-screen frames, audio and RAM.

## What "faithful" means

For the same controller input, the port produces exactly what the cartridge (as modelled by
the reference emulator) produces: identical work RAM, VDP memories, rendered frames, audio
samples and sound-chip register streams, frame by frame. Because CPU timing is reproduced,
the original's idle random draws and lag frames come out identical too; they are no longer
exceptions.

## Two layers

The game is a 68000 program whose RAM structures hold 32-bit pointers into ROM and RAM. Both
layers keep that address space: the user's ROM image at `0x000000` and 64 KiB of big-endian work
RAM at `0xFF0000`, reached through typed, named views generated from the decompilation.

1. **Readable game logic.** Every function of the decompilation is translated to Rust in
   `tje-game`, traceable to its decomp source and proven equivalent to the original, function by
   function, in check mode (identical RAM, registers and ordered device accesses on real game
   states). This is the documented, maintainable game and the base for the extras; it does not
   by itself carry the original's instruction timing.
2. **Cycle-exact engine.** The shipped faithful game runs code compiled from the user's own ROM
   at first launch: each original instruction becomes a native continuation with its exact
   semantics and its exact 68000 cycle cost, and interrupts are accepted at exactly the original
   instruction boundaries. Desktop builds compile to native code (Cranelift) and cache it; the
   browser build emits a WebAssembly module and caches it in browser storage. Nothing generated
   from the ROM is ever committed or distributed. There is no 68000 interpreter in the shipped
   game.

Both layers drive the same device layer without a 68000: the VDP, YM2612 and SN76489 models,
and the Z80 running the original sound driver from the user's ROM. Devices advance by exact
master-clock amounts, including refresh, VDP FIFO and DMA stalls, Z80 bus waits and YM busy
time.

**Hybrid host (development).** jgenesis runs the original ROM; Rust translations replace
original functions by address for check mode, and the reference emulator is the oracle for
both layers.

**Video.** The VDP model renders a pixel-exact 320×224 frame in software, including the per-line
changes the HBlank handler makes for split-screen. wgpu (Vulkan, Metal, DirectX 12, WebGPU or
WebGL 2) presents it with scaling and filters.

**Frontends.** Native: winit, wgpu, cpal and gilrs. Web: wasm-bindgen, wgpu, Web Audio
(AudioWorklet) and the Gamepad API; the ROM is chosen with a file picker and stays in the
browser's storage.

### Continuation boundary

`tje-engine-core` owns the CPU/bus/IR ABI; `tje-engine-runtime` owns instruction
admission, reset and exception/interrupt entry. `tje-native-platform` advances
the original devices and publishes deferred CPU writes. The borrowed playback
interface matches the host's frame/audio/input shape. The detailed ABI, fixed
10+44-cycle reference interrupt sequence, backend cache boundaries and proof
obligations are specified once in [ENGINE.md](ENGINE.md).

## Verification

1. **Reference.** The original ROM in jgenesis, headless, with scripted input.
2. **Readable layer, per function.** Check mode compares every translated function with the
   original on live game states, in both revisions.
3. **Engine, per instruction.** The compiled continuations match the reference instruction by
   instruction: registers, flags, memory and bus accesses, and cycle charges.
4. **Whole game.** Recorded sessions covering every subsystem replay through the shipped engine
   with identical RAM, VDP memories, frames, audio samples and sound-chip streams every frame:
   title and menus, 1P and 2P split-screen, every present, Earthlings, elevators, the map, ship
   pieces, Jam Out and the ending, in both revisions.
5. **CI** builds every target and runs the ROM-free checks; the suites that need the ROM run
   locally.

### Engine replay acceptance

`tje-engine-verify` is a development-only harness, never a shipped dependency.
It compares every frame's work RAM, VRAM/CRAM/VSRAM, complete VDP state (including
registers, latches, FIFO and DMA), Z80 RAM, pixels, exact stereo f64 sample bits and
counts, and ordered timestamped YM2612/PSG dispatches. Receipts list actual executed
original PCs and function coverage; unreached functions remain explicit. A mismatch
is replayed to the first unequal original-instruction boundary, with the frame,
scanline, master-clock position, original PC and committed function name.

`replay --instruction-proof` additionally compares all 25 architectural CPU words,
the base cycle charge, and every ordered 68000 bus read/write/IRQ acknowledge after
each scheduler step. Semantic, prefetch and discarded reads remain distinct.
Initial RAM equality and an all-origin actual RAM-commit journal (including Z80
banked writes) prove complete work-RAM equality at each boundary without scanning
64 KiB per instruction. Reports give separate step/instruction/access/RAM-commit
denominators. Frame/device/output comparisons remain mandatory. `accept` enables
this proof unconditionally for reference sanity, IR, native and browser Wasm.

```sh
# One session using the test-only IR oracle; reference selects two independent emulators.
cargo run --release -p tje-engine-verify -- replay \
  --rom roms/rev02.bin --session sessions/rev02-one-player.tje.json \
  --backend ir --instruction-proof --report scratch/engine-ir.json

# The release matrix: every *.tje.json in sessions, both ROM revisions,
# independent reference sanity, IR, native compiled code, and actual browser Wasm.
# WASM_PROOF is an executable browser proof wrapper (not an interpreter).
cargo run --release -p tje-engine-verify --features native -- accept \
  --rom-rev00 roms/rev00.bin --rom-rev02 roms/rev02.bin --sessions sessions \
  --cache-dir scratch/engine-cache --wasm-command "$WASM_PROOF" \
  --report scratch/engine-acceptance.json
```

`reference-sanity` takes the same ROM/session/report arguments without the cache or
Wasm arguments and is deliberately **not** a release-acceptance receipt. `replay
--backend native --cache-dir DIR` also reports native code size, cache status and
startup/compilation timings. The browser wrapper receives `--rom`, `--session` and
`--report`, writes a `tje_engine_verify::ReplayReport` with full instruction-proof
counters for the real Wasm backend,
and exits unsuccessfully on a mismatch. For a Node wrapper, use `--wasm-command
node --wasm-arg path/to/wrapper.mjs`. An unavailable backend or missing/partial
browser receipt fails acceptance; it is never replaced by the reference or IR
oracle. Reports and caches stay private in ignored scratch/user storage.

## Phases

0. **Foundations:** workspace, toolchains, CI, the public audit, ROM import, address-space views.
1. **Reference and hybrid host:** jgenesis reference, hooks, check mode, sessions.
2. **Platform:** frontends on the hybrid host.
3. **Translation waves** of the readable layer, subsystem by subsystem, reviewed.
4. **Cycle-exact engine:** instruction decoder and timing model, continuation runtime on the
   device layer, native and WebAssembly backends, per-instruction and whole-session proof.
5. **Release:** Windows, macOS and Linux builds and a hosted web build running the engine.

Phases 3 and 4 run in parallel.
