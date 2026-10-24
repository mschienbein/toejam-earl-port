# ToeJam & Earl: native port

A faithful native port of *ToeJam & Earl* (Sega Genesis / Mega Drive, 1991) in Rust, for
Windows, macOS, Linux and the browser (WebAssembly). It is built from the
[ToeJam & Earl matching decompilation](https://github.com/mschienbein/toejam-earl-decomp), which
rebuilds both retail ROMs byte for byte from readable C and assembly.

**Status: work in progress, not a finished release.** Both shipped frontends run the
cycle-exact engine compiled at first launch from your own cartridge: native code on
desktop and generated WebAssembly in the browser. No 68000 interpreter ships.
The plan is in [docs/PLAN.md](docs/PLAN.md) and progress in [docs/STATUS.md](docs/STATUS.md).
See also the [function catalogue](docs/FUNCTIONS.md) and [revision differences](docs/REVISIONS.md).

You need your own copy of the game (REV 02 "Rev A" or REV 00). Nothing from the game is in this
repository: the port reads graphics, sound and level data from your ROM.

## Goals

1. **Faithful first.** The port plays exactly like the cartridge: the same game logic, frames
   and sound, original bugs included. This is checked against the original ROM running in a
   reference emulator, not assumed.
2. **Then the extras:** widescreen, online co-op, quality-of-life changes, saves, enhanced
   graphics, new content and game modes, and modding.

## Run

Supply your own REV 02 / REV 00 cartridge; no download includes the game.
Native release workflow artifacts contain a Windows ZIP, a universal macOS
`.app` ZIP, or a Linux tarball/AppImage. Extract the package and launch
`toejam-earl.exe`, `ToeJam & Earl.app`, `./toejam-earl`, or the AppImage, then drop
your cartridge onto its window. The terminal alternative is:

```sh
./toejam-earl --rom /path/to/cartridge.bin
# From a source checkout with the pinned toolchain and native dependencies:
cargo run --locked --release -p tje-native -- --rom /path/to/cartridge.bin
```

Player 1 uses arrows, Z/X/C and Enter; player 2 uses W/S/A/D, G/H/J and Space.
Esc pauses, F5 resets and F11 toggles fullscreen. Run `--help` for more controls
and options. The imported ROM and settings stay in the OS user-data directory
printed at startup, not in the extracted package.
Open **Settings (F1)** for live remapping and options; see the [native controls guide](crates/tje-native/README.md).

For the browser build, open the current preview at
<https://mschienbein.github.io/toejam-earl-port/> (it tracks `main` and is a work in progress),
choose the ROM locally, and click **Play / enable audio**. Nothing is
uploaded. A downloaded web ZIP must be served on HTTPS or localhost; opening
`index.html` directly will not work. Its on-page controls list the browser-specific
bindings.

The browser's **Pixel shape** and **Scaling** controls are independent: choose
**Native (NTSC)** or **Square pixels**, each with **Integer** or **Fit (non-integer)**.
New settings use Native plus the largest whole vertical line multiple that fits;
the horizontal stretch keeps the original NTSC shape. **Sharp bilinear** is the
default filter, with nearest and linear still available. Previously saved browser
display choices retain their square-pixel shape, scale mode and explicit filter.
Press **F** or use the fullscreen button to toggle fullscreen without changing
these choices.

See [docs/RELEASING.md](docs/RELEASING.md) for runtime dependencies, checksum
verification, unsigned-package warnings, local native/web builds and the
maintainer release procedure. Release workflows run only for `v*` tags or an
allowed manual dispatch; configuring them does not publish a release.

## Crashes and bug reports

If the game crashes, both frontends save a local report: build, platform, ROM revision and
hash (never ROM contents), settings, the error, and the controller inputs since power-on, so
the crash can be replayed exactly. A screenshot is added only with your consent. Use **Report a
bug** to capture the same bundle on demand. On the next launch you're offered a prefilled
GitHub issue; nothing is sent unless you submit it. See the
[native controls guide](crates/tje-native/README.md) for details.

## Reproducible controller sessions

See [docs/SESSIONS.md](docs/SESSIONS.md) for the input-only format, native F9
recorder, reference/check/replacement replay, differential comparison and the
648-entry coverage report. The committed `sessions/` recordings contain only
controller inputs and metadata; replay requires your own matching cartridge.

## License

GPL-3.0, see [LICENSE](LICENSE).
