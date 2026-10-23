# ToeJam & Earl: native port

A faithful native port of *ToeJam & Earl* (Sega Genesis / Mega Drive, 1991) in Rust, for
Windows, macOS, Linux and the browser (WebAssembly). It is built from the
[ToeJam & Earl matching decompilation](https://github.com/mschienbein/toejam-earl-decomp), which
rebuilds both retail ROMs byte for byte from readable C and assembly.

**Status: just started, not playable yet.** The plan is in [docs/PLAN.md](docs/PLAN.md) and
progress in [docs/STATUS.md](docs/STATUS.md).

You need your own copy of the game (REV 02 "Rev A" or REV 00). Nothing from the game is in this
repository: the port reads graphics, sound and level data from your ROM.

## Goals

1. **Faithful first.** The port plays exactly like the cartridge: the same game logic, frames
   and sound, original bugs included. This is checked against the original ROM running in a
   reference emulator, not assumed.
2. **Then the extras:** widescreen, online co-op, quality-of-life changes, saves, enhanced
   graphics, new content and game modes, and modding.

## Reproducible controller sessions

See [docs/SESSIONS.md](docs/SESSIONS.md) for the input-only format, native F9
recorder, reference/check/replacement replay, differential comparison and the
648-entry coverage report. The committed `sessions/` recordings contain only
controller inputs and metadata; replay requires your own matching cartridge.

## License

GPL-3.0, see [LICENSE](LICENSE).
