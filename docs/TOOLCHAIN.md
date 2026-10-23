# Toolchains and platform smoke tests

Observed on 2026-10-23. This is a toolchain feasibility record, not a claim that a
finished game frontend exists. The smoke application, its lockfile, downloaded
packages, browser, logs and screenshots remain **untracked in
`scratch/Toolchains/`**. No ROM or extracted game data was used.

All command examples below were exercised on this machine. Paths are abbreviated
with `TC` = the absolute path to `scratch/Toolchains/` and `LOCAL` = `$TC/prefix`.
The smoke-specific commands need the investigation's scratch files; those files
are intentionally not distributed with a fresh clone. Package names and tool
requirements also apply when building the eventual frontend.

## Observed results

| Target | Observed result |
| --- | --- |
| Linux / WSLg XWayland | Vulkan and OpenGL both presented the clear colour and completed the one-second tone; both selected Mesa llvmpipe, a **CPU** adapter. |
| Windows GNU, built in WSL | DX12 and Vulkan both ran through Windows interop on the NVIDIA GeForce RTX 4090; WASAPI completed 48,000 tone frames. No Windows Rust installation or Visual Studio installation was needed. |
| Browser wasm32 | WebGPU and WebGL 2 both presented a 640×448 canvas for at least 120 frames in headless Chromium, initialized gamepad enumeration, and completed a user-gesture-started one-second Web Audio tone. Screenshots show the rendered canvas. |
| jgenesis `genesis-core` wasm32 | Its complete dependency graph checked successfully with `getrandom/wasm_js` enabled by the host. Measured wall time: **17.10 s**, with the cache qualifications below. |
| macOS | Not executed: no macOS host or Apple SDK was available. CI requirements, explicitly not a verified smoke result, are recorded below. |

The machine was Ubuntu 26.04.1 under WSL2, with WSLg and Windows
10.0.26300.9711. Native GCC was 15.2.0 and Clang was 21.1.8. Rust was
**1.99.1**, including the `wasm32-unknown-unknown` standard library. The smoke
application pinned these versions:

| Component | Version / configuration |
| --- | --- |
| winit | 0.30.13, default features including X11, Wayland and Wayland dynamic loading |
| wgpu | 30.0.1, default features plus `webgl` |
| cpal | 0.16.0, native default backend; no JACK or ASIO feature |
| gilrs | 0.11.2, default features; native enumeration and its browser Gamepad API backend |
| wasm-bindgen / CLI | 0.2.129 on both sides |
| Node / Playwright / Chromium | 24.22.1 / 1.56.1 / 141.0.7390.37, Playwright build 1194 |

No controller was attached. Successful enumeration is **not** evidence of tested
controller input, hotplug or rumble. The clear-colour test is not a cross-backend
colour-management or game-fidelity test.

## Linux: packages for ordinary contributors

The native build needs a C compiler/linker, `pkg-config`, ALSA development files
and udev development files. On Debian/Ubuntu the corresponding packages are
`build-essential`, `pkg-config`, `libasound2-dev` and `libudev-dev`. The headers
actually provided locally were `alsa/asoundlib.h` and `libudev.h`; the link-time
libraries were `libasound.so` and `libudev.so`.

For a desktop, the window/graphics runtime also needs X11 or Wayland, xkbcommon,
and an installed GPU driver. This machine already had `libx11-6`,
`libwayland-client0`, `libxkbcommon0`, `libvulkan1`, `mesa-vulkan-drivers`,
`libegl1` and `libgl1-mesa-dri`. XWayland additionally needed
`libxkbcommon-x11-0` and `libxcb-xkb1`. Default winit dynamically loads the window
libraries; the smoke built without their development packages.

A conventional package set, including optional window-system development
headers, was resolved with this **simulation**, not installed system-wide:

```sh
apt-get --simulate install --no-install-recommends build-essential pkg-config libasound2-dev libudev-dev libx11-dev libxkbcommon-dev libwayland-dev libvulkan1 mesa-vulkan-drivers
```

