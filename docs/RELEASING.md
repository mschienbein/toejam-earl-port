# Building and releasing

Native packages use the cycle-exact engine compiled from the user's cartridge at
first launch. The shipped native dependency graph excludes the hybrid host,
68000 interpreter and development verifier. Packaging alone is not a game-fidelity
proof; see [PLAN.md](PLAN.md) and the separate engine acceptance receipts.
No ROM, extracted asset, capture, save, user configuration or generated native
code cache is an input to a release build.

## Workflows and publication boundaries

- [Native release artifacts](../.github/workflows/release-native.yml) builds Linux,
  Windows GNU and universal macOS packages and uploads them as Actions artifacts.
  It does not create or publish a GitHub Release.
- [Web release and Pages](../.github/workflows/release-web.yml) builds the browser
  app with **wasm-bindgen CLI 0.2.129**, uploads a downloadable ZIP, then deploys
  exactly that staged static tree to GitHub Pages.
- Both workflows start only on a pushed `v*` tag or **Run workflow**. A manual run
  on a branch other than the repository's default branch is skipped; selecting an
  existing `v*` tag is also allowed. There is no pull-request, lane-branch push,
  schedule, `workflow_run` or reusable-workflow trigger. Ordinary CI is unchanged.
- A tag's name becomes its artifact version. Default-branch manual builds use
  `manual-<full commit SHA>`. Version labels must be safe filename components.
  Every package also contains `SOURCE.txt` with its exact source commit.
- Native and downloadable web artifacts are retained for **30 days**. Maintainers
  must download and attach the desired archives and manifests to a GitHub Release
  separately if durable release downloads are wanted. Publish the corresponding
  complete GPL source, including the pinned vendored code, with the binaries.

**Do not push a tag or dispatch the Pages workflow just to test the YAML.** A web
run really deploys after the `github-pages` environment permits it. Workflow
validation and all packaging steps below can be run without publishing anything.

### One-time GitHub Pages configuration

A repository administrator must select **Settings → Pages → Build and deployment
→ Source: GitHub Actions**. Configure the `github-pages` environment to permit the
default branch and `v*` tags; an optional required reviewer provides an additional
publication hold. The Pages job uses `pages: write` and OIDC `id-token: write` only
in the deployment job. Build jobs have read-only repository access and checkout
credentials are not persisted. No personal token, ROM secret, server-side ROM
endpoint, custom domain, COOP/COEP headers or asset extraction is needed.

After a maintainer intentionally pushes a release tag (or manually dispatches the
web workflow on an allowed ref), the deployed URL appears in the workflow's
`github-pages` environment. Repository subpaths work because runtime URLs are
relative. The ZIP and Pages deployment contain the same site files. ROM imports
remain in the visitor's browser storage and are never part of the deployment.

## Artifacts and runtime requirements

| Artifact suffix | Contents and requirements |
| --- | --- |
| `linux-x86_64.tar.gz` | Portable directory with `toejam-earl`, desktop metadata, original text-logo PNG, README, license and source receipt. CI builds on Ubuntu 22.04 (glibc 2.35 baseline). |
| `linux-x86_64.AppImage` | The same application in an AppDir with an `AppRun` launcher. It deliberately uses the host's glibc, ALSA, udev, window system and GPU driver rather than bundling system libraries. |
| `windows-x86_64-gnu.zip` | 64-bit Windows executable with embedded original text-logo icon, plus portable ICO, README, license and source receipt. Built with MinGW GCC POSIX; no Visual Studio installation is needed to run it. |
| `macos-universal.zip` | `ToeJam & Earl.app`, with x86_64 and arm64 slices, Info.plist, ICNS text-logo and ad-hoc code signature; macOS 11 or later. Includes README, license and source receipt beside the app. |
| `web.zip` | Static HTML, JavaScript, generated wasm-bindgen JS/Wasm, original text-logo favicon, license, README and source receipt. Serve via HTTPS or localhost, not `file://`. |

