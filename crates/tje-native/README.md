# Native controls and settings

Run `cargo run -p tje-native --bin toejam-earl`. Bring an original ToeJam & Earl
REV 02 or REV 00 cartridge: drop the file on the window, enter its full path in
the first-launch screen, or pass `--rom PATH`. Supported inputs are `.bin`, `.md`,
`.gen`, `.smd` and a ZIP containing one supported cartridge. Identification checks
the entire normalized image; a wrong game, modified image or bad dump is rejected
with an in-window explanation, without replacing the saved cartridge. After a
successful import later launches start directly.

**F1 opens Settings** and pauses the game. Escape cancels an active binding capture,
then closes Settings. The pause screen also has a Settings button. Settings apply
immediately and are saved; there is no Apply/restart step.

- **Video / audio:** integer-pixel or NTSC aspect-correct fit, nearest/linear
  filtering, vsync, fullscreen, logical-pixel window scale, volume, audio output
  and pause-on-focus-loss. Window dimensions and UI respect OS high-DPI scaling;
  integer game scaling remains an integer number of physical pixels.
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

Keyboard defaults: P1 arrows, Z/X/C, Enter; P2 W/S/A/D, G/H/J, Space. A/B/C on pads
are west/south/east, with D-pad/left stick and Start. Hotkeys: Escape pause, F5
reset, F6 scale, F7 filter, F8 vsync, F9 recording, F11 fullscreen,
PageUp/PageDown volume.

Recording is input-only. F9 starts from a hard reset; F9/reset/import/exit saves
under the data directory's `sessions/`. Settings, focus pauses and manual pauses
add no game/recording frames. Recorded controller state is the same state sent to
the backend. Replay remains the `tje-session` CLI's responsibility.

Data and configuration use `directories::ProjectDirs`: Linux XDG data/config,
Windows LocalAppData data/Roaming AppData configuration, macOS Application Support.
Both resolved directories are printed at startup and shown in Settings. Existing
`config.toml` files in the old data directory migrate only after the new file was
saved successfully. Version-1 settings preserve controls; version-2 adds UI,
focus/audio and persistent pad-slot preferences. Unsupported/invalid configuration
opens the settings screen with defaults and an error, leaving the original file
untouched until a deliberate settings change/default reset is saved.
`--data-dir PATH` isolates both directories for portable/test runs.

`--input-script` injects keyboard transitions through normal bindings, and
`--run-frames` counts emulated frames. `--run-ticks` additionally permits bounded
first-launch/settings tests without a cartridge. Captures and recorded game data
must stay private; do not publish game screenshots or cartridge-derived content.

Linux development requires ALSA/udev headers and pkg-config as documented in
`docs/TOOLCHAIN.md`; the overlay adds no GTK/desktop-portal requirement. WSLg uses
`env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET` on the documented XWayland route.
The workspace CI covers the native crate on Linux, Windows and macOS; browser
builds exclude this native-only crate. Local verification does not constitute a
macOS runtime test.
