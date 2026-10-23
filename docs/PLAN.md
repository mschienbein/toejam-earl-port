# Plan: the faithful port

## Decisions (user, 2026-10-23)

- Rust; WebAssembly for the browser; GPU rendering where it makes sense.
- A new repository that links to the decompilation. GPL-3.0, so components of
  [jgenesis](https://github.com/jsgroth/jgenesis) (GPL-3.0) can be used.
- Faithful first. The extras (widescreen, online co-op, quality of life, saves, enhanced
  graphics, new content and modes, modding) come after the faithful port works.

## What "faithful" means

For the same controller input, the port goes through the same game states as the cartridge:
identical work RAM after every logic tick, identical frames and identical sound-chip register
writes. Two console behaviours depend on CPU speed rather than on game logic, and are handled
explicitly:

- **Idle random draws.** While it waits for the next frame, the main loop keeps calling
  `GetRandomNumber` (decomp `FurtherInitialisation`), so the random stream depends on how much
  time the frame's work left over. Tests replay the exact per-frame counts recorded from the
  original; play mode uses a fixed, documented policy.
- **Lag frames.** When a frame's work overruns, the console's VBlank handler runs in the middle
  of it. The port never overruns, so tests compare at logic-tick boundaries and list the lag
  frames they skip.

## Architecture: hybrid first, then native

The game is a 68000 program whose RAM structures hold 32-bit pointers into ROM and RAM. The port
keeps that address space: the user's ROM image at `0x000000` and 64 KiB of big-endian work RAM at
`0xFF0000`, reached through typed, named views generated from the decompilation's RAM map and
headers. Original bugs that read past an array or reuse a field, pointer arithmetic, and ROM
pointers stored in RAM then behave exactly as on the console, and the port's RAM can be compared
byte for byte with the console's.

1. **Hybrid.** jgenesis runs the original ROM. Each Rust translation replaces its original
   function by address: when the 68000 reaches the function, the Rust version runs on the same
   RAM using the original calling convention (MCC68K passes every argument in a 32-bit stack
   slot and returns in D0), and it can call original functions that are not translated yet. The
   game stays playable at every step, and a check mode runs both versions on copies of the state
   and reports any difference in RAM, VDP writes or sound writes.
2. **Native.** Once every function is translated, the 68000 core leaves the shipped build. The
   Rust game logic drives the hardware it programs through the same port writes: the VDP, the
   YM2612 and SN76489, and the Z80, which keeps running the original sound driver from the
   user's ROM so sample playback stays exact.

**Video.** The VDP model renders a pixel-exact 320×224 frame in software, including the per-line
changes the HBlank handler makes for split-screen. wgpu (Vulkan, Metal, DirectX 12, WebGPU or
WebGL 2) presents it with scaling and filters. Drawing the VDP layers on the GPU waits for
widescreen and enhanced graphics, where it pays off.

**Frontends.** Native: winit, wgpu, cpal and gilrs. Web: wasm-bindgen, wgpu, Web Audio
(AudioWorklet) and the Gamepad API; the ROM is chosen with a file picker and stays in the
browser's storage.

## Verification

1. **Reference.** The original ROM in jgenesis, headless: scripted input and per-tick snapshots
   of RAM and VDP state, sound-chip register writes, idle random-draw counts and lag frames.
2. **Per function.** Check mode compares every translated function with the original, on live
   game states and on states sampled from recorded sessions.
3. **Whole game.** Recorded sessions covering every subsystem replay with identical RAM per logic
   tick, identical frames and identical sound writes: title and menus, 1P and 2P split-screen,
   every present, Earthlings, elevators, the map, ship pieces, Jam Out and the ending, in both
   revisions.
4. **CI** builds every target and runs the ROM-free checks; the suites that need the ROM run
   locally.

## Phases

0. **Foundations:** workspace, native and wasm toolchains, CI, the public audit (no ROM-derived
   bytes), ROM import and identification (REV 02 and REV 00), and address-space views generated
   from the decompilation.
1. **Reference and hybrid host:** jgenesis as headless reference, input recording and replay,
   function hooks with the MCC68K calling convention, and check mode.
2. **Platform:** the hybrid runs and plays on the native and web frontends.
3. **Translation waves**, subsystem by subsystem with leaf functions first; each wave verified
   and independently reviewed.
4. **Native:** the 68000 core leaves the build, and the full replay suite passes.
5. **Release:** Windows, macOS and Linux builds and a hosted web build.

## Open questions

- Which jgenesis interfaces the hybrid host needs (instruction-level hooks, nested calls, wasm
  builds), and whether to depend on a pinned revision or a vendored copy.
- The play-mode policy for idle random draws.
- How often the original lags, measured from recorded sessions.
