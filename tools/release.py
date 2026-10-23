#!/usr/bin/env python3
"""Package only explicitly selected public inputs; never walk a checkout or user data."""

from __future__ import annotations

import argparse
import hashlib
import plistlib
import re
import shutil
import struct
import subprocess
import tarfile
import tempfile
import tomllib
import zipfile
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
APP_NAME = "ToeJam & Earl"
APP_ID = "org.toejam-earl-port.native"
EXECUTABLE = "toejam-earl"
PLATFORMS = {
    "linux": "linux-x86_64",
    "windows": "windows-x86_64-gnu",
    "macos": "macos-universal",
}
WEB_FILES = (
    "index.html", "app.js", "audio.js", "audio-worklet.js", "input.js",
    "storage.js", "style.css",
)


def png_chunk(kind: bytes, payload: bytes) -> bytes:
    return (struct.pack(">I", len(payload)) + kind + payload
            + struct.pack(">I", zlib.crc32(kind + payload)))


def logo_png(size: int) -> bytes:
    """Original geometric TJE lettering; no fonts, game art or embedded image data."""
    background = (20, 30, 48, 255)
    paper = (241, 245, 249, 255)
    accent = (45, 212, 191, 255)
    pixels = bytearray(bytes(background) * size * size)

    def rectangle(left: int, top: int, right: int, bottom: int,
                  color: tuple[int, int, int, int]) -> None:
        x0, y0, x1, y1 = (value * size // 32 for value in (left, top, right, bottom))
        row = bytes(color) * (x1 - x0)
        for y in range(y0, y1):
            offset = (y * size + x0) * 4
            pixels[offset:offset + len(row)] = row

    rectangle(4, 6, 28, 8, accent)
    # T, J and E are drawn with simple solid strokes, not a bitmap font.
    rectangle(4, 12, 11, 14, paper)
    rectangle(7, 14, 9, 23, paper)
    rectangle(13, 12, 20, 14, paper)
    rectangle(18, 14, 20, 23, paper)
    rectangle(13, 21, 18, 23, paper)
    rectangle(13, 18, 15, 21, paper)
    rectangle(22, 12, 24, 23, paper)
    rectangle(24, 12, 28, 14, paper)
    rectangle(24, 16, 27, 18, paper)
    rectangle(24, 21, 28, 23, paper)
    rectangle(4, 26, 28, 27, accent)
    rows = b"".join(b"\0" + pixels[y * size * 4:(y + 1) * size * 4]
                    for y in range(size))
    header = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    return (b"\x89PNG\r\n\x1a\n" + png_chunk(b"IHDR", header)
            + png_chunk(b"IDAT", zlib.compress(rows, 9)) + png_chunk(b"IEND", b""))


def make_icons(destination: Path) -> None:
    destination.mkdir(parents=True, exist_ok=True)
    destination.joinpath("toejam-earl.png").write_bytes(logo_png(256))
    sizes = (16, 32, 48, 64, 128, 256)
    images = [logo_png(size) for size in sizes]
    entries = bytearray()
    offset = 6 + 16 * len(sizes)
    for size, image in zip(sizes, images, strict=True):
        entries.extend(struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0,
                                   1, 32, len(image), offset))
        offset += len(image)
    destination.joinpath("toejam-earl.ico").write_bytes(
        struct.pack("<HHH", 0, 1, len(sizes)) + entries + b"".join(images))
    chunks = b"".join(kind + struct.pack(">I", len(image) + 8) + image
                      for kind, image in ((b"ic07", logo_png(128)),
                                          (b"ic08", logo_png(256)),
                                          (b"ic09", logo_png(512)),
                                          (b"ic10", logo_png(1024))))
    destination.joinpath("toejam-earl.icns").write_bytes(
        b"icns" + struct.pack(">I", len(chunks) + 8) + chunks)
    # windres resolves this relative to the working directory, not the RC file.
    destination.joinpath("toejam-earl.rc").write_text(
        '1 ICON "toejam-earl.ico"\n', encoding="utf-8")