Each native artifact filename begins `toejam-earl-<version>-`. Each Actions download
also contains `SHA256SUMS.txt` covering its final archive(s) and, for Linux, the
AppImage. Checksums detect corruption; they are not a developer signature.

All packages also include [REVISIONS.md](REVISIONS.md), documenting the eight
retail behavior differences retained when the user supplies REV 00 rather than
REV 02, and the [native controls guide](../crates/tje-native/README.md).

Linux needs `libasound.so.2`, `libudev.so.1`, an X11 or Wayland desktop with
xkbcommon and a working Vulkan or OpenGL driver. AppImage does not make an old
system's libc compatible with a newer build. Use the tarball if FUSE is absent,
or invoke `./toejam-earl-<version>-linux-x86_64.AppImage --appimage-extract-and-run`.
Local WSL builds use the host toolchain and are not evidence for the CI baseline.
See [TOOLCHAIN.md](TOOLCHAIN.md) for the observed WSLg limitations and dependency
setup. Windows needs a compatible graphics driver (DX12, Vulkan or OpenGL).
macOS uses Metal. No macOS build/run claim is implied by local Linux packaging.

The binaries are not developer-signed or notarized. Windows SmartScreen or macOS
Gatekeeper may warn. Verify the checksum and source before using the operating
system's explicit user approval mechanism; do not disable system-wide protection.

## Running a package

1. Extract the archive into a writable folder (macOS: move the `.app` to
   Applications if desired). On Linux, keep the executable permission when
   extracting; `chmod +x` the AppImage if necessary.
2. Start `toejam-earl.exe`, `./toejam-earl`, the AppImage, or the macOS `.app`.
   Drop your own REV 02 / REV 00 cartridge onto the window on first launch. The
   CLI alternative is `./toejam-earl --rom /path/to/cartridge.bin` (Windows:
   `.\toejam-earl.exe --rom C:\path\to\cartridge.bin`; macOS:
   `"ToeJam & Earl.app/Contents/MacOS/toejam-earl" --rom /path/to/cartridge.bin`).
3. The normalized cartridge is remembered in the OS **local data** directory;
   later launches find it automatically and load its compiled engine cache with
   no file prompt. Settings use the OS configuration directory, generated code
   the OS cache directory. Windows cartridge/session data and code cache use
   `%LOCALAPPDATA%`, not redirected Roaming AppData; only settings roam. The first
   compilation displays progress. F1 → Cartridge / help can replace/forget the
   cartridge; `--forget-rom` removes its saved copy and compiled cache while
   preserving settings. `--data-dir PATH` isolates all these locations; do not
   publish its cartridge or cache.
4. Native player 1: arrows, Z/X/C = A/B/C, Enter = Start. Player 2: W/S/A/D,
   G/H/J = A/B/C, Space = Start. Esc pauses, F5 resets, F11 toggles fullscreen,
   F6 changes scaling, F7 changes filtering, F8 changes vsync, PageUp/PageDown
   change volume. F9 records controller inputs (and hard-resets when starting);
   see [SESSIONS.md](SESSIONS.md). `--help` lists all options.
5. For the web build, visit the Pages URL or serve the extracted site directory
   with `python3 -m http.server 8000 --directory PATH`, then open
   `http://localhost:8000`. Choose the ROM locally and click **Play / enable
   audio**. Use the controls listed on the page; browser player-2 and gamepad
   bindings differ from native defaults. WebGPU falls back to WebGL 2.

Native archives do not install file associations or write beside the executable.
They can be removed by deleting the extracted folder; user data is separate.

### Windows network-path override limitation

