# Native controls and settings

Run `cargo run --locked --release -p tje-native --bin toejam-earl`. Bring an original ToeJam & Earl
REV 02 or REV 00 cartridge: drop the file on the window, enter its full path in
the first-launch screen, or pass `--rom PATH`. Supported inputs are `.bin`, `.md`,
`.gen`, `.smd` and a ZIP containing one supported cartridge. Identification checks
the entire normalized image; a wrong game, modified image or bad dump is rejected
with an in-window explanation, without replacing the saved cartridge. The cartridge
is supplied **once**. A responsive first-launch progress screen compiles its original
instructions to native continuations on this computer and stores a private cache.
Later launches find the remembered cartridge and load that cache automatically,
without opening a file prompt. The shipped game has no 68000 interpreter, hybrid
host or original-call fallback.

Settings → Cartridge / help can replace the remembered cartridge or **Forget saved
cartridge and compiled cache** (confirmation required). `--forget-rom` does the same
at launch while keeping controls/settings. A missing/corrupt remembered image
returns to the friendly import screen. Cache corruption/staleness is rebuilt from
the verified cartridge; identity includes ROM, engine/lowering version, actual
clock divider, program and native target. Nothing compiled from a ROM is shipped.

On Windows both the remembered ROM/session data and compiled cache default to
**LocalAppData**, not Roaming AppData; only the settings directory roams. Leave
these defaults enabled or choose a **local-disk** `--data-dir`. Explicit UNC/network
overrides can stall during atomic cache publication (observed on the WSL UNC
share), even after compilation and payload flushing completed. See the
[release notes](../../docs/RELEASING.md#windows-network-path-override-limitation)
for the measured boundary. No interpreter fallback or non-atomic cache bypass is
used to hide a slow network filesystem.

**F1 opens Settings** and pauses the game. Escape cancels an active binding capture,
then closes Settings. The pause screen also has a Settings button. Settings apply
immediately and are saved; there is no Apply/restart step.

- **Video / audio:** independent **Pixel shape** and **Scale mode**, sharp-bilinear/
  nearest/linear filtering, vsync, fullscreen, logical-pixel window scale, volume,
  audio output and pause-on-focus-loss. Window dimensions and UI respect OS
  high-DPI scaling; integer game scaling uses whole physical source-line multiples.
- **Controls:** select either player, enable/disable the controller, choose a pad
  slot or Keyboard only, and set a stick deadzone. Click a keyboard/gamepad binding
  and press the new control; Clear removes that source. Pad captures accept mapped
  buttons and both sticks. A keyboard conflict across either player or a reserved
  hotkey is rejected; pad conflicts within that player's actions are rejected.
  Clear the old assignment first. A new pad assignment replaces the old sources
  for that action, including a default D-pad/stick pair.
- **Gamepad slots:** hotplug is automatic. Device model UUIDs/names and slot order
  are saved. Disconnected slots stay reserved; reconnecting a matching device
  restores its slot. Selecting the other player's slot swaps assignments rather
  than driving both players. Identical controller models without a unique serial
  cannot be distinguished after both are unplugged; assign the desired slots in
  Settings in that case.
- **Audio:** System default follows default-device changes. Selecting a named
  output waits for that device if disconnected. Failed/stalled streams, removed
  devices and format changes are retried automatically without resetting the
  game or ending a recording. No-device startup remains playable and displays
  the recovery status. Device-rate changes update the backend resampler and flush
  stale queued sound.

## Pixel shape and scaling

The default is **Native (NTSC) + Integer + Sharp bilinear**. Pixel shape can also
be **Square pixels**, and either shape supports **Fit (non-integer)**. Native uses
the Genesis NTSC pixel aspect ratio: H32 (256-wide) is 8/7 and H40 (320-wide) is
32/35; doubled-height interlace buffers multiply that ratio by two. Square uses
1:1 pixels, including doubled-height buffers. There is no widescreen/extended view.

Integer chooses the largest whole vertical source-line factor that fits **both**
the corrected width and source height. The width is that factor × source width ×
pixel aspect, so horizontal stretching can be fractional without sacrificing crisp
integer-height scanlines. If even 1× cannot fit, it falls back to Fit, preserving
the chosen shape without cropping. Fit fills the available area at the same aspect.
Viewport extents stay fractional; centered offsets are rounded down. Desktop and
browser tests consume the shared [viewport vectors](tests/data/viewport-vectors.json)
(H32/H40, 224/240 lines, doubled heights, exact/odd/small/large targets).

Sharp bilinear keeps pixel interiors sharp and blends fractional boundaries without
nearest-neighbor's uneven horizontal pixel widths. Nearest and Linear remain
selectable. F6 toggles Integer/Fit **without changing pixel shape**; F7 cycles Sharp
bilinear → Nearest → Linear. CLI overrides are `--pixel-shape native|square`,
`--scale integer|fit` and `--filter sharp|nearest|linear`.

Keyboard defaults: P1 arrows, Z/X/C, Enter; P2 W/S/A/D, G/H/J, Space. A/B/C on pads
are west/south/east, with D-pad/left stick and Start. Hotkeys: Escape pause, F5
reset, F6 scale, F7 filter, F8 vsync, F9 recording, F11 fullscreen,
PageUp/PageDown volume.

Recording is input-only. F9 starts from a hard reset; F9/reset/import/exit saves
under the data directory's `sessions/`. Settings, focus pauses and manual pauses
add no game/recording frames. Recorded controller state is the same state sent to
the backend. `--replay PATH.tje.json` runs a recorded session through the same
compiled engine and exits at its end. Session revision must match the cartridge.
Documented session recipes are applied at their exact frame boundaries. Keyboard
and pad state cannot alter replayed input; settings/pause remain available. F5
restarts the replay from power-on. F9 recording is disabled during replay.

Data/configuration/cache use `directories::ProjectDirs`: Linux XDG data/config/cache,
Windows LocalAppData data/cache and Roaming AppData configuration, macOS Application
Support data/configuration and Library/Caches. All resolved directories are printed
at startup and shown in Settings. Native code lives in the cache's `engine-native/`
child and is never loaded from the working directory or next to an arbitrary ROM.
Existing
`config.toml` files in the old data directory migrate only after the new file was
saved successfully. Version-1 settings preserve controls; version-2 adds UI,
focus/audio and persistent pad-slot preferences. Version 3 separates pixel shape
from scale: an explicitly stored old Integer choice migrates to Square + Integer;
old Fit migrates to Native + Fit. An unset mode adopts Native + Integer. Explicit
Nearest/Linear choices are retained; an unset filter adopts Sharp bilinear.
Unsupported/invalid configuration
opens the settings screen with defaults and an error, leaving the original file
untouched until a deliberate settings change/default reset is saved.
`--data-dir PATH` isolates data/configuration and a `cache/` child for portable/test runs.

`--input-script` injects keyboard transitions through normal bindings, and
`--run-frames` counts console frames. `--run-ticks` additionally permits bounded
first-launch/settings tests without a cartridge. Captures and recorded game data
must stay private; do not publish game screenshots or cartridge-derived content.

Linux development requires ALSA/udev headers and pkg-config as documented in
`docs/TOOLCHAIN.md`; the overlay adds no GTK/desktop-portal requirement. WSLg uses
`env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET` on the documented XWayland route.
The workspace CI covers the native crate on Linux, Windows and macOS; browser
builds exclude this native-only crate. Local verification does not constitute a
macOS runtime test.

## Development proof and shipping boundary

`cargo test --locked -p tje-native --test release_graph` checks the shipping graph
including dev edges for forbidden CPU/hybrid/verifier packages and oracle features.
No frontend feature enables them. The separate `tje-native-proof` package is built
by workspace CI but is not a dependency of the game and is never packaged:

```sh
cargo run --locked --release -p tje-native-proof -- \
  --rom /private/cartridge.bin --session /private/recording.tje.json \
  --cache-dir /private/engine-proof-cache --report /private/proof.json
```

It drives the same production `NativeBackend` and `Replay` library as the windowed
game and compares every frame byte-for-byte against the development reference:
CPU/clock position, complete work RAM, VDP memories/registers/full internal state,
Z80 RAM, full pixels, exact stereo f64 sample bits/counts and ordered timestamped
sound writes. The report contains metadata only. An opt-in real-ROM integration
test additionally proves first-import/remembered-second-launch native cache reuse:

```sh
TJE_REV02_ROM=/private/cartridge.bin cargo test --locked --release -p tje-native \
  --test first_launch -- --ignored
```

Apple ARM64 CI explicitly builds both the shipping frontend and the separate proof.
The release artifact remains one native executable; its package never contains a
user cartridge, generated code cache, proof executable, or development oracle.