An ordinary contributor can install those packages through their administrator's
normal package-management procedure. No untested privileged installation command
is presented here. For WSLg audio, the ALSA PulseAudio plugin and its runtime
libraries are also necessary; an ALSA header package alone does not make a tone
play. The additional Clang already present here was sufficient for the upstream
wasm check's C dependency (`zstd-sys`); a pure-Rust standard-library target alone
is not the complete environment for that dependency graph.

## Linux: the tested no-sudo route

Downloads were made in the scratch directory and unpacked into `LOCAL`, using
`dpkg-deb -x` with each downloaded `.deb` and that prefix. Nothing was installed
into `/usr`, and no system package database was changed. These were the actual
download groups:

```sh
apt-get download libasound2-dev libudev-dev pkgconf pkgconf-bin libpkgconf7 libasound2t64 libudev1
apt-get download libasound2-data libasound2-plugins libpulse0 libasyncns0 libsndfile1 libflac14 libmp3lame0 libmpg123-0t64 libogg0 libopus0 libvorbis0a libvorbisenc2 libx11-xcb1 pulseaudio-utils
apt-get download libxkbcommon-x11-0 libxcb-xkb1
```

The first group provides build metadata and libraries; the second provides the
ALSA-to-WSLg audio path and the tools used to record it. Only the PulseAudio ALSA
plugin was loaded. Other plugins in `libasound2-plugins` were not tested, and their
unneeded JACK/FFmpeg dependencies were not installed. The versions fetched here
included ALSA 1.2.15.3, udev 259 and pkgconf 2.5.1. These packages must come from a
repository compatible with the host distribution, not an arbitrary newer libc.

The scratch environment script exercised these settings; `TC` was an absolute
path so compiler/linker invocations could change directory safely:

```sh
export LOCAL="$TC/prefix"
export PATH="$LOCAL/usr/bin:$TC/tools/bin:$PATH"
export LD_LIBRARY_PATH="$LOCAL/usr/lib/x86_64-linux-gnu:$LOCAL/usr/lib/x86_64-linux-gnu/pulseaudio:/usr/lib/wsl/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export PKG_CONFIG_SYSROOT_DIR="$LOCAL"
export PKG_CONFIG_LIBDIR="$LOCAL/usr/lib/x86_64-linux-gnu/pkgconfig"
export ALSA_CONFIG_PATH="$TC/alsa.conf"
export ALSA_PLUGIN_DIR="$LOCAL/usr/lib/x86_64-linux-gnu/alsa-lib"
export DISPLAY=:0
export WAYLAND_DISPLAY=wayland-0
export XDG_RUNTIME_DIR=/run/user/1000
export PULSE_SERVER=unix:/mnt/wslg/PulseServer
export CARGO_BUILD_JOBS=8
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_INCREMENTAL=0
```

`PKG_CONFIG_SYSROOT_DIR` relocates the package files' `/usr` prefixes. Merely
adding their directory to `PKG_CONFIG_PATH` would not relocate their include and
library paths. The private PulseAudio library directory is needed as well as the
main library directory. The local development-package symlinks and their runtime
library packages must both be present.

The tested `alsa.conf` contained:

```text
pcm.!default {
    type pulse
    server "unix:/mnt/wslg/PulseServer"
}
ctl.!default {
    type pulse
    server "unix:/mnt/wslg/PulseServer"
}
```

The following probes returned ALSA `1.2.15.3`, udev `259`, relocated flags, and the
WSLg `RDPSink` / `RDPSink.monitor` endpoints:

```sh
pkg-config --modversion alsa libudev
pkgconf --cflags --libs alsa libudev
pactl info
pactl list short sinks
pactl list short sources
```

The native build and successful runs were:

```sh
CARGO_TARGET_DIR="$TC/smoke-target" cargo +1.99.1 build --manifest-path "$TC/smoke/Cargo.toml"
env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET SMOKE_BACKEND=vulkan "$TC/smoke-target/debug/platform-smoke"
env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET SMOKE_BACKEND=gl "$TC/smoke-target/debug/platform-smoke"
```

