"""ROM-free notice completeness, target selection and stale-policy regressions."""
from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import audit_public
import notices
import release


class NoticeTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.root.joinpath("third-party/texts").mkdir(parents=True)
        self.root.joinpath("tools").mkdir()
        self.root.joinpath("tools/notices.toml").write_text('accepted = ["MIT"]\n', encoding="utf-8")
        self.notice = b"Synthetic public notice for unit-test input\n"
        self.text = b"Synthetic public legal-text input; real texts are checked separately.\n"
        self.key = notices.digest(self.text)
        self.text_path = self.root / f"third-party/texts/{self.key}.txt"
        self.text_path.write_bytes(self.text)
        self.root.joinpath("third-party/linux.txt").write_bytes(self.notice)
        self.receipt = {
            "format": 1, "cargo_about": notices.VERSION, "graph": "linux",
            "targets": list(notices.GRAPHS["linux"][1]), "source_fingerprint": "current",
            "notice_sha256": notices.digest(self.notice), "runtime_texts": {},
            "dependencies": [{"name": "example", "selected": ["MIT"],
                              "texts": {"LICENSE": self.key}}],
        }
        self.save_receipt()
        patch = mock.patch.object(notices, "source_fingerprint", return_value="current")
        patch.start()
        self.addCleanup(patch.stop)

    def save_receipt(self) -> None:
        self.root.joinpath("third-party/linux.json").write_text(json.dumps(self.receipt), encoding="utf-8")

    def test_copies_full_text_and_exact_dependency_receipt(self) -> None:
        files = notices.package_files("linux", self.root)
        self.assertEqual(files["THIRD_PARTY_NOTICES.txt"], self.notice)
        self.assertEqual(files[f"THIRD_PARTY_LICENSES/{self.key}.txt"], self.text)
        self.assertEqual(json.loads(files["THIRD_PARTY_DEPENDENCIES.json"]), self.receipt)

    def test_stale_and_wrong_target_receipts_fail(self) -> None:
        self.receipt["source_fingerprint"] = "old"
        self.save_receipt()
        with self.assertRaisesRegex(ValueError, "stale"):
            notices.package_files("linux", self.root)
        self.receipt["source_fingerprint"] = "current"
        self.receipt["targets"] = ["wasm32-unknown-unknown"]
        self.save_receipt()
        with self.assertRaisesRegex(ValueError, "unsupported"):
            notices.package_files("linux", self.root)

    def test_unaccepted_license_or_missing_selection_fails(self) -> None:
        for selected in (["GPL-2.0-only"], []):
            self.receipt["dependencies"][0]["selected"] = selected
            self.save_receipt()
            with self.assertRaisesRegex(ValueError, "unaccepted"):
                notices.package_files("linux", self.root)

    def test_missing_changed_or_traversing_full_text_fails(self) -> None:
        self.text_path.unlink()
        with self.assertRaises(FileNotFoundError):
            notices.package_files("linux", self.root)
        self.text_path.write_bytes(b"altered")
        with self.assertRaisesRegex(ValueError, "checksum mismatch"):
            notices.package_files("linux", self.root)
        self.receipt["dependencies"][0]["texts"]["LICENSE"] = "../outside"
        self.save_receipt()
        with self.assertRaisesRegex(ValueError, "invalid.*key"):
            notices.package_files("linux", self.root)

    def test_changed_human_notice_fails(self) -> None:
        self.root.joinpath("third-party/linux.txt").write_bytes(b"altered")
        with self.assertRaisesRegex(ValueError, "notice checksum"):
            notices.package_files("linux", self.root)

    def test_packaging_preserves_input_symlink_guard(self) -> None:
        target = self.root / "elsewhere.txt"
        target.write_bytes(self.text)
        self.text_path.unlink()
        self.text_path.symlink_to(target)
        with mock.patch.object(release, "ROOT", self.root):
            with self.assertRaisesRegex(ValueError, "non-symlink"):
                release.copy_third_party_notes(self.root / "staged", "linux", set())


class CommittedNoticeTests(unittest.TestCase):
    def test_legal_reflow_preserves_all_words_without_padding(self) -> None:
        text = ("A public licence clause keeps every original word and punctuation mark even when its lines are reflowed.\n"
                + " " * 70 + "\n    Retain every copyright and licence condition.\n")
        quoted = notices.legal_text(text).decode()
        normalized = "\n".join(line.removeprefix("> ") for line in quoted.splitlines())
        self.assertEqual(normalized.split(), text.split())
        self.assertTrue(all(len(line) <= 60 for line in normalized.splitlines()))

    def test_long_notice_identifiers_soft_wrap_without_losing_characters(self) -> None:
        text = "url:\nhttps://example.invalid/sha512/" + bytes(range(128, 192)).hex() + "/download\n"
        unfolded = ("\n".join("> " + line for line in text.splitlines()) + "\n").encode()
        self.assertTrue(audit_public.content_reasons("LICENSE.txt", unfolded, None))
        quoted = notices.legal_text(text)
        normalized = "\n".join(line.removeprefix("> ") for line in quoted.decode().splitlines())
        self.assertEqual("".join(normalized.split()), "".join(text.split()))
        self.assertTrue(all(len(line) <= 60 for line in normalized.splitlines()))
        self.assertEqual(audit_public.content_reasons("LICENSE.txt", quoted, None), [])

    def test_fingerprint_is_independent_of_host_path_order_and_crlf(self) -> None:
        expected = notices.source_fingerprint()
        canonical_sorted = sorted

        def windows_path_order(values: object) -> list:
            items = list(values)
            if items and isinstance(items[0], Path):
                return canonical_sorted(items, key=lambda path: str(path).casefold())
            return canonical_sorted(items)

        with mock.patch.object(notices, "sorted", side_effect=windows_path_order, create=True):
            self.assertEqual(notices.source_fingerprint(), expected)
        original_read = Path.read_bytes
        with mock.patch.object(Path, "read_bytes", lambda path: original_read(path).replace(
                b"\r\n", b"\n").replace(b"\n", b"\r\n")):
            self.assertEqual(notices.source_fingerprint(), expected)

    def test_actual_target_specific_dependencies_and_full_texts(self) -> None:
        for graph in notices.GRAPHS:
            with self.subTest(graph=graph):
                files = notices.package_files(graph)
                receipt = json.loads(files["THIRD_PARTY_DEPENDENCIES.json"])
                dependencies = {item["name"]: item for item in receipt["dependencies"]}
                self.assertIn("MIT", dependencies["libm"]["selected"])
                libm_text = "\n".join(files[f"THIRD_PARTY_LICENSES/{key}.txt"].decode()
                                      for key in dependencies["libm"]["texts"].values())
                self.assertIn("Permission is hereby granted", libm_text)
                self.assertIn("Copyright", libm_text)
                self.assertGreaterEqual(len(receipt["runtime_texts"]), 3)
                if graph == "web":
                    self.assertNotIn("cpal", dependencies)
                    self.assertNotIn("epaint_default_fonts", dependencies)
                else:
                    self.assertIn("Apache-2.0", dependencies["cpal"]["selected"])
                    self.assertIn("Bitstream-Vera", dependencies["epaint_default_fonts"]["selected"])
                    self.assertIn("Ubuntu-font-1.0", dependencies["epaint_default_fonts"]["selected"])
                if graph == "macos":
                    self.assertEqual(set(receipt["targets"]),
                                     {"x86_64-apple-darwin", "aarch64-apple-darwin"})


if __name__ == "__main__":
    unittest.main()
