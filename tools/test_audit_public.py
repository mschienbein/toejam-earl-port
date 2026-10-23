"""ROM-free publication-gate regressions; all planted bytes are generated, not game data.

Run with: python -m unittest discover -s tools -p 'test_*.py' -v
The optional real-ROM content test reads private inputs only into memory. No real
ROM bytes are written into a fixture, Git index, commit or compiled artifact.
"""
from __future__ import annotations

import base64
import hashlib
import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import audit_public as audit


OFFSET = 0x3400
WINDOW_SIZE = 96
BENIGN = b"// Synthetic benign fixture.\n"
STATES = ("worktree", "staged", "history")


def synthetic_rom(seed: int) -> bytes:
    image = bytearray(hashlib.shake_256(f"audit fixture {seed}".encode()).digest(0x4000))
    # Deliberately low entropy: deterministic binary length, not entropy, must reject it.
    image[OFFSET:OFFSET + WINDOW_SIZE] = bytes((index * 5 + seed) % 19 for index in range(WINDOW_SIZE))
    return bytes(image)


IMAGES = [synthetic_rom(2), synthetic_rom(7)]


def declaration(expression: bytes) -> bytes:
    if expression.startswith(b"["):
        expression = b"&" + expression
    return b"const DATA: &[u8] = " + expression + b";\n"


def required_forms(window: bytes) -> dict[str, tuple[str, bytes]]:
    escapes = b"".join(f"\\x{value:02x}".encode() for value in window)
    return {
        "raw": ("txt", window),
        "hex": ("txt", window.hex().encode()),
        "numeric-array": ("txt", b"[" + b", ".join(str(value).encode() for value in window) + b"]"),
        "base64": ("txt", base64.b64encode(window)),
        "json": ("json", json.dumps(window.decode("latin1"), ensure_ascii=True).encode()),
        "rust-escape": ("rs", declaration(b'b"' + escapes + b'"')),
        "rust-byte-array": ("rs", declaration(b"[" + b", ".join(f"0x{value:02x}u8".encode() for value in window) + b"]")),
    }


def rust_atom_forms(window: bytes) -> dict[str, tuple[str, bytes]]:
    atoms = {
        "byte-character": [f"b'\\x{value:02x}'".encode() for value in window],
        "one-byte-string": [f'b"\\x{value:02x}"'.encode() for value in window],
        "underscored-suffix": [f"0x{value:02x}_u8".encode() for value in window],
        "byte-casts": [f"0x{value:02x} as i8 as u8".encode() for value in window],
        "parenthesized-casts": [f"({value + 256}u16) as u8".encode() for value in window],
    }
    forms = {name: ("rs", declaration(b"[" + b", ".join(values) + b"]")) for name, values in atoms.items()}
    forms["one-byte-string"] = ("rs", b"const DATA: &[&[u8]] = &[" + b", ".join(atoms["one-byte-string"]) + b"];\n")
    return forms


def git_command(repo: Path, *args: str, data: bytes | None = None) -> bytes:
    result = subprocess.run(
        ["git", "-C", str(repo), "-c", "user.name=Audit Fixture", "-c", "user.email=audit@example.invalid", *args],
        input=data, capture_output=True, check=True,
    )
    return result.stdout


def initialize_repo(path: Path) -> None:
    subprocess.run(["git", "init", "--object-format=sha1", "-b", "main", str(path)], capture_output=True, check=True)


def put(repo: Path, path: str, data: bytes) -> None:
    target = repo / path
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(data)