Both runs exited 0, logged presentation and sustained rendering, and generated
44,100 tone frames at the default ALSA device's 44.1 kHz rate. A separate capture
was made using:

```sh
parec --device=RDPSink.monitor --format=float32le --rate=48000 --channels=2 --raw
```

Its stdout was saved in scratch, not printed to the terminal. Analysis of the
final capture measured a 0.999979 s non-silent span, 440.00005 Hz and peak amplitude
0.100006. This verifies a signal at the WSLg sink monitor, not just callback
execution; it does not measure physical speakers.

**WSLg limitation observed here:** leaving `WAYLAND_DISPLAY` set selected native
Wayland, which disconnected with `Broken pipe` after the first presentation on
both Vulkan and GL. The successful route above uses XWayland. Its Vulkan adapter
was `llvmpipe (LLVM 21.1.8, 256 bits)`. GL also used llvmpipe and emitted a DRI3
acceleration warning but completed successfully. Do not describe these Linux
results as hardware acceleration or as a successful native-Wayland test.

Evidence: `logs/linux-vulkan-x11.log`, `logs/linux-gl-x11.log`,
`logs/linux-vulkan-x11-audio.f32` and
`logs/linux-vulkan-x11-audio-analysis.json`. Failed Wayland runs are retained as
`logs/linux-vulkan.log` and `logs/linux-gl-wayland.log`.

## Windows: user-local GNU cross-build, no Visual Studio

The least invasive successful route was to use the existing Linux Rust install
and unpack a MinGW cross-toolchain into the same scratch prefix. There was no
Windows Rust, GCC or MSVC compiler on PATH. This route changed no Windows
installation or registry settings and needed neither `cargo-xwin` nor Wine.

The following commands were exercised, with package downloads in the scratch
`downloads` directory and each package subsequently unpacked using `dpkg-deb -x`:

```sh
apt-get download gcc-mingw-w64-x86-64-posix gcc-mingw-w64-x86-64-posix-runtime gcc-mingw-w64-base mingw-w64-x86-64-dev mingw-w64-common binutils-mingw-w64-x86-64 binutils-mingw-w64-base binutils-common
rustup target add --toolchain 1.99.1 x86_64-pc-windows-gnu
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="$LOCAL/usr/bin/x86_64-w64-mingw32-gcc-posix"
export CC_x86_64_pc_windows_gnu="$LOCAL/usr/bin/x86_64-w64-mingw32-gcc-posix"
export AR_x86_64_pc_windows_gnu="$LOCAL/usr/bin/x86_64-w64-mingw32-ar"
CARGO_TARGET_DIR="$TC/windows-target" cargo +1.99.1 build --manifest-path "$TC/smoke/Cargo.toml" --bin platform-smoke --target x86_64-pc-windows-gnu
WSLENV=SMOKE_BACKEND/w SMOKE_BACKEND=dx12 "$TC/windows-target/x86_64-pc-windows-gnu/debug/platform-smoke.exe"
WSLENV=SMOKE_BACKEND/w SMOKE_BACKEND=vulkan "$TC/windows-target/x86_64-pc-windows-gnu/debug/platform-smoke.exe"
```

The actual compiler was GCC 13-posix. Use its explicit `-posix` name: unpacking
packages does not run `update-alternatives` to create the generic GCC command.
The relocated compiler successfully linked both a small C probe and the Rust
application. Import-table inspection showed Windows system DLL imports, not a
required MinGW runtime DLL sidecar for this build.

`WSLENV` forwarding matters: a Linux `SMOKE_BACKEND` assignment alone did not
reach the Windows process. `/w` makes this variable available to the Windows
child. The session had no existing `WSLENV` entries; the one-command assignment
above does not alter the parent shell's settings.

Both explicit backend runs exited 0, selected the RTX 4090, presented frames and
completed a 48,000-frame tone through the actual default WASAPI output device.
Evidence: `logs/windows-dx12.log`, `logs/windows-vulkan.log` and
`logs/windows-pe-headers.log`.

