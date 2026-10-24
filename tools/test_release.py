"""ROM-free release format and allowlist regressions, not platform run tests."""

from __future__ import annotations

import argparse
import hashlib
import io
import os
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


def executable_image(platform: str) -> bytes:
    """Synthetic headers exercise format validation, not machine-code execution."""
    data = bytearray(512)
    if platform == "linux":
        data[:7] = b"\x7fELF\x02\x01\x01"
        struct.pack_into("<HHI", data, 16, 2, 62, 1)
        struct.pack_into("<H", data, 52, 64)
    elif platform == "windows":
        data[:2] = b"MZ"
        struct.pack_into("<I", data, 60, 128)
        data[128:132] = b"PE\0\0"
        struct.pack_into("<HHIIIHH", data, 132, 0x8664, 0, 0, 0, 0, 112, 2)
        struct.pack_into("<H", data, 152, 0x20B)
    else:
        data[:4] = b"\xca\xfe\xba\xbe"
        struct.pack_into(">I", data, 4, 2)
        for index, cpu in enumerate((0x1000007, 0x100000C)):
            offset = 128 + index * 128
            struct.pack_into(">IIIII", data, 8 + index * 20, cpu, 0, offset, 64, 0)
            data[offset:offset + 4] = b"\xcf\xfa\xed\xfe"
            struct.pack_into("<I", data, offset + 4, cpu)
            struct.pack_into("<I", data, offset + 12, 2)
    return bytes(data)


def rom_signature_payload() -> bytes:
    data = bytearray(512)
    data[0x100:0x104] = b"SEGA"
    return bytes(data)


def appimage_header() -> bytes:
    data = bytearray(executable_image("linux")[:128])
    data[8:11] = b"AI\x02"
    struct.pack_into("<Q", data, 40, 64)
    struct.pack_into("<HH", data, 58, 64, 1)
    return bytes(data) + b"hsqs"


class ReleaseTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.binary = self.root / "synthetic-executable"
        self.binary.write_bytes(executable_image("linux"))
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
        self.assertEqual(icons.joinpath("toejam-earl.rc").read_bytes(),
                         b'1 ICON "toejam-earl.ico"\n')
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
            self.assertIn(f"{stem}/docs/REVISIONS.md", members)
            self.assertIn(f"{stem}/crates/tje-native/README.md", members)
            self.assertIn(f"{stem}/SOURCE.txt", members)
            self.assertIn(f"{stem}/THIRD_PARTY_NOTICES.txt", members)
            self.assertIn(f"{stem}/THIRD_PARTY_DEPENDENCIES.json", members)
            self.assertTrue(any(name.startswith(f"{stem}/THIRD_PARTY_LICENSES/") for name in members))
            self.assertFalse(any("cartridge.bin" in name or "/roms/" in name for name in members))
            self.assertEqual(archive.getmember(f"{stem}/toejam-earl").mode, 0o755)
        appdir = self.output / f"{stem}.AppDir"
        # NTFS does not expose POSIX execute bits; assert the shipped metadata instead.
        appdir_archive = self.root / "appdir.tar.gz"
        release.archive_tar_gz(appdir, appdir_archive, set())
        with tarfile.open(appdir_archive) as archive:
            self.assertEqual(archive.getmember(f"{appdir.name}/AppRun").mode, 0o755)
        self.assertIn('exec "$HERE/toejam-earl" "$@"', appdir.joinpath("AppRun").read_text())
        self.assertIn("Icon=toejam-earl\n", appdir.joinpath("toejam-earl.desktop").read_text())
        self.assertEqual(appdir.joinpath("toejam-earl").read_bytes(), self.binary.read_bytes())
        self.assertTrue(appdir.joinpath("AppRun").read_bytes().startswith(b"#!/bin/sh\n"))
        for name in ("AppRun", "toejam-earl.desktop", "SOURCE.txt"):
            self.assertNotIn(b"\r", appdir.joinpath(name).read_bytes())
        self.assertTrue(appdir.joinpath("crates/tje-native/README.md").is_file())
        self.assertTrue(appdir.joinpath("docs/REVISIONS.md").is_file())
        self.assertTrue(appdir.joinpath("THIRD_PARTY_NOTICES.txt").is_file())
        self.assertTrue(appdir.joinpath("THIRD_PARTY_LICENSES").is_dir())
        # Reusing a dirty staging tree must never pull a ROM into the AppImage.
        appdir.joinpath("cartridge.bin").write_text("must not survive a new staging operation")
        with self.assertRaisesRegex(ValueError, "AppDir must not already exist"):
            release.package_native("linux", self.binary, "v1.2.3", self.output)

    def test_windows_zip_contains_only_expected_portable_inputs(self) -> None:
        self.binary.write_bytes(executable_image("windows"))
        artifact = release.package_native("windows", self.binary, "manual-abc", self.output)
        stem = "toejam-earl-manual-abc-windows-x86_64-gnu"
        with zipfile.ZipFile(artifact) as archive:
            names = archive.namelist()
            self.assertIn(f"{stem}/toejam-earl.exe", names)
            self.assertIn(f"{stem}/icons/toejam-earl.ico", names)
            self.assertIn(f"{stem}/crates/tje-native/README.md", names)
            self.assertIn(f"{stem}/docs/REVISIONS.md", names)
            self.assertIn(f"{stem}/THIRD_PARTY_NOTICES.txt", names)
            self.assertIn(f"{stem}/THIRD_PARTY_DEPENDENCIES.json", names)
            self.assertTrue(any(name.startswith(f"{stem}/THIRD_PARTY_LICENSES/") for name in names))
            self.assertFalse(any(name.endswith((".rc", ".icns", ".o")) for name in names))
            self.assertEqual(archive.read(f"{stem}/toejam-earl.exe"), self.binary.read_bytes())
            self.assertIn(b"Commit: ", archive.read(f"{stem}/SOURCE.txt"))
            self.assertNotIn(b"\r", archive.read(f"{stem}/SOURCE.txt"))

    def test_macos_metadata_architecture_and_signing_contract(self) -> None:
        self.binary.write_bytes(executable_image("macos"))
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
        # Check Unix modes in the ZIP, not the host filesystem's limited Windows modes.
        artifact = self.root / "macos.zip"
        release.archive_zip(destination, artifact, set())
        with zipfile.ZipFile(artifact) as archive:
            member = archive.getinfo(executable.relative_to(destination.parent).as_posix())
            self.assertEqual((member.external_attr >> 16) & 0o777, 0o755)
        self.assertEqual(run.call_args_list, [
            mock.call(["lipo", "-verify_arch", "x86_64", "arm64", str(executable)], check=True),
            mock.call(["codesign", "--force", "--sign", "-", str(bundle)], check=True),
            mock.call(["codesign", "--verify", "--deep", "--strict", str(bundle)], check=True),
        ])

    def test_macos_package_includes_universal_target_legal_payload(self) -> None:
        self.binary.write_bytes(executable_image("macos"))
        with mock.patch.object(release.subprocess, "run"):
            artifact = release.package_native("macos", self.binary, "v1.2.3", self.output)
        stem = "toejam-earl-v1.2.3-macos-universal"
        with zipfile.ZipFile(artifact) as archive:
            self.assertIn(f"{stem}/THIRD_PARTY_NOTICES.txt", archive.namelist())
            resources = f"{stem}/ToeJam & Earl.app/Contents/Resources"
            self.assertIn(f"{resources}/THIRD_PARTY_NOTICES.txt", archive.namelist())
            self.assertIn(f"{resources}/THIRD_PARTY_DEPENDENCIES.json", archive.namelist())
            self.assertIn(f"{resources}/LICENSE", archive.namelist())
            receipt = archive.read(f"{stem}/THIRD_PARTY_DEPENDENCIES.json")
            self.assertIn(b"x86_64-apple-darwin", receipt)
            self.assertIn(b"aarch64-apple-darwin", receipt)

    def test_web_allowlist_excludes_tests_caches_and_roms(self) -> None:
        source = self.root / "source"
        web = source / "web"
        web.joinpath("pkg").mkdir(parents=True)
        for name in release.WEB_FILES:
            web.joinpath(name).write_text("</head><footer>Source</footer>" if name == "index.html" else "public runtime")
        web.joinpath("pkg", "tje_web.js").write_text(
            "import { load } from './snippets/engine/src/browser.js';\n")
        snippets = web / "pkg/snippets/engine/src"
        snippets.mkdir(parents=True)
        snippets.joinpath("browser.js").write_text("import * as shared from './cache.js'\n")
        snippets.joinpath("cache.js").write_text("export const read = 1;\n")
        snippets.joinpath("unreferenced.js").write_text("must never be copied")
        web.joinpath("pkg", "tje_web_bg.wasm").write_bytes(b"\0asm\x01\0\0\0")
        for name in ("tests.mjs", "cartridge.bin", "private-save.json"):
            web.joinpath(name).write_text("must never be copied")
        web.joinpath("pkg", "unexpected.bin").write_text("must never be copied")
        site = self.root / "site"
        legal = {"THIRD_PARTY_NOTICES.txt": b"unit-test notice\n",
                 "THIRD_PARTY_LICENSES/example.txt": b"unit-test full legal text\n",
                 "THIRD_PARTY_DEPENDENCIES.json": b'{"graph":"web"}\n'}
        with (mock.patch.object(release, "ROOT", source),
              mock.patch.object(release, "copy_public_notes"),
              mock.patch.object(release.notices, "package_files", return_value=legal)):
            artifact = release.package_web("v1.2.3", self.output, site)
        expected = set(release.WEB_FILES) | {
            "pkg/tje_web.js", "pkg/tje_web_bg.wasm", ".nojekyll", "icon.png",
            "pkg/snippets/engine/src/browser.js", "pkg/snippets/engine/src/cache.js",
            *legal.keys(),
        }
        self.assertEqual({path.relative_to(site).as_posix() for path in site.rglob("*") if path.is_file()},
                         expected)
        self.assertIn('href="./icon.png"', site.joinpath("index.html").read_text())
        self.assertIn('href="./THIRD_PARTY_NOTICES.txt"', site.joinpath("index.html").read_text())
        self.assertNotIn(b"\r", site.joinpath("index.html").read_bytes())
        for name, content in legal.items():
            self.assertEqual(site.joinpath(name).read_bytes(), content)
        with zipfile.ZipFile(artifact) as archive:
            self.assertEqual(set(archive.namelist()), {f"site/{name}" for name in expected})
        with self.assertRaisesRegex(ValueError, "web output must be empty"):
            release.package_web("v1.2.3", self.output, site)

    def test_web_snippet_imports_keep_path_symlink_and_rom_guards(self) -> None:
        source = self.root / "source"
        snippets = source / "web/pkg/snippets/engine"
        snippets.mkdir(parents=True)
        snippet = snippets / "loader.js"
        loader = b"import { load } from './snippets/engine/loader.js';"
        with mock.patch.object(release, "ROOT", source):
            for specifier in ("https://example.org/code.js", "../../private.js",
                              "./snippets/../../private.js", "./snippets/cache.bin"):
                with self.subTest(specifier=specifier):
                    with self.assertRaises(ValueError):
                        release.referenced_web_snippets(
                            f"import {{ load }} from '{specifier}';".encode(), set())
            private = self.root / "private.js"
            private.write_text("private")
            snippet.symlink_to(private)
            with self.assertRaisesRegex(ValueError, "non-symlink"):
                release.referenced_web_snippets(loader, set())
            snippet.unlink()
            snippet.write_bytes(rom_signature_payload())
            with self.assertRaisesRegex(ValueError, "Genesis ROM signature"):
                release.referenced_web_snippets(loader, set())

    def test_checksums_cover_only_final_artifacts(self) -> None:
        self.output.mkdir()
        stage = self.root / "public-stage"
        stage.mkdir()
        stage.joinpath("public.txt").write_text("public archive input")
        release.archive_tar_gz(stage, self.output / "a.tar.gz", set())
        release.archive_zip(stage, self.output / "b.zip", set())
        self.output.joinpath("not-an-artifact.txt").write_text("exclude")
        self.output.joinpath("dirty.AppDir").mkdir()
        expected = "".join(
            f"{hashlib.sha256(self.output.joinpath(name).read_bytes()).hexdigest()}  {name}\n"
            for name in ("a.tar.gz", "b.zip"))
        manifest = release.checksums(self.output)
        self.assertEqual(manifest.read_text(), expected)
        self.assertNotIn(b"\r", manifest.read_bytes())
        self.assertEqual(release.checksums(self.output).read_bytes(), manifest.read_bytes())

    def test_missing_executable_empty_artifacts_and_unsafe_versions_fail(self) -> None:
        with self.assertRaisesRegex(ValueError, "regular, non-symlink release input"):
            release.package_native("linux", self.root / "missing", "local", self.output)
        self.output.mkdir()
        with self.assertRaisesRegex(ValueError, "no release artifacts"):
            release.checksums(self.output)
        for value in ("", "../escape", "v1/other", "v1;command", "v1\nforged-row"):
            with self.assertRaises(argparse.ArgumentTypeError):
                release.release_version(value)
        self.assertEqual(release.release_version("v1.2.3-rc.1"), "v1.2.3-rc.1")

    def test_native_archives_are_reproducible_with_canonical_metadata(self) -> None:
        original_copy = release.copy_public_notes
        for platform in ("linux", "windows"):
            with self.subTest(platform=platform):
                self.binary.write_bytes(executable_image(platform))
                digests = []
                for index, timestamp in enumerate((650000000, 1700000000)):
                    def copy_with_metadata(destination: Path, hashes: set[str]) -> None:
                        original_copy(destination, hashes)
                        for path in destination.rglob("*"):
                            os.utime(path, (timestamp, timestamp))
                            path.chmod(0o777 if index == 0 else 0o700)

                    with (mock.patch.dict(os.environ, {"SOURCE_DATE_EPOCH": "946684800"}),
                          mock.patch.object(release, "copy_public_notes",
                                            side_effect=copy_with_metadata)):
                        artifact = release.package_native(
                            platform, self.binary, "v1.2.3", self.root / f"{platform}-{index}")
                    digests.append(hashlib.sha256(artifact.read_bytes()).digest())
                self.assertEqual(*digests)
                if platform == "linux":
                    header = artifact.read_bytes()[:10]
                    self.assertEqual(header[3], 0)  # No original filename in gzip.
                    self.assertEqual(struct.unpack_from("<I", header, 4)[0], 946684800)
                    with tarfile.open(artifact) as archive:
                        for member in archive:
                            self.assertEqual((member.uid, member.gid, member.uname, member.gname),
                                             (0, 0, "", ""))
                            self.assertEqual(member.mtime, 946684800)
                            self.assertEqual(member.mode, 0o755 if member.isdir() else
                                             release.archive_mode(member.name))
                else:
                    with zipfile.ZipFile(artifact) as archive:
                        for member in archive.infolist():
                            self.assertEqual(member.date_time, (2000, 1, 1, 0, 0, 0))
                            self.assertEqual(member.extra, b"")
                            self.assertEqual(member.create_system, 3)
                            self.assertEqual((member.external_attr >> 16) & 0o777,
                                             release.archive_mode(member.filename))

    def test_archive_epoch_defaults_and_zip_epoch_clamping(self) -> None:
        with mock.patch.dict(os.environ, {}, clear=True):
            self.assertEqual(release.archive_epoch(), 0)
        for value in ("-1", "4294967296", "not-a-timestamp"):
            with mock.patch.dict(os.environ, {"SOURCE_DATE_EPOCH": value}):
                with self.assertRaises(ValueError):
                    release.archive_epoch()
        stage = self.root / "early"
        stage.mkdir()
        stage.joinpath("public.txt").write_text("public")
        artifact = self.root / "early.zip"
        with mock.patch.dict(os.environ, {"SOURCE_DATE_EPOCH": "0"}):
            release.archive_zip(stage, artifact, set())
        with zipfile.ZipFile(artifact) as archive:
            self.assertEqual(archive.infolist()[0].date_time, (1980, 1, 1, 0, 0, 0))

    def test_native_format_and_architecture_validation(self) -> None:
        for platform in ("linux", "windows", "macos"):
            self.binary.write_bytes(executable_image(platform))
            release.validate_native_binary(self.binary, platform, set())
        invalid = [
            ("linux", b"not an executable"),
            ("linux", executable_image("windows")),
            ("windows", executable_image("linux")),
            ("windows", b"MZ" + bytes(62)),
            ("macos", executable_image("linux")),
        ]
        wrong_elf = bytearray(executable_image("linux"))
        struct.pack_into("<H", wrong_elf, 18, 183)  # AArch64 is not the Linux release target.
        invalid.append(("linux", bytes(wrong_elf)))
        wrong_pe = bytearray(executable_image("windows"))
        struct.pack_into("<H", wrong_pe, 132, 0xAA64)
        invalid.append(("windows", bytes(wrong_pe)))
        dll = bytearray(executable_image("windows"))
        struct.pack_into("<H", dll, 150, 0x2002)
        invalid.append(("windows", bytes(dll)))
        missing_slice = bytearray(executable_image("macos"))
        struct.pack_into(">I", missing_slice, 4, 1)
        invalid.append(("macos", bytes(missing_slice)))
        for platform, data in invalid:
            with self.subTest(platform=platform, header=data[:4]):
                self.binary.write_bytes(data)
                with self.assertRaisesRegex(ValueError, "expected a .* executable"):
                    release.package_native(platform, self.binary, "local", self.output)

    def test_renamed_rom_and_private_hash_match_are_rejected(self) -> None:
        self.binary.write_bytes(rom_signature_payload())
        for platform in release.PLATFORMS:
            with self.assertRaisesRegex(ValueError, "Genesis ROM signature"):
                release.package_native(platform, self.binary, "local", self.output)
        source = self.root / "source"
        source.joinpath("roms").mkdir(parents=True)
        payload = executable_image("linux")  # No SEGA header; rejection must use the private hash.
        source.joinpath("roms/rev02.bin").write_bytes(payload)
        self.binary.write_bytes(payload)
        with mock.patch.object(release, "ROOT", source):
            with self.assertRaisesRegex(ValueError, "known ROM hash"):
                release.package_native("linux", self.binary, "local", self.output)

    def test_web_rejects_wasm_symlinks_parent_symlinks_and_invalid_magic(self) -> None:
        source = self.root / "source"
        web = source / "web"
        package = web / "pkg"
        package.mkdir(parents=True)
        for name in release.WEB_FILES:
            web.joinpath(name).write_text("</head>" if name == "index.html" else "public")
        package.joinpath("tje_web.js").write_text("public JavaScript")
        private = self.root / "private.bin"
        private.write_bytes(rom_signature_payload())
        wasm = package / "tje_web_bg.wasm"
        wasm.symlink_to(private)
        with mock.patch.object(release, "ROOT", source):
            with self.assertRaisesRegex(ValueError, "non-symlink"):
                release.package_web("local", self.output, self.root / "symlink-site")
            wasm.unlink()
            wasm.write_bytes(b"not a WebAssembly module")
            with self.assertRaisesRegex(ValueError, "WebAssembly v1"):
                release.package_web("local", self.output, self.root / "invalid-site")
            wasm.write_bytes(rom_signature_payload())
            with self.assertRaisesRegex(ValueError, "Genesis ROM signature"):
                release.package_web("local", self.output, self.root / "rom-site")
            wasm.write_bytes(b"\0asm\x01\0\0\0")
            package.rename(web / "original-pkg")
            package.symlink_to(web / "original-pkg", target_is_directory=True)
            with self.assertRaisesRegex(ValueError, "non-symlink"):
                release.package_web("local", self.output, self.root / "parent-symlink-site")

    def test_final_archive_scan_rejects_rom_members_and_known_hashes(self) -> None:
        self.output.mkdir()
        for suffix in (".zip", ".tar.gz"):
            for use_hash in (False, True):
                payload = b"private reference without a cartridge header" if use_hash else rom_signature_payload()
                hashes = {hashlib.sha1(payload).hexdigest()} if use_hash else set()
                artifact = self.output / f"polluted{suffix}"
                if suffix == ".zip":
                    with zipfile.ZipFile(artifact, "w") as archive:
                        archive.writestr("renamed-resource.dat", payload)
                else:
                    with tarfile.open(artifact, "w:gz") as archive:
                        member = tarfile.TarInfo("renamed-resource.dat")
                        member.size = len(payload)
                        archive.addfile(member, io.BytesIO(payload))
                with self.assertRaisesRegex(ValueError, "known ROM hash" if use_hash else
                                            "Genesis ROM signature"):
                    release.scan_artifact(artifact, hashes)
        with zipfile.ZipFile(self.output / "polluted.zip", "w") as archive:
            archive.writestr("renamed-resource.dat", rom_signature_payload())
        with self.assertRaisesRegex(ValueError, "Genesis ROM signature"):
            release.checksums(self.output)
        self.assertFalse(self.output.joinpath("SHA256SUMS.txt").exists())

    def test_appimage_scan_reads_members_without_executing_the_image(self) -> None:
        artifact = self.root / "image.AppImage"
        artifact.write_bytes(appimage_header())
        listing = ("drwxr-xr-x 0/0 10 2020-01-01 00:00 squashfs-root\n"
                   "-rw-r--r-- 0/0 6 2020-01-01 00:00 squashfs-root/public.txt\n"
                   "lrwxrwxrwx 0/0 10 2020-01-01 00:00 squashfs-root/link -> public.txt\n")
        with mock.patch.object(release.subprocess, "check_output",
                               side_effect=[listing, b"public"]) as run:
            release.scan_artifact(artifact, set())
        self.assertTrue(all(call.args[0][0] == "unsquashfs" for call in run.call_args_list))
        polluted = listing.replace(" 6 ", " 512 ")
        with mock.patch.object(release.subprocess, "check_output",
                               side_effect=[polluted, rom_signature_payload()]):
            with self.assertRaisesRegex(ValueError, "Genesis ROM signature"):
                release.scan_artifact(artifact, set())
        external = listing.replace("link -> public.txt", "link -> /private/cartridge.bin")
        with mock.patch.object(release.subprocess, "check_output", return_value=external):
            with self.assertRaisesRegex(ValueError, "external or cyclic"):
                release.scan_artifact(artifact, set())


if __name__ == "__main__":
    unittest.main()
