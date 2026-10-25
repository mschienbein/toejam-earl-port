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

## Online two-player co-op (opt-in)

Netplay is **off on every launch**. Offline play neither opens a network socket
nor captures rollback states. Open **Settings → Online co-op** after importing a
cartridge; replay and recording must be stopped. Starting or disconnecting resets
to local power-on. The host is P1 and the joiner P2, but **both use their local P1
keyboard/gamepad bindings**.

For two native instances on one machine, leave the host bind address
`0.0.0.0:19791` and advertised address `127.0.0.1:19791`, choose **Host**, then copy
its connection code. The host dialog stays open for copying; close Settings (F1)
after sharing. Paste the code into the second instance and choose **Join** (it
uses an automatic local port). Across a LAN, advertise the host's LAN IP; across
the Internet, forward that UDP port and advertise the host's reachable public IP.
IPv6 addresses use `[address]:port`. Native UDP has no discovery, STUN, server,
account or relay. It is not encrypted: exchange its random capability code only
with a trusted peer. Unsupported NAT/firewall paths cannot connect.

In the browser, open **Online two-player co-op** below the controls. Host creates
an offer code; joiner pastes it and creates an answer; host pastes the answer and
applies it. The page uses encrypted WebRTC unreliable/unordered data channels
with manual, complete-ICE signaling. Free public STUN is an explicit unchecked
option; without it, no public discovery service is contacted. TURN is never used.
Browser and native codes are intentionally separate: direct native-to-browser
interoperability is not implemented.

Both peers compare normalized ROM SHA-1, engine version, faithful simulation
profile, audio sample rate, delay/window and complete power-on state before
advancing. **No ROM, graphics, audio, compiled code or snapshots are transmitted.**
Inputs use 0–8 delay frames (default 2); last-input prediction rolls back at most
12 frames, then waits rather than skipping simulation. A cumulative ACK and
bounded input retransmission tolerate packet loss/reordering. Every 30 confirmed
frames, complete canonical machine-state hashes detect divergence and stop play.
The settings/status show rollback count, replayed frames, maximum depth and waits.

Save/load, rewind, fast-forward, reset/import and input recording are disabled
online. Manual pause/settings (and a hidden browser tab) stop local advancement
but transport keeps pumping; the other peer waits at the prediction bound.
Native focus-loss auto-pause is ignored online so two local windows can run:
losing focus releases held keys rather than pausing the peer. After resuming,
corrections catch up. Only each new presented frame's audio is queued: replayed
audio is discarded, not played twice. Predicted audio can have discontinuities;
this prototype does not promise artifact-free audio or arbitrary-latency play.
Playtest reports omit replayable input recordings during netplay because earlier
predictions can be revised.

`tje-netplay` contains ROM-free delayed/lossy/reordered two-peer and real UDP
tests. The native library also has ignored **production-engine** tests, comparing
canonical final bytes (CPU, all hardware, framebuffer and exact stereo sample
bits) to the no-latency run of the existing two-player recording:

```sh
TJE_REV02_ROM=/private/rev02.bin cargo test --locked --release -p tje-native netplay::tests -- --ignored --nocapture
```