The scratch library was an `rlib` for native builds. Asking the GNU linker to
also produce the large debug `cdylib` failed with `export ordinal too large`.
Only the web build requested `cdylib` explicitly. This is a smoke packaging
choice, not evidence that GNU executables cannot support the frontend stack.

**MSVC is not verified here.** Native Windows MSVC builds need the MSVC linker
and Windows SDK/native libraries; the normal Rust installer obtains those through
Visual Studio or its Build Tools. Installing only the Rust MSVC target is not
sufficient. Visual Studio Build Tools were deliberately not installed. The
[official Rust Windows installation documentation](https://rust-lang.github.io/rustup/installation/windows-msvc.html)
describes that separate route; no untested MSVC commands are listed here.

## Web: wasm-bindgen, WebGPU / WebGL 2 and Web Audio

The smoke used one Rust application with target-specific audio: cpal on native
platforms, and the browser's Web Audio API on wasm. A real button click resumed
an `AudioContext`, scheduled a 440 Hz oscillator for one second, and observed its
ended event. Browser gamepad enumeration was initialized through gilrs. This
smoke did **not** implement or test the game's streaming AudioWorklet pipeline.

The wasm library and version-matched, user-local CLI were built with:

```sh
cargo +1.99.1 install wasm-bindgen-cli --version 0.2.129 --locked --root "$TC/tools" --target-dir "$TC/bindgen-target"
CARGO_TARGET_DIR="$TC/web-target" cargo +1.99.1 rustc --manifest-path "$TC/smoke/Cargo.toml" --lib --crate-type cdylib --target wasm32-unknown-unknown
"$TC/tools/bin/wasm-bindgen" --version
"$TC/tools/bin/wasm-bindgen" --target web --out-dir "$TC/smoke/web/pkg" "$TC/web-target/wasm32-unknown-unknown/debug/platform_smoke.wasm"
```

The CLI reported `wasm-bindgen 0.2.129`, matching the Rust dependency. There was
no nightly compiler, atomics/shared-memory flag or standard-library rebuild in
this smoke. wgpu's `webgl` feature enabled the WebGL 2 path; the device requested
`Limits::downlevel_webgl2_defaults()` with the adapter's resolution limits.
Default WebGPU compute limits failed on WebGL 2 with
`max_compute_workgroups_per_dimension` requested as 65535 but supported as 0.

A user-local headless browser was installed with:

```sh
npm install --prefix "$TC/browser" --save-exact playwright@1.56.1
export PLAYWRIGHT_HOST_PLATFORM_OVERRIDE=ubuntu24.04-x64
export PLAYWRIGHT_BROWSERS_PATH="$TC/browser/browsers"
"$TC/browser/node_modules/.bin/playwright" install chromium
apt-get download libnspr4 libnss3
```

The last two packages were unpacked into `LOCAL` and found through the same
`LD_LIBRARY_PATH`; ALSA was already supplied by the Linux steps. Playwright
1.56.1 did not recognize Ubuntu 26.04 without the explicit Ubuntu 24.04 browser
build override. Its initial host-validation warning was not solved with sudo.

The scratch server served `.wasm` as `application/wasm`, on localhost:

```sh
node "$TC/smoke/serve.mjs"
node "$TC/smoke/browser-check.mjs" webgpu
node "$TC/smoke/browser-check.mjs" webgl
```

The server occupies one terminal; the two checks run in another environment with
the exports above. The page is at `http://127.0.0.1:8765/`; the harness chooses
`?backend=webgpu` or `?backend=webgl`. Localhost supplied a secure context. No
cross-origin-isolation headers were required by this single-threaded smoke.

The headless harness used these **tested browser arguments**:

```text
--enable-unsafe-webgpu
--enable-unsafe-swiftshader
--use-angle=swiftshader
--enable-features=Vulkan
--use-vulkan=swiftshader
--disable-vulkan-surface
```

These opt into software GPU implementations and are for the trusted local test,
not launch recommendations for browsing arbitrary sites. Without the last three
arguments, WebGPU adapter/device creation appeared to succeed, but Chromium
reported `A valid external Instance reference no longer exists` and the canvas
was white. A separate plain-JavaScript reproduction failed the same way. Merely
checking for `navigator.gpu`, or recording a submitted first frame, was therefore
not adequate evidence.

With the complete arguments, **both** harness commands exited 0 after at least
120 frames, a running AudioContext, the real click and the oscillator-ended
event. Their screenshots visibly show the clear colour. WebGPU reported the
`BrowserWebGpu` backend and a CPU adapter; WebGL reported ANGLE / Vulkan
SwiftShader. Evidence: `logs/web-webgpu.log`, `logs/web-webgpu.png`,
`logs/web-webgl.log` and `logs/web-webgl.png`.

## jgenesis core on wasm32

The reference revision was
`d41e2d975771c78ee2efe80f8ec5768b87e3f7eb`. It was copied rather than modified in
place:

```sh
cp -a /Users/mooki/Code/toejam-earl-port/scratch/ref/jgenesis /Users/mooki/Code/toejam-earl-port/scratch/Toolchains/jgenesis
```

From the copy's workspace root, the initial command was:

```sh
cargo check -p genesis-core --target wasm32-unknown-unknown
```

It failed in `getrandom` 0.4.3: wasm unknown targets require the `wasm_js` feature.
The existing upstream `jgenesis-web` enables that feature through a direct
`getrandom` dependency. No emulator source changes were needed.

To check precisely the core dependency graph without pulling in the full web
frontend, the scratch `wasm-core-check` library depended on the copied
`backend/genesis-core` and added `getrandom = { version = "0.4", features =
["wasm_js"] }`. It re-exported the core and had its own empty `[workspace]`
section, so Cargo did not accidentally join the port's parent workspace. The
successful command was:

```sh
CARGO_TARGET_DIR="$TC/target" cargo +1.99.1 check --manifest-path "$TC/wasm-core-check/Cargo.toml" --target wasm32-unknown-unknown
```

This checked `genesis-core`, `genesis-components`, the 68000 and Z80 cores,
Genesis configuration, the PSG, Sega CD / 32X dependencies and the common crates.
It exited 0 in **17.10 s wall** (Cargo: 17.08 s; user: 44.34 s; system: 8.03 s),
with 8 jobs, debug info disabled and incremental compilation disabled. The crate
cache and some artifacts from the initial failed check were warm; this is **not**
a clean-machine benchmark.

There were no special `RUSTFLAGS`, atomics, nightly features, `build-std`, or
changes to the copied emulator source. The upstream web frontend's separate
shared-memory/AudioWorklet configuration is not a requirement for checking the
core. This establishes compilation, not emulator execution or ROM equivalence in
a browser. Evidence: `logs/jgenesis-wasm-check.log`,
`logs/jgenesis-wasm-with-rng.log` and `logs/jgenesis-wasm-with-rng.time`.

## Runtime WebAssembly engine compilation

`tje-engine-wasmgen` compiles `tje-engine-core`'s ROM-specialized primitive IR
inside the wasm32 application. Each original instruction becomes a separate
Wasm function; ordered memory operations call the runtime's raw numeric bus
exports. The module imports application memory, owns a PC-indexed function
table, and installs one entry in the application's function table. Subsequent
runtime steps stay entirely within Wasm, including bus callbacks.

The application must be linked with wasm-ld's `--growable-table`: its default
fixed table maximum prevents installing generated continuations. The workspace's
`.cargo/config.toml` sets this only for `wasm32-unknown-unknown`. Both browser
build scripts explicitly load that configuration even when invoked outside the
workspace. External applications using this backend must supply the same linker
flag; setting unrelated `RUSTFLAGS` must not discard it.

Compilation uses asynchronous `WebAssembly.instantiate`, never the synchronous
`WebAssembly.Module` constructor. Individual instruction functions are bounded
at 256 KiB / 16,384 SSA values and modules at 64 MiB. The cartridge-addressed
dispatch table has at most 524,288 slots. Missing/odd PCs fail closed rather than
entering an interpreter. IndexedDB V2 stores module bytes under the verified
cartridge SHA-1, `ENGINE_VERSION` and actual 68000 master-clock divider. Records
bind all three identities and every module byte with SHA-256 before instantiation.
Invalid records/modules are evicted and regenerated once from the ROM; fresh
compiler errors propagate rather than retrying. V1's bare byte records are removed.
A verified usable hit skips IR generation. Distinct dividers coexist; a new engine
version removes older engine versions for that cartridge.
`BrowserBackend::load(rom_sha1, m68k_divider, build_ir)` requires the same divider
as `compile_program_with_divider` in the builder (normally seven).
Change `ENGINE_VERSION` whenever the generator, core semantics, CPU layout or
runtime import ABI changes. Generated bytes contain cartridge-derived code:
never commit, distribute or upload them.

For a user-requested forgotten cartridge, drop the live engine and await
`BrowserBackend::forget_rom(verified_rom_sha1)` alongside removal of the stored
ROM. This deletes every cached engine version and divider for that cartridge,
including obsolete records, without clearing other ROMs. Storage errors propagate;
the frontend must not report complete deletion if the cache transaction failed.

The opt-in `browser-proof` feature is a **development oracle**, not a frontend
feature: it links the reference emulator and compares complete frames, device
memories, audio sample bits and sound-chip streams. Never enable it for the
shipped browser build. To build and run the private proof harness:

```sh
export CARGO_TARGET_DIR="$P/scratch/wasm-proof/target"
export TJE_WASM_PROOF_DIR="$P/scratch/wasm-proof/pkg"
bash crates/tje-engine-wasmgen/browser/build.sh
# PLAYWRIGHT_MODULE and PLAYWRIGHT_BROWSERS_PATH can select the user-local
# installation described above; no browser packages are vendored.
node crates/tje-engine-wasmgen/browser/verify.mjs --primitives \
  --report "$P/scratch/wasm-proof/primitives.json"
node crates/tje-engine-wasmgen/browser/verify.mjs \
  --rom "$P/roms/rev02.bin" --session sessions/rev02-two-player.tje.json \
  --report "$P/scratch/wasm-proof/rev02-two-player.json"
```

The retail command writes the verifier's `ReplayReport` at `--report`, plus
`*.metrics.json` for cold/warm generation/instantiation times, module size and
uncapped production-device frame-rate capacity. It implements the
`tje-engine-verify --wasm-command` external-runner protocol. `--cache-only`
exercises real Chromium IndexedDB and asynchronous compilation without a ROM
or a built Rust proof module. Measurements and exactness claims belong to the
corresponding private run receipts, not merely a successful wasm32 build.

## macOS CI requirements — not locally verified

[INFERENCE] CI should use real macOS runners, with native ARM64
(`aarch64-apple-darwin`) and x86-64 (`x86_64-apple-darwin`) coverage. The required
steps are:

1. Supply Xcode Command Line Tools, the Apple linker and the macOS SDK. Hosted
   macOS runner images normally provide these; the job should record the selected
   Xcode/SDK rather than assume a Linux linker can produce the final application.
2. Install/select the repository's pinned Rust 1.99.1 and the runner's Rust target.
   A separate wasm job can use the already verified single-threaded wasm route.
3. Build/link the native frontend and run the ROM-free Rust suites on each
   architecture. Default native dependencies use macOS system frameworks for
   windows, Metal graphics, audio and HID; ALSA/udev packages are Linux-only.
4. Retain the built application as a CI artifact. Do not run the Windows GNU
   cross-linker configuration or Linux `PKG_CONFIG_*` overrides in this job.
5. Run the window-and-tone smoke only on a macOS session with WindowServer and a
   real output device. A successful link, a headless CI job, or Rosetta execution
   is not proof of native Metal presentation or CoreAudio playback on both
   architectures.

No macOS command is presented as tested, and no macOS smoke pass is claimed.
The [official Rust Apple target documentation](https://doc.rust-lang.org/rustc/platform-support/apple-darwin.html)
explains the SDK/linker requirement. Actual macOS runtime verification remains
an explicit platform evidence gap, not a hidden success claim.

## Native cycle-exact compiler

`tje-engine-cranelift` is a **native-only** crate. Exclude it, alongside
`tje-native`, from workspace WebAssembly builds. It lowers the compiler core's
primitive SSA directly to one native continuation per original instruction.
Only ordered device-bus callbacks cross into Rust; no opcode decoder or semantic
interpreter is used during execution.

The desktop loading entry point is
`tje_engine_cranelift::load_engine(verified_rom, &config, &cache, progress)`.
It builds the reachable program using committed code/jump-table metadata and
the user's ROM, loads or compiles the complete native program, then starts the
backend-independent device runtime. Progress is synchronous: the frontend should
forward notifications from its loading worker to its existing UI channel.
The returned runtime owns the `play_frame`, `frame_buffer`, audio-rate, reset,
and input interfaces; `engine.backend().stats()` exposes observed loading costs.

`CacheConfig::for_current_user()` uses the same local application-data root as
the desktop frontend, with a private `engine-native` subdirectory. Native code is
keyed and checked against the ROM SHA-1, complete IR SHA-256, engine/ABI version,
target triple and detected CPU/compiler flags. Loading additionally checks a
SHA-256 file checksum and every continuation's address, alignment and bounds.
A damaged or stale cache is rebuilt; other I/O failures are reported. Verified
code is copied into anonymous writable memory and changed to read/execute before
use, never executed from the mutable cache file. Checksums detect corruption, not
an attacker able to replace the current user's application data.

Changing page permissions is not instruction-cache synchronization. After the
RW-to-RX transition, publication calls the matching Wasmtime cache-coherence
helper's `clear_cache` and then `pipeline_flush_mt`, before any entry pointer can
escape. This uses Darwin's `sys_icache_invalidate`, Windows'
`FlushInstructionCache`, and the required AArch64 Linux cache maintenance and
cross-core synchronization. A publication failure returns an error; execution
does not proceed with unsynchronized code.

The Apple Silicon packaging contract is **ad-hoc/unsigned, without Hardened
Runtime**: anonymous pages are writable during population and read/execute after
publication, never simultaneously writable and executable. CI's native Apple
Silicon runner must execute the synthetic lowering tests, including
`code_publication_survives_loading_on_another_thread_and_cache_reuse`, which
checks actual execution on the loading and consuming threads for both cold
compilation and verified cache reload. A cross-build alone is not this evidence.
No local macOS execution result is claimed.

Hardened Runtime is explicitly unsupported by this allocation path. Do not add
`codesign --options runtime` to packaging without implementing and testing the
`MAP_JIT`, `com.apple.security.cs.allow-jit`, and per-thread JIT write-protection
path (`pthread_jit_write_protect_np`, or Apple's newer allowlisted callback API
where applicable). See Apple's
[JIT porting guidance](https://developer.apple.com/documentation/apple-silicon/porting-just-in-time-compilers-to-apple-silicon)
and [`sys_icache_invalidate` contract](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man3/sys_icache_invalidate.3.html).

The small `first_launch` example uses exactly that loader and the production
frame/audio API, with no window, audio-device or interpreter dependency. Its JSON
contains counts and timings only, never ROM, RAM, frame or sample payloads:

```sh
. /home/michael/.local/share/tje-dev/env.sh
cargo run --release -p tje-engine-cranelift --example first_launch -- \
  --rom /private/rev02.bin --data-dir /private/native-cache \
  --warmup-frames 60 --frames 600 --output /private/native-linux.json
```

A fresh data directory measures first-launch compilation; repeating the command
with the same directory measures verified cache loading. The reported backend
startup excludes ROM import and IR lifting, while total loading includes both.
Runtime throughput excludes the explicitly reported warmup frames.

For the Windows GNU route above, build the same example with
`--target x86_64-pc-windows-gnu` and execute its `.exe` through WSL interop.
Pass Windows-accessible paths (obtain them with `wslpath -w`) for the ROM, data
directory and optional report. The Windows process detects its own native ISA
and calling convention; Linux and Windows caches are not interchangeable.
These commands describe the reproducible measurement path, not an assertion
that any particular session or unmeasured platform is exact.
