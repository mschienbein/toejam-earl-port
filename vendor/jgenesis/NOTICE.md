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

Local changes are marked `TJE hybrid` or `TJE standalone cutover` in code:

- `backend/genesis-core/Cargo.toml`: remove CD, 32X, SH-2 and addon debugger-test dependencies.
- `backend/genesis-core/src/api.rs`: standalone-only construction/reset/render/tick/save API, remove addon/channel-debugger paths, and add narrow CPU/bus borrow accessors for instruction-boundary interception.
- `backend/genesis-core/src/lib.rs`: re-export the bus type and opt-in hardware-write event type.
- `backend/genesis-core/src/bus.rs`: remove addon fields, memory-map dispatch, clocks and debugger coupling; expose pending-write flush; record VDP/YM2612/PSG writes at chip dispatch, including Z80 writes.
- `backend/genesis-core/src/audio.rs`: remove absent CD PCM/CD-DA/32X PWM models and resamplers; retain Genesis filters/resamplers and the upstream standalone sample addition order (including zero contributions for IEEE-754 bit identity).
- `backend/genesis-components/src/vdp.rs`: immutable VRAM/CRAM/VSRAM/register accessors and public typed register definitions.
- `backend/genesis-components/src/vdp/registers.rs`: derive equality for allocation-free comparisons of immutable register views.
- `cpu/m68000-emu/src/core.rs`: readiness query prevents hooks from running before IRQ handling, stopped CPUs or bus stalls.

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