def copy_public_notes(destination: Path) -> None:
    for source in ("README.md", "LICENSE", "docs/RELEASING.md", "docs/PLAN.md",
                   "docs/STATUS.md", "docs/SESSIONS.md", "docs/TOOLCHAIN.md",
                   "vendor/jgenesis/NOTICE.md"):
        target = destination / source
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / source, target)
    revision = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    destination.joinpath("SOURCE.txt").write_text(
        "Source: https://github.com/mschienbein/toejam-earl-port\n"
        f"Commit: {revision}\nLicense: GPL-3.0 (see LICENSE)\n"
        "No ROM or game assets are included. Supply your own cartridge.\n",
        encoding="utf-8")


def archive_zip(directory: Path, output: Path) -> None:
    with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for path in sorted(directory.rglob("*")):
            if path.is_file():
                archive.write(path, path.relative_to(directory.parent))


def stage_linux(binary: Path, destination: Path) -> None:
    executable = destination / EXECUTABLE
    shutil.copyfile(binary, executable)
    executable.chmod(0o755)
    destination.joinpath("toejam-earl.png").write_bytes(logo_png(256))
    destination.joinpath("toejam-earl.desktop").write_text(
        "[Desktop Entry]\nType=Application\nName=ToeJam & Earl\n"
        "Comment=Faithful port; requires your own cartridge\n"
        "Exec=toejam-earl\nIcon=toejam-earl\nTerminal=false\n"
        "Categories=Game;\nStartupWMClass=toejam-earl\n", encoding="utf-8")


def stage_macos(binary: Path, destination: Path, version: str) -> None:
    bundle = destination / f"{APP_NAME}.app"
    contents = bundle / "Contents"
    executable = contents / "MacOS" / EXECUTABLE
    executable.parent.mkdir(parents=True)
    shutil.copyfile(binary, executable)
    executable.chmod(0o755)
    resources = contents / "Resources"
    resources.mkdir()
    with tempfile.TemporaryDirectory() as temporary:
        icons = Path(temporary)
        make_icons(icons)
        shutil.copyfile(icons / "toejam-earl.icns", resources / "toejam-earl.icns")
    crate_version = tomllib.loads((ROOT / "crates/tje-native/Cargo.toml").read_text(
        encoding="utf-8"))["package"]["version"]
    match = re.match(r"v?(\d+\.\d+\.\d+)(?:[-+].*)?$", version)
    bundle_version = match.group(1) if match else crate_version
    metadata = {
        "CFBundleName": APP_NAME,
        "CFBundleDisplayName": APP_NAME,
        "CFBundleIdentifier": APP_ID,
        "CFBundleExecutable": EXECUTABLE,
        "CFBundleIconFile": "toejam-earl.icns",
        "CFBundlePackageType": "APPL",
        "CFBundleShortVersionString": bundle_version,
        "CFBundleVersion": bundle_version,
        "LSMinimumSystemVersion": "11.0",
        "NSHighResolutionCapable": True,
        "NSHumanReadableCopyright": "GPL-3.0; game data supplied separately by the user",
    }
    with contents.joinpath("Info.plist").open("wb") as stream:
        plistlib.dump(metadata, stream)
    subprocess.run(["lipo", "-verify_arch", "x86_64", "arm64", str(executable)], check=True)
    subprocess.run(["codesign", "--force", "--sign", "-", str(bundle)], check=True)
    subprocess.run(["codesign", "--verify", "--deep", "--strict", str(bundle)], check=True)