Use the normal local directories, or a local-disk `--data-dir`. An explicit UNC override such as
`\\wsl.localhost\Ubuntu\...` can stall during atomic native-cache publication.
The one instrumented test completed ROM import (18 ms), native compilation
(12.55 s), buffered cache payload (0.57 s) and `sync_all` (11 ms), then remained
inside the temporary file's atomic persist operation for over 96 seconds.
No timeout-free UNC guarantee is made. The application does not weaken atomic
publication/checksums or silently switch execution backends to bypass this.
Default Windows data/cache are local even when Roaming AppData is redirected.

## Local builds, without publication

Use the Rust version and targets in `rust-toolchain.toml`. All builds use the
committed Cargo lockfile. Build products, icons, staging trees and downloaded
tools must stay under ignored `scratch/` (or outside the checkout). The packaging
helper uses explicit file allowlists; it never recursively packages the checkout,
`roms/`, a target directory, or user data. Its only binary input is the executable
you explicitly name (or the two generated web runtime files).

From the repository root on Linux:

```sh
# On this WSL development machine; other contributors install their own deps.
. /home/michael/.local/share/tje-dev/env.sh
export CARGO_TARGET_DIR="$PWD/scratch/release-target"
export CARGO_BUILD_JOBS=4
export CARGO_INCREMENTAL=0
VERSION=local
OUT="$PWD/scratch/release/dist"
cargo build --locked --release --package tje-native --bin toejam-earl
python3 tools/release.py native --platform linux \
  --binary "$CARGO_TARGET_DIR/release/toejam-earl" --version "$VERSION" --out "$OUT"
```