`TJE_NETPLAY_FRAMES` selects 120–12000 frames (default 3600, including both players' movement);
`TJE_NETPLAY_CACHE` selects a private compiled-code cache. For the UDP test,
`TJE_NETPLAY_TEST_IP` may select a local LAN interface instead of loopback.

After building `web/pkg`, `node web/verify-netplay.mjs /private/rev02.bin`
exercises real direct WebRTC, the production Wasm engine, simulated delivery
delay/loss, and the two-context host/join UI. Set `PLAYWRIGHT_MODULE` /
`PLAYWRIGHT_BROWSERS_PATH` to an existing Playwright/Chromium installation and
`TJE_WEB_ARTIFACT_DIR` to a private directory under `scratch/` for the hash-only
JSON receipt. No STUN or public signaling service is used by this harness.
Native copy/paste uses the OS clipboard, not egui's per-window fallback; the
ignored `desktop_clipboard_roundtrip_for_connection_codes` test exercises two
independent clipboard contexts on a running desktop.

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

**Display shader** is an independent, default-**Off** opt-in GPU preset, shared
with the browser. Off leaves the existing pipeline's output byte-for-byte
unchanged, including its Sharp/Nearest/Linear filter. **Smooth (soft bilinear)**
widens sharp-bilinear seams to two physical pixels (linear at 2× or below), using
a linear sampler without overwriting the saved filter. **CRT (flat)** adds
source-line scanlines and a physical-pixel RGB aperture mask; **CRT (curved)**
also bows the image and adds a mild vignette/black corners. CRT uses the selected
filter. Scanlines fade out below 2× to avoid undersampling. All presets retain
pixel-shape and scale choices, support dynamic framebuffer sizes, affect only
game video (not the overlay), and never change engine timing, input or audio.
Native CLI: `--display-shader off|smooth|crt|crt-curved`. Settings files predating
this field load Off; turning it back Off restores the original display.

Both frontends append the exact same newly authored WGSL effect functions from
`tje-display`. Enabled effects sample encoded sRGB pixels and convert for the
target format; legacy Off keeps each frontend's original color/interpolation
path. This is a single fullscreen-triangle GPU pass, not CPU image processing
or an intermediate-framebuffer chain. CRT approximates a display; it is not a
model of a particular television.

Pixel/screenshot and presentation-only performance acceptance (Vulkan adapter
required; software Vulkan is supported):

```sh
TJE_SHADER_ARTIFACT_DIR=/private/scratch/shaders \
  cargo test --locked -p tje-display --test pixels -- --ignored --nocapture
```

The procedural pattern contains no game assets. The test compares Off against
independent pre-extras native/browser shader fixtures for all filters, target
encodings and representative exact/fractional/small/dynamic source sizes;
compares enabled native/browser pixels within one 8-bit step (two when comparing
sRGB versus encoded render targets, allowing hardware conversion/filter rounding);
writes before/after screenshots and measured GPU-completion wall times. It is
explicitly ignored in ordinary workspace tests because CI need not have a GPU.
Browser control/persistence/screenshots and a bounded running performance check:
`node web/verify-shaders.mjs /private/rev02.bin webgl` (repeat with `webgpu`).
Screenshot artifacts stay private under `scratch/`; never publish game images.

Keyboard defaults: P1 arrows, Z/X/C, Enter; P2 W/S/A/D, G/H/J, Space. A/B/C on pads
are west/south/east, with D-pad/left stick and Start. Hotkeys: Escape pause, F5
reset, F6 scale, F7 filter, F8 vsync, F9 recording, F11 fullscreen,
PageUp/PageDown volume.

### Optional save states, rewind and fast-forward

Settings → Extras contains three independent switches, all **off by default**.
Enable local save states for five slots per ROM: **F2** saves, **F3** loads and
**F4** selects the next slot; the overlay also has slot/save/load controls.
Files live in the data directory's `save-states/<ROM SHA-1>/slot-N.tje-state`.
Writes replace a slot only after its private temporary file is synced. Unknown
formats, corrupt states and another ROM are rejected without changing the game.
These files contain copyrighted runtime RAM and must never be shared or committed.

Enable rewind, then hold **Backspace** or use the overlay checkpoint button.
The ring samples every three frames, evicting oldest images within a **64 MiB
image-allocation budget** (plus bounded ring bookkeeping). Releasing rewind
continues a new branch; reset/import/load/disabling rewind discards old history.
Enable fast-forward, then hold **Tab**, or toggle 4x speed in the overlay.
Fast-forward executes every original frame but mutes output and flushes old
queued audio. Existing custom game/hotkey mappings take precedence over these
shortcuts; overlay controls remain available.

Load/rewind are unavailable during input recording or input replay. After a
load/rewind, automatic input-only playtest reports are intentionally omitted
until hard reset: subsequent inputs alone cannot reproduce the restored RAM.
Snapshots never go into reports. Output sample rate adapts to the current audio
device when loading a state made with a different rate; the core restore API
itself restores the saved resampler and all exact pending audio unchanged.

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

## Local crash and bug reports

Panics and fatal frontend errors automatically save a self-contained
`reports/report-….tje-report.json` bundle in the OS-local **data** directory above.
On the next launch a report offer can show that folder and open a prefilled issue
on `mschienbein/toejam-earl-port`. **Nothing is uploaded, and GitHub is not contacted
until you click the issue button.** Review the file, then attach it yourself
(ZIP the JSON first if GitHub requires it). Never attach your ROM or compiled cache.
Settings has **Report a bug…**, with a free-text note, for non-crashing problems.

Bundles collect the build/commit/target, OS, verified ROM revision and SHA-1
**only**, settings, error and available backtrace, and the last completed
frame/PC/master-clock/player-position summary. They contain neither ROM bytes,
generated code, audio, nor a machine-memory snapshot. A last-game-frame PNG is
included **only after you opt in** in the report dialog; it contains copyrighted
game graphics. That consent is remembered for future crashes and can be turned
off in the same dialog. Notes, backtraces and settings may contain personal details;
review them before sharing.

An automatic controller-only ring records every played console frame, independently
of F9. It holds **216,000 frames (864,000 raw input bytes; about one hour at 60 Hz)**,
restarts on hard reset/import, and never advances while paused. Whenever it fits,
the embedded `recording.session` is the existing tje-session format, continuous
from power-on. If a frame fails mid-step, `recording.in_flight` identifies its input
and that attempted frame is the session's last input. After overflow the report
contains an explicitly non-replayable `input_tail`, not a misleading power-on
session; the developer replay rejects it. Screenshot and report serialization add
bounded overhead only when opted in / capturing a report. Saved reports are kept
until you remove them from the report directory.
When reporting a developer replay with an existing declared RAM recipe, its
already-applied setup writes are preserved too; no live RAM is read into a report.

Replay a downloaded complete native **or browser** report using your own matching
cartridge and the shipping native engine:

```sh
cargo run --locked -p tje-native --bin toejam-earl -- \
  --rom /private/rev02.bin --replay-report /private/report.tje-report.json
```

It exits at the recording's end and, for a completed-frame report, checks the
captured frame/PC/master-clock/player summary (`REPORT_REPLAY_MATCH`). An unfinished
frame is attempted to reproduce the fault instead. You can alternatively extract
`recording.session` as JSON and use the ordinary `--replay PATH.tje.json` or
`tje-session replay --rom … --session … --report …` developer runner.
This is a diagnostic summary check, not a claim of full-machine byte equality.

For actual capture testing **only debug builds** honor
`TJE_DEBUG_CRASH_FRAME=240`; the native panic occurs after recording that completed
frame. Browser debug builds expose a hidden `window.tje.debugCrash()` and optional
`?debug-crash-frame=240` trigger; release builds contain no such Wasm method.
The report schema/export boundary is transport-independent: a future approved
upload adapter can consume the same bundle after explicit consent, but there is
no server, account integration or automatic upload now.

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