class GitAuditCase(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="tje-audit-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.repo = self.root / "repo"
        initialize_repo(self.repo)
        put(self.repo, "README.md", b"# Synthetic audit fixture\n")
        self.git("add", "README.md")
        self.git("commit", "-m", "Synthetic initial fixture")

    def git(self, *args: str, data: bytes | None = None) -> bytes:
        return git_command(self.repo, *args, data=data)

    def inspect(self, private: bool, *, history: bool = True) -> list[audit.Finding]:
        with mock.patch.object(audit, "reference_images", return_value=IMAGES) as references:
            findings, summary = audit.audit(self.repo, history=history, no_rom=not private)
            self.assertEqual(summary["rom_check"], private)
            self.assertEqual(references.call_count, int(private))
            return findings

    def matrix(self, forms: dict[str, tuple[str, bytes]]) -> list[tuple[str, str]]:
        cases = []
        for state in STATES:
            for label, (suffix, payload) in forms.items():
                path = f"cases/{state}/{label}.{suffix}"
                put(self.repo, path, payload if state == "history" else BENIGN)
                cases.append((state, path))
        self.git("add", "cases")
        self.git("commit", "-m", "Synthetic historical payloads")
        for state, path in cases:
            if state == "history":
                (self.repo / path).unlink()
        self.git("add", "-u", "cases")
        self.git("commit", "-m", "Remove synthetic historical payloads")
        for state, path in cases:
            label = Path(path).stem
            if state != "history":
                put(self.repo, path, forms[label][1])
                if state == "staged":
                    self.git("add", path)
                    put(self.repo, path, BENIGN)
        return cases

    def assert_matrix(self, cases: list[tuple[str, str]], private: bool) -> None:
        findings = self.inspect(private)
        for state, path in cases:
            with self.subTest(private=private, state=state, path=path):
                matches = [finding for finding in findings if finding.path == path]
                if state == "history":
                    matches = [finding for finding in matches if finding.source.startswith("tree:")]
                else:
                    source = "index:0" if state == "staged" else "worktree"
                    matches = [finding for finding in matches if finding.source == source]
                self.assertTrue(matches, (state, path, private))


class EncodingGitStateTests(GitAuditCase):
    def test_reviewer_required_42_cases_in_both_modes(self) -> None:
        forms = {}
        for revision, image in zip(("rev02", "rev00"), IMAGES):
            window = image[OFFSET:OFFSET + WINDOW_SIZE]
            forms.update({f"{revision}-{name}": value for name, value in required_forms(window).items()})
        cases = self.matrix(forms)
        self.assertEqual(len(cases), 42)
        for private in (False, True):
            self.assert_matrix(cases, private)

    def test_rust_atom_variants_both_revisions_and_all_states(self) -> None:
        forms = {}
        for revision, image in zip(("rev02", "rev00"), IMAGES):
            window = image[OFFSET:OFFSET + WINDOW_SIZE]
            forms.update({f"{revision}-{name}": value for name, value in rust_atom_forms(window).items()})
        cases = self.matrix(forms)
        self.assertEqual(len(cases), 30)
        for private in (False, True):
            self.assert_matrix(cases, private)

    def test_cli_no_rom_fails_on_decoded_binary(self) -> None:
        put(self.repo, "payload.txt", BENIGN)
        self.git("add", "payload.txt")
        self.git("commit", "-m", "Synthetic tracked payload")
        put(self.repo, "payload.txt", IMAGES[0][OFFSET:OFFSET + WINDOW_SIZE].hex().encode())
        result = subprocess.run(
            [sys.executable, str(Path(audit.__file__)), "--repo", str(self.repo), "--history", "--no-rom"],
            capture_output=True, text=True,
        )
        self.assertEqual(result.returncode, 1)
        self.assertIn("decoded non-text payload", result.stdout)

    def test_every_conflict_index_stage_is_audited(self) -> None:
        payload = rust_atom_forms(IMAGES[0][OFFSET:OFFSET + WINDOW_SIZE])["byte-character"][1]
        oid = self.git("hash-object", "-w", "--stdin", data=payload).strip().decode()
        entries = "".join(f"100644 {oid} {stage}\tconflict.rs\n" for stage in (1, 2, 3))
        self.git("update-index", "--index-info", data=entries.encode())
        for private in (False, True):
            findings = self.inspect(private)
            self.assertTrue(all(any(f.path == "conflict.rs" and f.source == f"index:{stage}" for f in findings) for stage in (1, 2, 3)))


class DecoderTests(unittest.TestCase):
    def test_binary_length_guard_does_not_depend_on_entropy(self) -> None:
        for size in (64, 96, 192):
            binary = bytes(size)
            for form in (binary.hex().encode(), json.dumps(binary.decode()).encode(), declaration(b'b"' + b"\\0" * size + b'"')):
                with self.subTest(size=size, form_length=len(form)):
                    self.assertTrue(audit.content_reasons("fixture.rs", form, None))

    def test_wrapped_low_entropy_base64_remains_binary_data_not_prose(self) -> None:
        encoded = base64.b64encode(bytes(WINDOW_SIZE))
        for width in (4, 16, 64):
            wrapped = b"\n".join(encoded[position:position + width] for position in range(0, len(encoded), width))
            self.assertTrue(audit.content_reasons("fixture.txt", wrapped, None))

    def test_byte_casts_preserve_rust_low_byte_semantics(self) -> None:
        for token, expected in ((b"0xff_u8", 255), (b"-1i8 as u8", 255),
                                (b"257u16) as u8", 1), (b"255 as i8", 255), (b"-257 as u8", 255)):
            self.assertEqual(audit.literal_values(token)[1], bytes([expected]), token)

    def test_rust_hex_escape_is_exactly_two_digits_and_nul_is_one_escape(self) -> None:
        self.assertEqual(audit.rust_byte_string(b'b"\\x12face"'), bytes([18]) + b"face")
        self.assertEqual(audit.rust_byte_string(b'b"\\01"'), bytes(1) + b"1")
        self.assertEqual(audit.rust_byte_string(b'br##"\\x12"##'), b"\\x12")
        for token in (b'b"\\x0"', b'b"\\q"'):
            with self.assertRaises(audit.DecodeLimitError):
                audit.rust_byte_string(token)

    def test_python_bytes_literals_decode_without_compiling_regex_snippets(self) -> None:
        cases = (
            (r'b"\x12face"', bytes([18]) + b"face"),
            (r'rb"\x12"', br"\x12"),
            (r"b'\101\0'", b"A" + bytes(1)),
            (r'b"\s"', br"\s"),
            (r'b"\u1234"', br"\u1234"),
            ('b"""line\r\nnext"""', b"line\nnext"),
        )
        for token, expected in cases:
            self.assertEqual(audit.python_byte_literal(token), expected, token)
        for token in (r'b"\x0"', r'b"\xZZ"', 'b"日本"'):
            self.assertIsNone(audit.python_byte_literal(token), token)

    def test_ordinary_and_raw_byte_string_atoms_join_before_rom_matching(self) -> None:
        text = (b"Synthetic public text block. " * 4)[:96]
        index = audit.RomIndex([text])
        for prefix, suffix in ((b'b"', b'"'), (b'br"', b'"'), (b'br##"', b'"##')):
            atoms = [prefix + text[position:position + 2] + suffix for position in range(0, len(text), 2)]
            source = b"const DATA: &[&[u8]] = &[" + b", ".join(atoms) + b"];\n"
            reasons = audit.content_reasons("fixture.rs", source, index)
            self.assertTrue(any("ROM window" in reason for reason in reasons), reasons)

    @unittest.skipUnless(shutil.which("rustc"), "Rust compiler absent; lexer/Git-state tests remain ROM-free")
    def test_rust_atom_fixtures_compile_and_emit_exact_synthetic_bytes(self) -> None:
        window = IMAGES[0][OFFSET:OFFSET + WINDOW_SIZE]
        forms = rust_atom_forms(window)
        forms.update({name: value for name, value in required_forms(window).items() if value[0] == "rs"})
        with tempfile.TemporaryDirectory(prefix="tje-audit-rust-proof-") as directory:
            for name, (_, declaration_source) in forms.items():
                with self.subTest(form=name):
                    source = Path(directory) / (name.replace("-", "_") + ".rs")
                    executable = source.with_suffix(".exe" if sys.platform == "win32" else "")
                    if name == "one-byte-string":
                        main = (b"fn main() { let bytes: Vec<u8> = DATA.iter().flat_map(|p| p.iter().copied()).collect(); "
                                b"std::io::Write::write_all(&mut std::io::stdout(), &bytes).unwrap(); }\n")
                    else:
                        main = b"fn main() { std::io::Write::write_all(&mut std::io::stdout(), DATA).unwrap(); }\n"
                    source.write_bytes(declaration_source + main)
                    subprocess.run(["rustc", "--edition=2024", str(source), "-o", str(executable)],
                                   capture_output=True, check=True)
                    result = subprocess.run([str(executable)], capture_output=True, check=True)
                    self.assertEqual(result.stdout, window)

    def test_ordinary_code_docs_and_lock_hashes_stay_clean(self) -> None:
        checksum = hashlib.sha256(b"ordinary package checksum").hexdigest()
        samples = {
            "ordinary.rs": b'fn hello() { let x = 0x1000u32; let bytes = b"Ordinary textual error messages remain valid source, not an encoded binary asset."; println!("hello"); }\n',
            "comparison.rs": b'fn compare(width: usize) { if width==16 { println!("Ordinary width equality in a long source file is not quoted-printable binary."); } }\n',
            "ordinary.md": b"# Documentation\n\nPlain prose and a code example: `let bytes = [1u8, 2u8, 3u8];`.\n",
            "Cargo.lock": f'version = 4\n\n[[package]]\nname = "ordinary"\nversion = "1.2.3"\nchecksum = "{checksum}"\n'.encode(),
        }
        index = audit.RomIndex(IMAGES)
        for path, source in samples.items():
            for private in (None, index):
                with self.subTest(path=path, private=private is not None):
                    self.assertEqual(audit.content_reasons(path, source, private), [])

    def test_actual_cargo_lock_and_docs_remain_clean(self) -> None:
        root = Path(audit.__file__).resolve().parents[1]
        for path in ("Cargo.lock", "AGENTS.md", "docs/PLAN.md", "README.md", "tools/audit_public.py"):
            self.assertEqual(audit.content_reasons(path, (root / path).read_bytes(), None), [], path)

    def test_optional_real_rom_forms_in_memory_only(self) -> None:
        root = Path(audit.__file__).resolve().parents[1]
        if not all((root / "roms" / name).is_file() for name in ("rev02.bin", "rev00.bin")):
            self.skipTest("private ROMs absent; all synthetic Git-state regressions still ran")
        images = audit.reference_images(root)
        index = audit.RomIndex(images)
        for revision, image in zip(("REV 02", "REV 00"), images):
            window = image[OFFSET:OFFSET + WINDOW_SIZE]
            forms = required_forms(window) | rust_atom_forms(window)
            for name, (suffix, source) in forms.items():
                for private in (None, index):
                    with self.subTest(revision=revision, form=name, private=private is not None):
                        self.assertTrue(audit.content_reasons(f"in-memory.{suffix}", source, private))


class CompileTimeInclusionTests(GitAuditCase):
    def test_private_and_indirect_includes_in_all_states(self) -> None:
        sources = {
            "private-bin": b'const ROM: &[u8] = include_bytes!("roms/rev02.bin");\n',
            "private-md": b'const ROM: &str = include_str!(r#"roms/rev00.md"#);\n',
            "ignored": b'const ROM: &str = include_str!("ignored.md");\n',
            "concat": b'const ROM: &[u8] = include_bytes!(concat!("roms/", "rev02.bin"));\n',
            "env": b'const ROM: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/roms/rev02.bin"));\n',
            "namespace": b'const ROM: &[u8] = std::include_bytes! { "roms/rev02.bin" };\n',
            "bytes-alias": b'use std::{include_bytes as embed}; const ROM: &[u8] = embed!("roms/rev02.bin");\n',
            "string-alias": b'pub use std::include_str as embed; const ROM: &str = embed!("roms/rev00.md");\n',
        }
        put(self.repo, ".gitignore", b"**/ignored.md\n")
        self.git("add", ".gitignore")
        self.git("commit", "-m", "Synthetic ignored input policy")
        forms = {name: ("rs", source) for name, source in sources.items()}
        cases = self.matrix(forms)
        for state in STATES:
            put(self.repo, f"cases/{state}/ignored.md", b"Ignored private input.\n")
        for private in (False, True):
            self.assert_matrix(cases, private)

    def test_only_same_state_regular_tracked_targets_are_allowed(self) -> None:
        put(self.repo, "src/main.rs", b'const TEXT: &str = include_str!(r#"../public.txt"#);\n')
        put(self.repo, "public.txt", b"Public tracked text.\n")
        self.git("add", "src/main.rs", "public.txt")
        self.git("commit", "-m", "Synthetic public compile-time input")
        self.assertEqual(self.inspect(False), [])
        # A deletion staged in the index must not borrow the target from HEAD.
        (self.repo / "public.txt").unlink()
        self.git("add", "-u", "public.txt")
        findings = self.inspect(False)
        self.assertTrue(any(f.path == "src/main.rs" and f.source == "index:0" for f in findings))
        self.assertTrue(any(f.path == "src/main.rs" and f.source == "worktree" for f in findings))

    def test_historical_include_cannot_borrow_a_future_target(self) -> None:
        put(self.repo, "main.rs", b'const TEXT: &str = include_str!("later.txt");\n')
        self.git("add", "main.rs")
        self.git("commit", "-m", "Synthetic unresolved historical include")
        put(self.repo, "later.txt", b"Public input added later.\n")
        self.git("add", "later.txt")
        self.git("commit", "-m", "Synthetic later target")
        findings = self.inspect(False)
        self.assertTrue(any(f.path == "main.rs" and f.source.startswith("tree:") for f in findings))
        self.assertFalse(any(f.path == "main.rs" and f.source in ("worktree", "index:0") for f in findings))

    def test_symlink_index_target_is_not_a_regular_public_input(self) -> None:
        put(self.repo, "main.rs", b'const TEXT: &str = include_str!("public.txt");\n')
        put(self.repo, "public.txt", b"Public worktree file.\n")
        self.git("add", "main.rs")
        oid = self.git("hash-object", "-w", "--stdin", data=b"elsewhere.txt").strip().decode()
        self.git("update-index", "--add", "--cacheinfo", f"120000,{oid},public.txt")
        self.assertTrue(any(f.path == "main.rs" and f.source == "index:0" for f in self.inspect(False)))

    def test_macro_spellings_comments_strings_lifetimes_and_unicode_space(self) -> None:
        ordinary = b'''// include_bytes!("roms/rev02.bin")
/* nested /* include_str!("private.bin") */ comment */
const DESCRIPTION: &str = r##"include_bytes!("not-an-inclusion")"##;
fn borrow<'a>(value: &'a str) -> &'a str { value }
'''
        self.assertEqual(audit.rust_inclusions("ordinary.rs", ordinary), ([], []))
        for source in (
            b'const X: &[u8] = include_bytes! /* comment */ ("roms/rev02.bin");',
            'const X: &[u8] = include_bytes\u200e!("roms/rev02.bin");'.encode(),
            b'const X: &[u8] = r#include_bytes!["roms/rev02.bin"];',
        ):
            self.assertTrue(audit.rust_inclusions("main.rs", source)[1])
        for target in ("../outside.txt", "/absolute.txt", "C:/private.txt"):
            source = f'const X: &str = include_str!("{target}");'.encode()
            self.assertTrue(audit.rust_inclusions("main.rs", source)[1])


class VendorProvenanceTests(GitAuditCase):
    def setUp(self) -> None:
        super().setUp()
        self.upstream = self.root / "upstream"
        initialize_repo(self.upstream)
        self.baselines = {
            "Cargo.toml": b"[workspace]\n",
            "table.rs": required_forms(IMAGES[0][OFFSET:OFFSET + WINDOW_SIZE])["rust-byte-array"][1],
            "include.rs": b'const TEXT: &[u8] = include_bytes!(\n    "public.txt"\n);\n',
            "public.txt": b"Public upstream text.\n",
        }
        for path, source in self.baselines.items():
            put(self.upstream, path, source)
        git_command(self.upstream, "add", ".")
        git_command(self.upstream, "commit", "-m", "Synthetic public upstream")
        revision = git_command(self.upstream, "rev-parse", "HEAD").strip().decode()
        patch = mock.patch.multiple(audit, UPSTREAM_URL=self.upstream.as_uri(), UPSTREAM_REVISION=revision)
        patch.start()
        self.addCleanup(patch.stop)
        for path, source in self.baselines.items():
            put(self.repo, audit.VENDOR_PREFIX + path, source)
        put(self.repo, audit.VENDOR_PREFIX + "NOTICE.md", b"# Local synthetic vendor notice\n")
        self.git("add", "vendor")
        self.git("commit", "-m", "Synthetic vendor import before provenance bootstrap")
        self.document = audit.generate_vendor_manifest(self.repo)
        self.write_manifest()
        self.git("commit", "-m", "Synthetic verified provenance manifest")

    def write_manifest(self) -> None:
        put(self.repo, audit.VENDOR_MANIFEST, (json.dumps(self.document, sort_keys=True, indent=2) + "\n").encode())
        self.git("add", audit.VENDOR_MANIFEST)

    def test_verified_upstream_tables_are_exempt_in_both_modes_and_history(self) -> None:
        for private in (False, True):
            self.assertEqual(self.inspect(private), [])

    def test_modified_files_audit_only_the_new_lines(self) -> None:
        path = audit.VENDOR_PREFIX + "table.rs"
        put(self.repo, path, self.baselines["table.rs"] + b"pub fn accessor() -> bool { true }\n")
        self.document = audit.generate_vendor_manifest(self.repo)
        self.assertTrue(self.document["files"]["table.rs"]["modified"])
        self.write_manifest()
        for private in (False, True):
            self.assertEqual(self.inspect(private), [])

    def test_removal_only_vendor_changes_have_no_new_bytes_to_audit(self) -> None:
        put(self.repo, audit.VENDOR_PREFIX + "table.rs", b"")
        for private in (False, True):
            self.assertEqual(self.inspect(private), [])

    def test_modified_vendor_payload_in_all_three_git_states(self) -> None:
        path = audit.VENDOR_PREFIX + "table.rs"
        addition = required_forms(IMAGES[1][OFFSET:OFFSET + WINDOW_SIZE])["rust-escape"][1]
        payload = self.baselines["table.rs"] + addition
        for state in STATES:
            put(self.repo, path, payload)
            if state != "worktree":
                self.git("add", path)
            if state == "staged":
                put(self.repo, path, self.baselines["table.rs"])
            elif state == "history":
                self.git("commit", "-m", "Synthetic modified vendor payload")
                put(self.repo, path, self.baselines["table.rs"])
                self.git("add", path)
                self.git("commit", "-m", "Restore synthetic upstream vendor file")
            for private in (False, True):
                self.assert_matrix([(state, path)], private)

    def test_unknown_vendor_file_fails(self) -> None:
        path = audit.VENDOR_PREFIX + "unknown.rs"
        put(self.repo, path, b"pub fn harmless() {}\n")
        self.git("add", path)
        for private in (False, True):
            self.assertTrue(any(f.path == path and "absent from" in f.reason for f in self.inspect(private)))
        with self.assertRaises(audit.AuditError):
            audit.generate_vendor_manifest(self.repo)

    def test_forged_upstream_blob_path_url_revision_and_schema_fail_closed(self) -> None:
        original = json.loads(json.dumps(self.document))
        for mutation in ("blob", "path", "url", "revision", "schema", "local"):
            self.document = json.loads(json.dumps(original))
            if mutation == "blob":
                self.document["files"]["table.rs"]["upstream_blob"] = "0" * 40
            elif mutation == "path":
                self.document["files"]["table.rs"]["upstream_path"] = "absent.rs"
            elif mutation == "local":
                self.document["files"]["table.rs"].update(upstream_path=None, upstream_blob=None, modified=True)
            else:
                self.document[mutation] = "not the pinned value"
            self.write_manifest()
            with self.subTest(mutation=mutation), self.assertRaises(audit.AuditError):
                self.inspect(False)

    def test_invalid_historical_manifest_is_not_hidden_by_current_valid_metadata(self) -> None:
        original = json.loads(json.dumps(self.document))
        self.document["files"]["table.rs"]["upstream_blob"] = "0" * 40
        self.write_manifest()
        self.git("commit", "-m", "Synthetic forged historical provenance")
        self.document = original
        self.write_manifest()
        self.git("commit", "-m", "Restore synthetic verified provenance")
        findings = self.inspect(False)
        self.assertTrue(any(f.path == audit.VENDOR_MANIFEST and f.source.startswith("tree:") for f in findings))

    def test_duplicate_manifest_keys_fail_closed(self) -> None:
        document = json.dumps(self.document).replace('"schema": 1', '"schema": 0, "schema": 1')
        put(self.repo, audit.VENDOR_MANIFEST, document.encode())
        self.git("add", audit.VENDOR_MANIFEST)
        with self.assertRaises(audit.AuditError):
            self.inspect(False)

    def test_vendor_without_any_tracked_manifest_fails_closed(self) -> None:
        repo = self.root / "missing-manifest"
        initialize_repo(repo)
        put(repo, audit.VENDOR_PREFIX + "table.rs", self.baselines["table.rs"])
        git_command(repo, "add", "vendor")
        git_command(repo, "commit", "-m", "Synthetic vendor without provenance")
        with self.assertRaises(audit.AuditError):
            audit.audit(repo, history=True, no_rom=True)

    def test_missing_current_manifest_fails(self) -> None:
        (self.repo / audit.VENDOR_MANIFEST).unlink()
        self.git("add", "-u", audit.VENDOR_MANIFEST)
        self.assertTrue(any("UPSTREAM.json" in f.reason for f in self.inspect(False)))

    def test_cached_verified_tree_survives_network_unavailability(self) -> None:
        self.upstream.rename(self.root / "upstream-unavailable")
        self.assertEqual(self.inspect(False), [])

    def test_missing_cache_and_unavailable_upstream_fail_closed(self) -> None:
        shutil.rmtree(self.repo / ".cache")
        self.upstream.rename(self.root / "upstream-unavailable")
        with self.assertRaises(audit.AuditError):
            self.inspect(False)

    def test_local_notice_gets_no_content_exemption(self) -> None:
        path = audit.VENDOR_PREFIX + "NOTICE.md"
        put(self.repo, path, b"# Local notice\n" + IMAGES[1][OFFSET:OFFSET + WINDOW_SIZE].hex().encode())
        self.assertTrue(any(f.path == path for f in self.inspect(False)))

    def test_changed_include_argument_is_checked_even_when_macro_name_is_unchanged(self) -> None:
        path = audit.VENDOR_PREFIX + "include.rs"
        source = self.baselines["include.rs"].replace(b'"public.txt"', b'"../../roms/rev02.bin"')
        put(self.repo, path, source)
        self.assertTrue(any(f.path == path and "compile-time inclusion" in f.reason for f in self.inspect(False)))

    def test_vendor_symlink_cannot_inherit_an_upstream_content_exemption(self) -> None:
        path = audit.VENDOR_PREFIX + "table.rs"
        oid = self.git("hash-object", "-w", "--stdin", data=self.baselines["table.rs"]).strip().decode()
        self.git("update-index", "--add", "--cacheinfo", f"120000,{oid},{path}")
        self.assertTrue(any(f.path == path and "regular tracked" in f.reason for f in self.inspect(False)))


if __name__ == "__main__":
    unittest.main()
