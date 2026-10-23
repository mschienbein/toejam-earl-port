"""ROM-free release format and allowlist regressions, not platform run tests."""

from __future__ import annotations

import argparse
import hashlib
import plistlib
import struct
import tarfile
import tempfile
import unittest
import zipfile
import zlib
from pathlib import Path
from unittest import mock

import release


class ReleaseTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.binary = self.root / "synthetic-executable"
        self.binary.write_text("synthetic packaging input, never executed\n", encoding="utf-8")
        self.output = self.root / "dist"

    def test_png_chunks_dimensions_and_original_lettering(self) -> None:
        image = release.logo_png(256)
        self.assertEqual(image[:8], b"\x89PNG\r\n\x1a\n")
        offset = 8
        compressed = bytearray()
        while offset < len(image):
            length = struct.unpack_from(">I", image, offset)[0]
            kind = image[offset + 4:offset + 8]
            payload = image[offset + 8:offset + 8 + length]
            crc = struct.unpack_from(">I", image, offset + 8 + length)[0]
            self.assertEqual(crc, zlib.crc32(kind + payload))
            if kind == b"IHDR":
                self.assertEqual(struct.unpack(">IIBBBBB", payload), (256, 256, 8, 6, 0, 0, 0))
            elif kind == b"IDAT":
                compressed.extend(payload)
            offset += length + 12
        rows = zlib.decompress(compressed)
        self.assertEqual(len(rows), 256 * (1 + 256 * 4))
        self.assertEqual(rows[0], 0)  # Every row uses the unfiltered PNG format.
        self.assertEqual(image, release.logo_png(256))
        self.assertGreater(len(set(rows)), 3)

    def test_ico_and_icns_contain_complete_png_images(self) -> None:
        icons = self.root / "icons"
        release.make_icons(icons)
        ico = icons.joinpath("toejam-earl.ico").read_bytes()
        self.assertEqual(struct.unpack_from("<HHH", ico), (0, 1, 6))
        for index, size in enumerate((16, 32, 48, 64, 128, 256)):
            width, height, _, _, planes, depth, length, offset = struct.unpack_from(
                "<BBBBHHII", ico, 6 + 16 * index)
            self.assertEqual((width or 256, height or 256, planes, depth), (size, size, 1, 32))
            self.assertEqual(ico[offset:offset + length], release.logo_png(size))
        icns = icons.joinpath("toejam-earl.icns").read_bytes()
        self.assertEqual(icns[:4], b"icns")
        self.assertEqual(struct.unpack_from(">I", icns, 4)[0], len(icns))
        offset = 8
        for kind, size in ((b"ic07", 128), (b"ic08", 256), (b"ic09", 512), (b"ic10", 1024)):
            self.assertEqual(icns[offset:offset + 4], kind)
            length = struct.unpack_from(">I", icns, offset + 4)[0]
            self.assertEqual(icns[offset + 8:offset + length], release.logo_png(size))
            offset += length
        self.assertEqual(offset, len(icns))

    def test_linux_tarball_and_appdir_are_explicit_public_inputs(self) -> None:
        self.binary.parent.joinpath("cartridge.bin").write_text("must never be copied")
        artifact = release.package_native("linux", self.binary, "v1.2.3", self.output)
        stem = "toejam-earl-v1.2.3-linux-x86_64"
        with tarfile.open(artifact) as archive:
            members = archive.getnames()
            self.assertIn(f"{stem}/toejam-earl", members)
            self.assertIn(f"{stem}/README.md", members)
            self.assertIn(f"{stem}/LICENSE", members)
            self.assertIn(f"{stem}/docs/RELEASING.md", members)
            self.assertIn(f"{stem}/SOURCE.txt", members)
            self.assertFalse(any("cartridge.bin" in name or "/roms/" in name for name in members))
            self.assertEqual(archive.getmember(f"{stem}/toejam-earl").mode, 0o755)
        appdir = self.output / f"{stem}.AppDir"
        self.assertTrue(appdir.joinpath("AppRun").stat().st_mode & 0o111)
        self.assertIn('exec "$HERE/toejam-earl" "$@"', appdir.joinpath("AppRun").read_text())
        self.assertIn("Icon=toejam-earl\n", appdir.joinpath("toejam-earl.desktop").read_text())
        self.assertEqual(appdir.joinpath("toejam-earl").read_bytes(), self.binary.read_bytes())
        # Reusing a dirty staging tree must never pull a ROM into the AppImage.
        appdir.joinpath("cartridge.bin").write_text("must not survive a new staging operation")
        with self.assertRaisesRegex(ValueError, "AppDir must not already exist"):
            release.package_native("linux", self.binary, "v1.2.3", self.output)

    def test_windows_zip_contains_only_expected_portable_inputs(self) -> None:
        artifact = release.package_native("windows", self.binary, "manual-abc", self.output)
        stem = "toejam-earl-manual-abc-windows-x86_64-gnu"
        with zipfile.ZipFile(artifact) as archive:
            names = archive.namelist()
            self.assertIn(f"{stem}/toejam-earl.exe", names)
            self.assertIn(f"{stem}/icons/toejam-earl.ico", names)
            self.assertFalse(any(name.endswith((".rc", ".icns", ".o")) for name in names))
            self.assertEqual(archive.read(f"{stem}/toejam-earl.exe"), self.binary.read_bytes())
            self.assertIn(b"Commit: ", archive.read(f"{stem}/SOURCE.txt"))

    def test_macos_metadata_architecture_and_signing_contract(self) -> None:
        destination = self.root / "macos"
        with mock.patch.object(release.subprocess, "run") as run:
            release.stage_macos(self.binary, destination, "v1.2.3-rc1")
        bundle = destination / "ToeJam & Earl.app"
        with bundle.joinpath("Contents/Info.plist").open("rb") as stream:
            metadata = plistlib.load(stream)
        self.assertEqual(metadata["CFBundleIdentifier"], release.APP_ID)
        self.assertEqual(metadata["CFBundleExecutable"], "toejam-earl")
        self.assertEqual(metadata["CFBundleShortVersionString"], "1.2.3")
        self.assertEqual(metadata["LSMinimumSystemVersion"], "11.0")
        executable = bundle / "Contents/MacOS/toejam-earl"
        self.assertEqual(executable.stat().st_mode & 0o777, 0o755)
        self.assertEqual(run.call_args_list, [
            mock.call(["lipo", "-verify_arch", "x86_64", "arm64", str(executable)], check=True),
            mock.call(["codesign", "--force", "--sign", "-", str(bundle)], check=True),
            mock.call(["codesign", "--verify", "--deep", "--strict", str(bundle)], check=True),
        ])

    def test_web_allowlist_excludes_tests_caches_and_roms(self) -> None:
        source = self.root / "source"
        web = source / "web"
        web.joinpath("pkg").mkdir(parents=True)
        for name in release.WEB_FILES:
            web.joinpath(name).write_text("</head>" if name == "index.html" else "public runtime")
        for name in ("tje_web.js", "tje_web_bg.wasm"):
            web.joinpath("pkg", name).write_text("synthetic generated runtime")
        for name in ("tests.mjs", "cartridge.bin", "private-save.json"):
            web.joinpath(name).write_text("must never be copied")
        web.joinpath("pkg", "unexpected.bin").write_text("must never be copied")
        site = self.root / "site"
        with (mock.patch.object(release, "ROOT", source),
              mock.patch.object(release, "copy_public_notes")):
            artifact = release.package_web("v1.2.3", self.output, site)
        expected = set(release.WEB_FILES) | {
            "pkg/tje_web.js", "pkg/tje_web_bg.wasm", ".nojekyll", "icon.png",
        }
        self.assertEqual({str(path.relative_to(site)) for path in site.rglob("*") if path.is_file()},
                         expected)
        self.assertIn('href="./icon.png"', site.joinpath("index.html").read_text())
        with zipfile.ZipFile(artifact) as archive:
            self.assertEqual(set(archive.namelist()), {f"site/{name}" for name in expected})
        with self.assertRaisesRegex(ValueError, "web output must be empty"):
            release.package_web("v1.2.3", self.output, site)

    def test_checksums_cover_only_final_artifacts(self) -> None:
        self.output.mkdir()
        payload = b"synthetic archive input"
        for name in ("a.tar.gz", "b.zip", "c.AppImage"):
            self.output.joinpath(name).write_bytes(payload)
        self.output.joinpath("not-an-artifact.txt").write_text("exclude")
        self.output.joinpath("dirty.AppDir").mkdir()
        digest = hashlib.sha256(payload).hexdigest()
        manifest = release.checksums(self.output)
        self.assertEqual(manifest.read_text(), "".join(
            f"{digest}  {name}\n" for name in ("a.tar.gz", "b.zip", "c.AppImage")))
        self.assertEqual(release.checksums(self.output).read_bytes(), manifest.read_bytes())

    def test_missing_executable_empty_artifacts_and_unsafe_versions_fail(self) -> None:
        with self.assertRaisesRegex(ValueError, "regular release executable"):
            release.package_native("linux", self.root / "missing", "local", self.output)
        self.output.mkdir()
        with self.assertRaisesRegex(ValueError, "no release artifacts"):
            release.checksums(self.output)
        for value in ("", "../escape", "v1/other", "v1;command", "v1\nforged-row"):
            with self.assertRaises(argparse.ArgumentTypeError):
                release.release_version(value)
        self.assertEqual(release.release_version("v1.2.3-rc.1"), "v1.2.3-rc.1")


if __name__ == "__main__":
    unittest.main()