def package_native(platform: str, binary: Path, version: str, output: Path) -> Path:
    if not binary.is_file() or binary.is_symlink():
        raise ValueError(f"expected a regular release executable: {binary}")
    output.mkdir(parents=True, exist_ok=True)
    stem = f"toejam-earl-{version}-{PLATFORMS[platform]}"
    appdir = output / f"{stem}.AppDir"
    if platform == "linux" and (appdir.exists() or appdir.is_symlink()):
        raise ValueError(f"AppDir must not already exist: {appdir}")
    with tempfile.TemporaryDirectory(dir=output) as temporary:
        directory = Path(temporary) / stem
        directory.mkdir()
        copy_public_notes(directory)
        if platform == "linux":
            stage_linux(binary, directory)
            artifact = output / f"{stem}.tar.gz"
            with tarfile.open(artifact, "w:gz") as archive:
                archive.add(directory, arcname=stem)
            # appimagetool consumes this independently of the portable tarball.
            appdir.mkdir()
            stage_linux(binary, appdir)
            copy_public_notes(appdir)
            appdir.joinpath("AppRun").write_text(
                '#!/bin/sh\nHERE="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"\n'
                'exec "$HERE/toejam-earl" "$@"\n', encoding="utf-8")
            appdir.joinpath("AppRun").chmod(0o755)
        else:
            if platform == "windows":
                shutil.copyfile(binary, directory / f"{EXECUTABLE}.exe")
                icons = directory / "icons"
                make_icons(icons)
                # Only the ICO is useful to a portable Windows installation.
                for name in ("toejam-earl.png", "toejam-earl.icns", "toejam-earl.rc"):
                    icons.joinpath(name).unlink()
            else:
                stage_macos(binary, directory, version)
            artifact = output / f"{stem}.zip"
            archive_zip(directory, artifact)
    return artifact


def package_web(version: str, output: Path, site: Path) -> Path:
    # Exact allowlist: never copy the whole web tree (tests, caches or user data).
    site.mkdir(parents=True, exist_ok=True)
    if any(site.iterdir()):
        raise ValueError(f"web output must be empty: {site}")
    for name in WEB_FILES:
        shutil.copyfile(ROOT / "web" / name, site / name)
    package = site / "pkg"
    package.mkdir()
    for name in ("tje_web.js", "tje_web_bg.wasm"):
        shutil.copyfile(ROOT / "web/pkg" / name, package / name)
    copy_public_notes(site)
    site.joinpath(".nojekyll").touch()
    site.joinpath("icon.png").write_bytes(logo_png(256))
    index = site / "index.html"
    index.write_text(index.read_text(encoding="utf-8").replace(
        "</head>", '  <link rel="icon" href="./icon.png" type="image/png">\n</head>'),
        encoding="utf-8")
    output.mkdir(parents=True, exist_ok=True)
    artifact = output / f"toejam-earl-{version}-web.zip"
    archive_zip(site, artifact)
    return artifact


def checksums(output: Path) -> Path:
    artifacts = sorted(path for path in output.iterdir() if path.is_file()
                       and (path.name.endswith((".tar.gz", ".zip", ".AppImage"))))
    if not artifacts:
        raise ValueError(f"no release artifacts in {output}")
    lines = []
    for artifact in artifacts:
        with artifact.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        lines.append(f"{digest}  {artifact.name}\n")
    manifest = output / "SHA256SUMS.txt"
    manifest.write_text("".join(lines), encoding="utf-8")
    return manifest


def release_version(value: str) -> str:
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,95}", value):
        raise argparse.ArgumentTypeError("version must be a safe, nonempty filename component")
    return value


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    subcommands = parser.add_subparsers(dest="command", required=True)
    icons = subcommands.add_parser("icons", help="generate original text-logo platform icons")
    icons.add_argument("--out", type=Path, required=True)
    native = subcommands.add_parser("native", help="package a built native executable")
    native.add_argument("--platform", choices=PLATFORMS, required=True)
    native.add_argument("--binary", type=Path, required=True)
    native.add_argument("--version", type=release_version, required=True)
    native.add_argument("--out", type=Path, required=True)
    web = subcommands.add_parser("web", help="stage the built browser app and create its ZIP")
    web.add_argument("--version", type=release_version, required=True)
    web.add_argument("--out", type=Path, required=True)
    web.add_argument("--site", type=Path, required=True)
    sums = subcommands.add_parser("checksums", help="hash final archives and AppImages")
    sums.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    if args.command == "icons":
        make_icons(args.out)
    elif args.command == "native":
        print(package_native(args.platform, args.binary, args.version, args.out))
    elif args.command == "web":
        print(package_web(args.version, args.out, args.site))
    else:
        print(checksums(args.out))


if __name__ == "__main__":
    main()
