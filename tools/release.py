#!/usr/bin/env python3
"""Package validated public inputs with reproducible archive metadata and ROM checks."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import io
import os
import plistlib
import posixpath
import re
import shutil
import stat
import struct
import subprocess
import tarfile
import tempfile
import tomllib
import time
import zipfile
import zlib
from pathlib import Path, PurePosixPath

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
KNOWN_ROM_SHA1 = frozenset((
    "85e8d0a4fac591b25b77c35680ac4175976f251b",
    "7f82d8b57fff88bdca5d8aff85b01e231dc1239a",
))


def known_rom_hashes() -> set[str]:
    """Also reject any private reference image, even if its header was modified."""
    hashes = set(KNOWN_ROM_SHA1)
    for name in ("rev02.bin", "rev00.bin"):
        try:
            image = (ROOT / "roms" / name).read_bytes()
        except FileNotFoundError:
            continue
        hashes.add(hashlib.sha1(image).hexdigest())
    return hashes


def reject_rom(data: bytes, name: object, rom_hashes: set[str]) -> None:
    if data[0x100:0x104] == b"SEGA":
        raise ValueError(f"Genesis ROM signature in release input: {name}")
    if hashlib.sha1(data).hexdigest() in rom_hashes:
        raise ValueError(f"known ROM hash in release input: {name}")


def read_input(path: Path, rom_hashes: set[str], root: Path | None = None) -> bytes:
    # Check every component below a public root, including web/pkg itself.
    candidates = [path]
    if root is not None:
        relative = path.relative_to(root)
        candidates.extend(root / parent for parent in relative.parents)
    if any(candidate.is_symlink() for candidate in candidates) or not path.is_file():
        raise ValueError(f"expected a regular, non-symlink release input: {path}")
    data = path.read_bytes()
    reject_rom(data, path, rom_hashes)
    return data


def validate_native_binary(path: Path, platform: str, rom_hashes: set[str]) -> None:
    data = read_input(path, rom_hashes)
    if platform == "linux":
        valid = (len(data) >= 64 and data[:7] == b"\x7fELF\x02\x01\x01"
                 and struct.unpack_from("<H", data, 16)[0] in (2, 3)
                 and struct.unpack_from("<H", data, 18)[0] == 62)
    elif platform == "windows":
        valid = False
        if len(data) >= 64 and data[:2] == b"MZ":
            offset = struct.unpack_from("<I", data, 60)[0]
            if 64 <= offset <= len(data) - 24:
                machine, _, _, _, _, optional_size, flags = struct.unpack_from(
                    "<HHIIIHH", data, offset + 4)
                valid = (data[offset:offset + 4] == b"PE\0\0" and machine == 0x8664
                         and flags & 2 != 0 and flags & 0x2000 == 0
                         and optional_size >= 112 and offset + 24 + optional_size <= len(data)
                         and struct.unpack_from("<H", data, offset + 24)[0] == 0x20B)
    else:
        valid = valid_universal_macho(data)
    if not valid:
        raise ValueError(f"expected a {PLATFORMS[platform]} executable: {path}")


def valid_universal_macho(data: bytes) -> bool:
    formats = {
        b"\xca\xfe\xba\xbe": (">", False), b"\xbe\xba\xfe\xca": ("<", False),
        b"\xca\xfe\xba\xbf": (">", True), b"\xbf\xba\xfe\xca": ("<", True),
    }
    if len(data) < 8 or data[:4] not in formats:
        return False
    endian, wide = formats[data[:4]]
    count = struct.unpack_from(endian + "I", data, 4)[0]
    entry_size = 32 if wide else 20
    if count != 2 or len(data) < 8 + count * entry_size:
        return False
    architectures = set()
    ranges = []
    for index in range(count):
        entry = struct.unpack_from(endian + ("IIQQII" if wide else "IIIII"),
                                   data, 8 + index * entry_size)
        cpu, _, offset, size = entry[:4]
        if (cpu not in (0x1000007, 0x100000C) or cpu in architectures
                or offset < 8 + count * entry_size or size < 32
                or offset + size > len(data)):
            return False
        header = data[offset:offset + 32]
        if (header[:4] != b"\xcf\xfa\xed\xfe"
                or struct.unpack_from("<I", header, 4)[0] != cpu
                or struct.unpack_from("<I", header, 12)[0] != 2):
            return False
        architectures.add(cpu)
        ranges.append((offset, offset + size))
    return len(architectures) == 2 and sorted(ranges)[0][1] <= sorted(ranges)[1][0]


def archive_epoch() -> int:
    # No wall-clock fallback: local builds are reproducible without setting it.
    value = int(os.environ.get("SOURCE_DATE_EPOCH", "0"))
    if not 0 <= value <= 0xFFFFFFFF:
        raise ValueError("SOURCE_DATE_EPOCH must be an unsigned 32-bit timestamp")
    return value


def archive_mode(name: str) -> int:
    return 0o755 if PurePosixPath(name).name in (
        EXECUTABLE, f"{EXECUTABLE}.exe", "AppRun") else 0o644


def scan_directory(directory: Path, rom_hashes: set[str]) -> None:
    for path in sorted(directory.rglob("*")):
        if path.is_symlink():
            raise ValueError(f"symlink in release staging tree: {path}")
        if not path.is_dir():
            read_input(path, rom_hashes, directory)


def scan_appimage(artifact: Path, header: bytes, rom_hashes: set[str]) -> None:
    # Locate the SquashFS using ELF metadata; never execute the artifact to unpack it.
    if (len(header) < 64 or header[:7] != b"\x7fELF\x02\x01\x01"
            or header[8:11] != b"AI\x02"
            or struct.unpack_from("<H", header, 18)[0] != 62):
        raise ValueError(f"expected a type-2 x86_64 AppImage: {artifact}")
    offset = (struct.unpack_from("<Q", header, 40)[0]
              + struct.unpack_from("<H", header, 58)[0]
              * struct.unpack_from("<H", header, 60)[0])
    if header[offset:offset + 4] != b"hsqs":
        raise ValueError(f"missing AppImage SquashFS at ELF boundary: {artifact}")
    command = ["unsquashfs", "-o", str(offset), str(artifact.resolve())]
    listing = subprocess.check_output([command[0], "-lln", *command[1:]], text=True)
    entries = {}
    links = {}
    for line in listing.splitlines():
        if not line:
            continue
        fields = line.split(maxsplit=5)
        if len(fields) != 6 or not re.fullmatch(r"[-dl][rwxstST-]{9}", fields[0]):
            raise ValueError(f"unsupported AppImage directory entry: {artifact}")
        name = fields[5]
        if fields[0][0] == "l":
            name, separator, target = name.partition(" -> ")
            if not separator:
                raise ValueError(f"invalid AppImage symbolic link: {artifact}")
            links[name] = posixpath.normpath(posixpath.join(posixpath.dirname(name), target))
        if (not re.fullmatch(r"squashfs-root(?:/[A-Za-z0-9._-]+)*", name)
                or ".." in PurePosixPath(name).parts or name in entries):
            raise ValueError(f"unsafe AppImage member path: {artifact}")
        entries[name] = (fields[0][0], int(fields[2]))
    if not entries:
        raise ValueError(f"empty AppImage filesystem: {artifact}")
    for name, target in links.items():
        visited = {name}
        while target in links and target not in visited:
            visited.add(target)
            target = links[target]
        if target in visited or target not in entries:
            raise ValueError(f"external or cyclic AppImage symbolic link: {artifact}")
    for name, (kind, size) in entries.items():
        if kind == "-":
            member = name.removeprefix("squashfs-root/")
            data = subprocess.check_output(
                [command[0], "-processors", "1", "-cat", *command[1:], member])
            if len(data) != size:
                raise ValueError(f"AppImage member size mismatch: {artifact}")
            reject_rom(data, f"{artifact}:{member}", rom_hashes)


def scan_artifact(artifact: Path, rom_hashes: set[str] | None = None) -> None:
    hashes = known_rom_hashes() if rom_hashes is None else rom_hashes
    data = read_input(artifact, hashes)
    if artifact.name.endswith(".AppImage"):
        scan_appimage(artifact, data, hashes)
    elif artifact.name.endswith(".zip"):
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            for member in archive.infolist():
                if stat.S_ISLNK(member.external_attr >> 16):
                    raise ValueError(f"symlink in release ZIP: {member.filename}")
                if not member.is_dir():
                    reject_rom(archive.read(member), f"{artifact}:{member.filename}", hashes)
    elif artifact.name.endswith(".tar.gz"):
        with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
            for member in archive:
                if member.isdir():
                    continue
                if not member.isfile():
                    raise ValueError(f"non-regular member in release tar: {member.name}")
                stream = archive.extractfile(member)
                assert stream is not None
                with stream:
                    reject_rom(stream.read(), f"{artifact}:{member.name}", hashes)
    else:
        raise ValueError(f"unsupported release artifact: {artifact}")


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


def copy_public_notes(destination: Path, rom_hashes: set[str]) -> None:
    for source in ("README.md", "LICENSE", "docs/RELEASING.md", "docs/PLAN.md",
                   "docs/STATUS.md", "docs/SESSIONS.md", "docs/TOOLCHAIN.md",
                   "vendor/jgenesis/NOTICE.md"):
        target = destination / source
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(read_input(ROOT / source, rom_hashes, ROOT))
    revision = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    destination.joinpath("SOURCE.txt").write_text(
        "Source: https://github.com/mschienbein/toejam-earl-port\n"
        f"Commit: {revision}\nLicense: GPL-3.0 (see LICENSE)\n"
        "No ROM or game assets are included. Supply your own cartridge.\n",
        encoding="utf-8")


def archive_zip(directory: Path, output: Path, rom_hashes: set[str]) -> None:
    scan_directory(directory, rom_hashes)
    timestamp = time.gmtime(max(archive_epoch(), 315532800))[:6]
    with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for path in sorted(directory.rglob("*")):
            if path.is_file():
                name = path.relative_to(directory.parent).as_posix()
                member = zipfile.ZipInfo(name, date_time=timestamp)
                member.create_system = 3
                member.external_attr = (stat.S_IFREG | archive_mode(name)) << 16
                archive.writestr(member, read_input(path, rom_hashes, directory),
                                 compress_type=zipfile.ZIP_DEFLATED, compresslevel=9)
    scan_artifact(output, rom_hashes)


def archive_tar_gz(directory: Path, output: Path, rom_hashes: set[str]) -> None:
    scan_directory(directory, rom_hashes)
    epoch = archive_epoch()
    with output.open("wb") as stream:
        # GzipFile's empty name suppresses host paths; mtime suppresses the clock.
        with gzip.GzipFile(filename="", mode="wb", fileobj=stream, mtime=epoch) as compressed:
            with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as archive:
                for path in (directory, *sorted(directory.rglob("*"))):
                    name = path.relative_to(directory.parent).as_posix()
                    member = tarfile.TarInfo(name)
                    member.mtime = epoch
                    member.uid = member.gid = 0
                    member.uname = member.gname = ""
                    member.mode = 0o755 if path.is_dir() else archive_mode(name)
                    if path.is_dir():
                        member.type = tarfile.DIRTYPE
                        archive.addfile(member)
                    else:
                        data = read_input(path, rom_hashes, directory)
                        member.size = len(data)
                        archive.addfile(member, io.BytesIO(data))
    scan_artifact(output, rom_hashes)


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
    rom_hashes = known_rom_hashes()
    validate_native_binary(binary, platform, rom_hashes)
    output.mkdir(parents=True, exist_ok=True)
    stem = f"toejam-earl-{version}-{PLATFORMS[platform]}"
    appdir = output / f"{stem}.AppDir"
    if platform == "linux" and (appdir.exists() or appdir.is_symlink()):
        raise ValueError(f"AppDir must not already exist: {appdir}")
    with tempfile.TemporaryDirectory(dir=output) as temporary:
        directory = Path(temporary) / stem
        directory.mkdir()
        copy_public_notes(directory, rom_hashes)
        if platform == "linux":
            stage_linux(binary, directory)
            artifact = output / f"{stem}.tar.gz"
            archive_tar_gz(directory, artifact, rom_hashes)
            # appimagetool consumes this independently of the portable tarball.
            appdir.mkdir()
            stage_linux(binary, appdir)
            copy_public_notes(appdir, rom_hashes)
            appdir.joinpath("AppRun").write_text(
                '#!/bin/sh\nHERE="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"\n'
                'exec "$HERE/toejam-earl" "$@"\n', encoding="utf-8")
            appdir.joinpath("AppRun").chmod(0o755)
            scan_directory(appdir, rom_hashes)
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
            archive_zip(directory, artifact, rom_hashes)
    return artifact


def package_web(version: str, output: Path, site: Path) -> Path:
    # Exact allowlist: never copy the whole web tree (tests, caches or user data).
    rom_hashes = known_rom_hashes()
    if site.is_symlink():
        raise ValueError(f"web output must not be a symlink: {site}")
    site.mkdir(parents=True, exist_ok=True)
    if any(site.iterdir()):
        raise ValueError(f"web output must be empty: {site}")
    for name in WEB_FILES:
        site.joinpath(name).write_bytes(read_input(ROOT / "web" / name, rom_hashes, ROOT))
    package = site / "pkg"
    package.mkdir()
    for name in ("tje_web.js", "tje_web_bg.wasm"):
        data = read_input(ROOT / "web/pkg" / name, rom_hashes, ROOT)
        if name.endswith(".wasm") and not data.startswith(b"\0asm\x01\0\0\0"):
            raise ValueError(f"expected a WebAssembly v1 module: {name}")
        package.joinpath(name).write_bytes(data)
    copy_public_notes(site, rom_hashes)
    site.joinpath(".nojekyll").touch()
    site.joinpath("icon.png").write_bytes(logo_png(256))
    index = site / "index.html"
    index.write_text(index.read_text(encoding="utf-8").replace(
        "</head>", '  <link rel="icon" href="./icon.png" type="image/png">\n</head>'),
        encoding="utf-8")
    output.mkdir(parents=True, exist_ok=True)
    artifact = output / f"toejam-earl-{version}-web.zip"
    archive_zip(site, artifact, rom_hashes)
    return artifact


def checksums(output: Path) -> Path:
    artifacts = sorted(path for path in output.iterdir() if path.is_file()
                       and (path.name.endswith((".tar.gz", ".zip", ".AppImage"))))
    if not artifacts:
        raise ValueError(f"no release artifacts in {output}")
    rom_hashes = known_rom_hashes()
    lines = []
    for artifact in artifacts:
        scan_artifact(artifact, rom_hashes)
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
    sums = subcommands.add_parser("checksums", help="scan final archives/AppImages for ROMs, then hash")
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