The Linux packaging command also stages
`$OUT/toejam-earl-$VERSION-linux-x86_64.AppDir`. For an AppImage, use
[appimagetool 1.9.1](https://github.com/AppImage/appimagetool/releases/tag/1.9.1)
and [type2-runtime 20251108](https://github.com/AppImage/type2-runtime/releases/tag/20251108).
The native workflow pins both download SHA-256 values and demonstrates the
FUSE-free invocation with `--runtime-file`. GPU drivers and system libraries are
not copied into this AppDir.

### Windows GNU cross-build (Linux / WSL)

Follow the Windows user-local GNU cross-build section of
[TOOLCHAIN.md](TOOLCHAIN.md) for the MinGW installation. With the environment
above, the explicit `-posix` compiler name matters for an unpacked, non-system installation:

```sh
rustup target add x86_64-pc-windows-gnu
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="$LOCAL/usr/bin/x86_64-w64-mingw32-gcc-posix"
export CC_x86_64_pc_windows_gnu="$LOCAL/usr/bin/x86_64-w64-mingw32-gcc-posix"
export AR_x86_64_pc_windows_gnu="$LOCAL/usr/bin/x86_64-w64-mingw32-ar"
ICONS="$PWD/scratch/release/icons"
python3 tools/release.py icons --out "$ICONS"
(
  cd "$ICONS"
  "$LOCAL/usr/bin/x86_64-w64-mingw32-windres" \
    --preprocessor "$CC_x86_64_pc_windows_gnu" \
    --preprocessor-arg=-E --preprocessor-arg=-xc --preprocessor-arg=-DRC_INVOKED \
    --input toejam-earl.rc --output icon.o --output-format coff
)
cargo rustc --locked --release --package tje-native --bin toejam-earl \
  --target x86_64-pc-windows-gnu -- -C "link-arg=$ICONS/icon.o"
python3 tools/release.py native --platform windows \
  --binary "$CARGO_TARGET_DIR/x86_64-pc-windows-gnu/release/toejam-earl.exe" \
  --version "$VERSION" --out "$OUT"
```

The workflow installs the equivalent distribution MinGW packages. The release
build does not need Wine or Windows interop to produce the ZIP. Cross-compilation
alone is not a Windows gameplay smoke test.

### macOS universal bundle

On a macOS host with Xcode command-line tools, follow the native workflow's two
`cargo build --target` commands, combine their executables with `lipo -create`,
then run `tools/release.py native --platform macos --binary PATH ...`. The helper
requires both `x86_64` and `arm64`, creates the app metadata and icon, and ad-hoc
signs and verifies the complete bundle before ZIP creation. It is not a
cross-build route from Linux and does not perform Apple notarization.

### Browser and checksums

```sh
cargo install wasm-bindgen-cli --version 0.2.129 --locked --root "$PWD/scratch/bindgen"
WASM_BINDGEN="$PWD/scratch/bindgen/bin/wasm-bindgen" bash web/build.sh
# --site must be empty; do not point it at web/ or an existing deployment.
python3 tools/release.py web --version "$VERSION" --out "$OUT" \
  --site "$PWD/scratch/release/site"
# Run this after AppImage creation too, if building one.
python3 tools/release.py checksums --out "$OUT"
(cd "$OUT" && sha256sum --check SHA256SUMS.txt)
```

On macOS verify with `shasum -a 256 -c SHA256SUMS.txt`. On Windows use PowerShell
`Get-FileHash .\toejam-earl-<version>-windows-x86_64-gnu.zip -Algorithm SHA256`
and compare with the matching manifest row.

## Validation before a maintainer releases

```sh
actionlint .github/workflows/release-native.yml .github/workflows/release-web.yml
python3 -m unittest discover -s tools -p 'test_release.py' -v
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo build --locked --workspace --target wasm32-unknown-unknown --exclude tje-native --exclude tje-engine-cranelift --exclude tje-native-proof
python3 tools/audit_public.py
```

Use a current actionlint (1.7.12 was used for the initial release workflow
validation). The workflows additionally run the ROM-free full-history public
audit before building. Maintainers must separately complete the private fidelity
and platform run checks before calling a build a finished native release. Do not
commit generated icons: `tools/release.py` draws the original navy/teal **TJE**
text mark procedurally, with no fonts, game art, extracted bytes or placeholder
artwork. PNG, ICO and ICNS files exist only in scratch and built artifacts.

## Third-party licences and notices

Every package and the deployed static site include `THIRD_PARTY_NOTICES.txt`,
`THIRD_PARTY_DEPENDENCIES.json`, and `THIRD_PARTY_LICENSES/*.txt`. These contain
the full selected licence texts, actual upstream copyright/licence/NOTICE files,
and the pinned Rust standard library's complete copyright report and reference
licences. The large standard-library report is split at text boundaries to keep
each public source file below the publication audit's blob limit; nothing is
truncated. Source repositories and precise package versions are listed alongside
the obligations. Upstream alternative licence texts are retained as notices,
not a claim that every alternative is selected.
Full legal texts are presented as quotations with whitespace reflowed for
readability. An equality check ensures every upstream word and punctuation
token remains in order. The quotation layout also prevents decorative rules
or disclaimer lines being mistaken for nested encodings by the public audit;
no content exemption or licence clause is added.
The macOS `.app` retains its legal payload under `Contents/Resources`, not just
beside the bundle. The generated web page links to its published notices.

`tools/notices.py` uses **cargo-about 0.9.2**, not an unpinned global installation.
On Linux x86_64 it bootstraps the published musl executable into ignored
`scratch/notice-tools/`, verifies its archive SHA-256 and checks its version.
`CARGO_ABOUT` may instead name a user-local installation of that exact version.
It performs no native builds and needs no private ROM, compiler probes or
extracted game data:

```sh
python3 tools/notices.py          # regenerate committed third-party/ outputs
python3 tools/notices.py --check  # fail on stale notices or unaccepted licences
```

The accepted selections are explicit in `tools/notices.toml`. Generation uses
the actual default-feature release roots, not the whole development workspace:
`tje-native` for Linux x86_64, Windows GNU x86_64, macOS x86_64 and arm64, and
`tje-web` for wasm32. Dev-only dependencies are excluded; normal and build
dependencies are retained. Every harvested graph is independently compared
with `cargo tree --locked --edges normal,build --target ...`; any coverage
difference fails. The universal macOS notice is the union of its two graphs.
Crates with missing vendored metadata use checksum-verified upstream GPL
clarifications, not inferred permissions.

CI and both release workflows regenerate all four graph receipts and compare
every byte. A lockfile, manifest, production JS/CSS, toolchain or policy change
requires regeneration. Public-input fingerprints normalize Git CRLF checkout
translation, so native Windows packaging accepts the same reviewed graph.
Packaging also fails on stale inputs, unaccepted selections, absent/corrupt
licence texts and notice hash mismatches; the existing symlink and ROM-input
guards apply to the notice payload too.

### Compatibility assessment and explicit font flags

This is an engineering assessment of the pinned upstream terms, not a substitute
for legal advice. The selected **software** licences permit this GPL-3.0-only
combination:

- **MIT, BSD-2-Clause, BSD-3-Clause, ISC and Zlib** are permissive/GPL-compatible.
  Preserve their notices. Less-common terms are scoped to the exact reviewed
  packages, rather than accepting arbitrary new dependencies automatically.
- **Apache-2.0** is compatible with **GPLv3**, not GPLv2-only. Its full terms and
  every packaged upstream NOTICE are retained, including the Apache-only
  `cpal` dependency. LLVM exceptions are additional permissions and their actual
  upstream texts are retained where supplied.
- **Unicode-3.0** permits redistribution/modification with its notice. Both its
  terms and the associated Unicode data notices are retained.
- **MPL-2.0** (`option-ext`) is combined under the secondary-licence mechanism
  of section 3.3. Its source repository/version and full MPL terms are provided.
  Generation rejects an attached “Incompatible With Secondary Licenses” notice
  in its code rather than silently accepting that materially different case.
- **GPL-3.0-only** applies to the project and retained jgenesis components.
  Rust's full library report also retains its platform/source exceptions and
  upstream LGPL-2.1-or-later notices; the latter is compatible with GPLv3.

References: [FSF licence compatibility list](https://www.gnu.org/licenses/license-list.html),
[GPLv3 compatibility explanation](https://www.gnu.org/licenses/quick-guide-gplv3),
and [Mozilla MPL FAQ, questions 14 and 25](https://www.mozilla.org/en-US/MPL/2.0/FAQ/).
No GPL-2.0-only software selection or unknown licence is accepted. Merely
including an upstream alternative or compiler reference licence text does not
select that licence for the combined program.

**Font flag — these are separately licensed assets, not GPL-relicensed code.**
Native egui embeds Hack, Ubuntu Light and the egui icon subset. Hack carries
Source Foundry's MIT terms, DejaVu public-domain attribution and **Bitstream Vera**
terms. Ubuntu Light remains under **Ubuntu-font-1.0**; **OFL-1.1** also appears in
the font package's aggregate declaration. The Bitstream, OFL and Ubuntu terms
are accepted only for `epaint_default_fonts`, not as general GPL-compatible
software licences. In particular, Ubuntu's licence prohibits relicensing the
font under GPL; the font is redistributed unchanged as a separately licensed
embedded asset. Its embedding/distribution permission does not erase its own
notice, naming and trademark conditions. These distinct terms are flagged in
every native notice, with their complete text. Maintainers must preserve that
asset/code distinction and must not describe every bundled byte as GPL-only.
See [Canonical's font licence/embedding FAQ](https://canonical.com/legal/font-licence/faq).
The precise font notice files and version are hash-pinned in
`tools/font-notices.json`; a changed font/licence requires explicit review.

The browser ships repository-owned GPL JS plus generated wasm-bindgen glue and
the repository's engine snippet; no npm package is a runtime asset. Its CSS uses
system fonts, so no native font file is included in the web site. Release logos
are procedural and contain no external font. This inventory/fingerprint must be
updated before adding a new JS library, font, or other licensed runtime asset.

