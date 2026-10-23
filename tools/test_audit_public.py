"""ROM-free publication-gate regressions; all planted bytes are generated, not game data.

Run with uv and tools/audit_requirements.txt; see AGENTS.md for the exact command.
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


def commented_array(window: bytes, language: str = "rs", *, nested: bool = False) -> bytes:
    rows = [b", ".join(f"0x{value:02x}".encode() for value in window[start:start + 8])
            for start in range(0, len(window), 8)]
    comment = b"/* outer row boundary\n /* nested annotation */\n outer tail */" if nested else b"/* row boundary\n annotation text\n*/"
    separator = b",\n# row boundary\n" if language == "py" else b",\n" + comment + b"\n"
    body = separator.join(rows)
    if language == "rs":
        return declaration(b"[\n" + body + b"\n]")
    if language in ("c", "cpp"):
        return b"const unsigned char DATA[] = {\n" + body + b"\n};\n"
    if language == "py":
        return b"DATA = [\n" + body + b"\n]\n"
    return b"const DATA = new Uint8Array([\n" + body + b"\n]);\n"


def concat_hex(window: bytes, *, nested: bool = False, raw: bool = False) -> bytes:
    fragments = [window[start:start + 8].hex().encode() for start in range(0, len(window), 8)]
    literals = [(b'r##"' + value + b'"##') if raw else (b'"' + value + b'"') for value in fragments]
    if nested:
        literals = [b"concat!(" + b", ".join(literals[start:start + 2]) + b")"
                    for start in range(0, len(literals), 2)]
    macro = b"r#concat![" if raw else b"std::concat!("
    closing = b"]" if raw else b")"
    return b"const HEX: &str = " + macro + b"\n" + b",\n/* fragment boundary\n annotation */\n".join(literals) + b"\n" + closing + b";\n"


def grammar_forms(window: bytes) -> dict[str, tuple[str, bytes]]:
    fragments = [b'"' + window[start:start + 8].hex().encode() + b'"'
                 for start in range(0, len(window), 8)]
    c_strings = b" /* fragment\n boundary */\n".join(fragments)
    py_strings = b" # fragment boundary\n".join(fragments)
    javascript = commented_array(window, "js")
    python = commented_array(window, "py")
    fstring = b'DATA = f"{bytes([\n' + python.split(b"[\n", 1)[1].rsplit(b"\n]", 1)[0] + b'\n])!r}"\n'
    return {
        "multiline-comment": ("rs", commented_array(window)),
        "nested-comment": ("rs", commented_array(window, nested=True)),
        "concat-hex": ("rs", concat_hex(window)),
        "nested-concat": ("rs", concat_hex(window, nested=True)),
        "raw-concat": ("rs", concat_hex(window, raw=True)),
        "c-comments": ("c", commented_array(window, "c")),
        "c-adjacent": ("c", b"const char *HEX =\n" + c_strings + b";\n"),
        "python-comments": ("py", python),
        "python-adjacent": ("py", b"HEX = (\n" + py_strings + b"\n)\n"),
        "python-fstring": ("py", fstring),
        "javascript-comments": ("js", javascript),
        "html-script-comments": ("html", b"<script>\n" + javascript + b"</script>\n"),
    }


def encoded_table_forms(window: bytes) -> dict[str, tuple[str, bytes]]:
    strings = [b'"' + window[start:start + 8].hex().encode() + b'"'
               for start in range(0, len(window), 8)]
    body = b",\n".join(strings)
    return {
        "rust-array": ("rs", b"const HEX: [&str; 12] = [\n" + body + b"\n];\n"),
        "python-list": ("py", b"HEX = [\n" + body + b"\n]\n"),
        "yaml-sequence": ("yaml", b"HEX:\n" + b"".join(b"  - " + value + b"\n" for value in strings)),
        "csharp-array": ("cs", b"class Sample { static string[] HEX = new string[] {\n" + body + b"\n}; }\n"),
    }


def compile_rust(directory: Path, name: str, source: bytes) -> bytes:
    path = directory / (name.replace("-", "_") + ".rs")
    executable = path.with_suffix(".exe" if sys.platform == "win32" else "")
    path.write_bytes(source)
    subprocess.run(["rustc", "--edition=2024", str(path), "-o", str(executable)],
                   capture_output=True, check=True)
    return subprocess.run([str(executable)], capture_output=True, check=True).stdout


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

    def matrix(self, forms: dict[str, tuple[str, bytes]], prefix: str = "cases") -> list[tuple[str, str]]:
        cases = []
        for state in STATES:
            for label, (suffix, payload) in forms.items():
                path = f"{prefix}/{state}/{label}.{suffix}"
                put(self.repo, path, payload if state == "history" else BENIGN)
                cases.append((state, path))
        self.git("add", prefix)
        self.git("commit", "-m", "Synthetic historical payloads")
        for state, path in cases:
            if state == "history":
                (self.repo / path).unlink()
        self.git("add", "-u", prefix)
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

    def test_grammar_boundaries_both_revisions_all_git_states_and_modes(self) -> None:
        forms = {}
        for revision, image in zip(("rev02", "rev00"), IMAGES):
            window = image[OFFSET:OFFSET + WINDOW_SIZE]
            forms.update({f"{revision}-{name}": value for name, value in grammar_forms(window).items()})
        cases = self.matrix(forms)
        self.assertEqual(len(cases), 72)
        for private in (False, True):
            self.assert_matrix(cases, private)

    def test_split_literal_tables_both_revisions_all_states_and_modes(self) -> None:
        forms = {}
        for revision, image in zip(("rev02", "rev00"), IMAGES):
            window = image[OFFSET:OFFSET + WINDOW_SIZE]
            forms.update({f"{revision}-{name}": value for name, value in encoded_table_forms(window).items()})
        cases = self.matrix(forms)
        self.assertEqual(len(cases), 24)
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


class PublicationScopeTests(GitAuditCase):
    def test_publication_refs_exclude_unpublished_branches_and_worktree_state(self) -> None:
        self.git("checkout", "-b", "unpublished")
        put(self.repo, "private.bin", b"Synthetic disallowed artifact, not game bytes.")
        self.git("add", "private.bin")
        self.git("commit", "-m", "Synthetic unpublished artifact")
        findings, _ = audit.audit(self.repo, history=True, no_rom=True)
        self.assertTrue(findings)
        findings, summary = audit.audit(self.repo, no_rom=True, publish_refs=["main"])
        self.assertEqual(findings, [])
        self.assertEqual(summary["tracked_worktree_files"], 0)
        self.assertTrue(summary["history"])
        findings, _ = audit.audit(self.repo, no_rom=True, publish_refs=["main", "unpublished"])
        self.assertTrue(any(f.path == "private.bin" for f in findings))
        result = subprocess.run(
            [sys.executable, str(Path(audit.__file__)), "--repo", str(self.repo), "--refs", "main", "--no-rom"],
            capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("history scope: refs main", result.stdout)

    def test_invalid_publication_refs_fail_closed(self) -> None:
        for refs in ([], ["--all"], ["missing-reference"]):
            with self.subTest(refs=refs), self.assertRaises(audit.AuditError):
                audit.audit(self.repo, no_rom=True, publish_refs=refs)


class DecoderTests(unittest.TestCase):
    def test_binary_length_guard_does_not_depend_on_entropy(self) -> None:
        for size in (64, 96, 192):
            binary = bytes(size)
            forms = (("txt", binary.hex().encode()), ("json", json.dumps(binary.decode()).encode()),
                     ("rs", declaration(b'b"' + b"\\0" * size + b'"')))
            for suffix, form in forms:
                with self.subTest(size=size, suffix=suffix, form_length=len(form)):
                    self.assertTrue(audit.content_reasons("fixture." + suffix, form, None))

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
        slash = chr(92)
        invalid = (f'b"{slash}x0"', f'b"{slash}xZZ"', 'b"日本"')
        for token in invalid:
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
                    root = Path(directory)
                    if name == "one-byte-string":
                        main = (b"fn main() { let bytes: Vec<u8> = DATA.iter().flat_map(|p| p.iter().copied()).collect(); "
                                b"std::io::Write::write_all(&mut std::io::stdout(), &bytes).unwrap(); }\n")
                    else:
                        main = b"fn main() { std::io::Write::write_all(&mut std::io::stdout(), DATA).unwrap(); }\n"
                    self.assertEqual(compile_rust(root, name, declaration_source + main), window)

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
            forms = required_forms(window) | rust_atom_forms(window) | grammar_forms(window) | encoded_table_forms(window)
            for name, (suffix, source) in forms.items():
                for private in (None, index):
                    with self.subTest(revision=revision, form=name, private=private is not None):
                        self.assertTrue(audit.content_reasons(f"in-memory.{suffix}", source, private))


class StructuralSourceTests(unittest.TestCase):
    def test_dense_rust_code_tokens_are_not_opaque_encoded_payloads(self) -> None:
        owner = b"OWNED_SOURCES"
        argument = b"translation.id.metadata().source"
        owned = owner + b".contains(&" + argument + b")"
        arguments = b"arguments[2]"
        read = b"fs::read_to_string(&" + arguments + b")?"
        parsed = b"(InputScript::parse(&" + read + b")?)"
        constant = b"ids::" + (b"SOME_LONG_CONSTANT_NAME_" * 4) + b"END,"
        fields = [b'"enabled":true', b'"count":3', b'"name":"ordinary"']
        json = b"serde_json::json!({" + b",".join(fields) + b"})"
        sources = (
            b"fn demo() { let _ = " + owned + b"; }",
            b"fn demo() { let _ = " + parsed + b"; }",
            b"const IDS: &[FunctionId] = &[" + constant + b"];",
            b"fn demo() { let _ = " + json + b"; }",
        )
        index = audit.RomIndex(IMAGES)
        for source in sources:
            for private in (None, index):
                self.assertEqual(audit.content_reasons("dense.rs", source, private), [])

    def test_literal_and_comment_payloads_keep_the_encoding_and_rom_checks(self) -> None:
        window = IMAGES[0][OFFSET:OFFSET + WINDOW_SIZE]
        encoded = window.hex().encode()
        sources = (
            b'const HEX: &str = "' + encoded + b'";\n',
            b"// " + encoded + b"\nfn ordinary() {}\n",
            b"/* " + encoded + b" */\nfn ordinary() {}\n",
        )
        for source in sources:
            for private in (None, audit.RomIndex(IMAGES)):
                self.assertTrue(audit.content_reasons("payload.rs", source, private))
        wrapped = base64.b64encode(bytes(WINDOW_SIZE))
        comments = b"\n".join(b"///" + wrapped[start:start + 32] for start in range(0, len(wrapped), 32))
        self.assertTrue(audit.content_reasons("comment.rs", comments, None))

    def test_nested_containers_tuples_and_sibling_literal_runs(self) -> None:
        window = IMAGES[0][OFFSET:OFFSET + WINDOW_SIZE]
        chunks = [b'"' + window[start:start + 8].hex().encode() + b'"'
                  for start in range(0, len(window), 8)]
        first, second = b", ".join(chunks[:6]), b", ".join(chunks[6:])
        samples = {
            "tuple.py": b"HEX = (" + b", ".join(chunks) + b")",
            "tuple.rs": b"const HEX: (&str,&str,&str,&str,&str,&str,&str,&str,&str,&str,&str,&str) = (" + b", ".join(chunks) + b");",
            "nested.py": b"HEX = [[" + first + b"], [" + second + b"]]",
            "nested.rs": b"const HEX: [[&str;6];2] = [[" + first + b"], [" + second + b"]];",
            "vector.rs": b'fn demo() { let hex = vec![' + b", ".join(chunks) + b"]; }",
            "flow.yaml": b"HEX: [" + b", ".join(chunks) + b"]",
            "nested.yaml": b"HEX: [[" + first + b"], [" + second + b"]]",
            "tuple.cs": b"class Sample { static object HEX = (" + b", ".join(chunks) + b"); }",
            "siblings.py": b"HEX = [external, " + b", ".join(chunks) + b", another]",
        }
        for path, source in samples.items():
            for private in (None, audit.RomIndex(IMAGES)):
                with self.subTest(path=path, private=private is not None):
                    self.assertTrue(audit.content_reasons(path, source, private))

    def test_container_byte_literals_keep_binary_size_checks(self) -> None:
        window = IMAGES[0][OFFSET:OFFSET + WINDOW_SIZE]
        fragments = [b'b"' + b"".join(f"\\x{value:02x}".encode() for value in window[start:start + 8]) + b'"'
                     for start in range(0, len(window), 8)]
        for path, source in (
            ("bytes.py", b"PARTS = [" + b", ".join(fragments) + b"]"),
            ("bytes.rs", b"const PARTS: [&[u8];12] = [" + b", ".join(fragments) + b"];"),
        ):
            self.assertTrue(audit.content_reasons(path, source, None))

    def test_container_aggregation_does_not_merge_fields_or_dense_code(self) -> None:
        fragment = b'"' + bytes(24).hex().encode() + b'"'
        samples = {
            "fields.py": b'DATA = {"first": ' + fragment + b', "second": ' + fragment + b"}",
            "fields.rs": b'fn demo() { let _ = serde_json::json!({"first":' + fragment + b',"second":' + fragment + b"}); }",
            "fields.yaml": b"first: " + fragment + b"\nsecond: " + fragment + b"\n",
            "fields.cs": b"class Record { public string First; public string Second; static Record R = new Record { First = " + fragment + b", Second = " + fragment + b" }; }",
            "text.rs": b'const WORDS: [&str;4] = ["Hello", " ", "world", "!"];',
            "text.py": b'WORDS = ["Hello", " ", "world", "!"]',
            "text.yaml": b'WORDS: ["Hello", " ", "world", "!"]',
            "text.cs": b'class Sample { static string[] Words = { "Hello", " ", "world", "!" }; }',
        }
        for path, source in samples.items():
            with self.subTest(path=path):
                self.assertEqual(audit.content_reasons(path, source, None), [])

    def test_identifier_fact_tables_still_receive_private_decoded_checks(self) -> None:
        binary = bytes([0xb6]) * WINDOW_SIZE
        encoded = base64.b64encode(binary)
        literals = [b'"' + encoded[start:start + 8] + b'"' for start in range(0, len(encoded), 8)]
        source = b"PARTS = [" + b", ".join(literals) + b"]"
        index = audit.RomIndex([binary])
        reasons = audit.content_reasons("identifiers.py", source, index)
        self.assertTrue(any("ROM window" in reason for reason in reasons), reasons)
        # Alphabetic-only hexadecimal chunks are still encoded data, not names.
        hex_literals = [b'"' + (b"abcdefab" * 2) + b'"'] * 12
        hex_source = b"PARTS = [" + b", ".join(hex_literals) + b"]"
        self.assertTrue(audit.content_reasons("hex.py", hex_source, None))

    def test_cpu_field_names_with_short_hex_shaped_identifiers_are_not_payloads(self) -> None:
        source = b'''const CPU_FIELDS: [&str; 25] = [
    "d0", "d1", "d2", "d3", "d4", "d5", "d6", "d7",
    "a0", "a1", "a2", "a3", "a4", "a5", "a6",
    "usp", "ssp", "pc", "sr", "prefetch", "pending_interrupt",
    "stopped", "frozen", "address_error", "last_muldiv",
];'''
        context = audit.SourceContext.build("instruction_proof.rs", source)
        payloads = list(audit.literal_payloads(context, audit.DecodeBudget(), individual=True))
        joined = next(payload for payload in payloads if len(payload) == 102)
        self.assertIn(joined, context.fact_payloads)
        self.assertEqual(len(base64.urlsafe_b64decode(joined + b"=" * (-len(joined) % 4))), 76)
        for private in (None, audit.RomIndex(IMAGES)):
            self.assertEqual(audit.content_reasons("instruction_proof.rs", source, private), [])

    def test_identifier_rule_does_not_exempt_hex_bytes_or_nonidentifier_base64(self) -> None:
        zeroes = bytes(WINDOW_SIZE)
        base64_fragments = base64.b64encode(bytes([0xff]) * WINDOW_SIZE)
        fragments = [base64_fragments[start:start + 8] for start in range(0, len(base64_fragments), 8)]
        samples = {
            "tiny-hex": [b"ab"] * WINDOW_SIZE,
            "hex-plus-name": [zeroes[start:start + 8].hex().encode() for start in range(0, len(zeroes), 8)] + [b"field_name"],
            "base64-nonidentifiers": fragments,
        }
        for name, parts in samples.items():
            source = b"PARTS = [" + b", ".join(b'"' + part + b'"' for part in parts) + b"]"
            with self.subTest(name=name):
                self.assertTrue(audit.content_reasons(name + ".py", source, None))
        # Even printable byte fragments retain their original literal category.
        parts = [b'b"field_name"', b'b"d0"', b'b"a0"']
        source = b"PARTS = [" + b", ".join(parts) + b"]"
        context = audit.SourceContext.build("bytes.py", source)
        list(audit.literal_payloads(context, audit.DecodeBudget(), individual=True))
        self.assertEqual(context.fact_payloads, set())

    def test_malformed_example_text_is_not_reparsed_as_valid_c(self) -> None:
        slash = chr(92)
        example = (f'b"{slash}x0"' + f'b"{slash}xZZ"').encode()
        reason = audit.encoded_reason(example, None, (), audit.DecodeBudget(), "text", path="example.txt")
        self.assertIsNone(reason)
        with mock.patch.object(audit, "c_literals", side_effect=audit.DecodeLimitError("budget exhausted")):
            with self.assertRaises(audit.DecodeLimitError):
                audit.encoded_reason(example, None, (), audit.DecodeBudget(), "text", path="example.txt")

    def test_csharp_grammar_and_literal_forms_are_real(self) -> None:
        self.assertEqual(audit.parser_language("table.cs"), "csharp")
        self.assertEqual(audit.source_literal(b'@"a""b\\c"', "text.cs"), b'a"b\\c')
        self.assertEqual(audit.source_literal(b'"\\x12face"', "text.cs"), chr(0x12fa).encode() + b"ce")
        self.assertEqual(audit.source_literal(b'"\\01"', "text.cs"), bytes(1) + b"1")

    @unittest.skipUnless(shutil.which("rustc"), "Rust compiler absent; all container detections still run")
    def test_rust_and_python_table_proofs_reconstruct_synthetic_windows(self) -> None:
        window = IMAGES[0][OFFSET:OFFSET + WINDOW_SIZE]
        forms = encoded_table_forms(window)
        with tempfile.TemporaryDirectory(prefix="tje-audit-table-proof-") as directory:
            root = Path(directory)
            main = (b"fn main() { let text = HEX.concat(); let bytes: Vec<u8> = text.as_bytes().chunks(2).map(|p| "
                    b"u8::from_str_radix(std::str::from_utf8(p).unwrap(),16).unwrap()).collect(); "
                    b"std::io::Write::write_all(&mut std::io::stdout(), &bytes).unwrap(); }\n")
            rust = forms["rust-array"][1] + main
            self.assertEqual(compile_rust(root, "hex_table", rust), window)
            python = forms["python-list"][1] + b'import sys\nsys.stdout.buffer.write(bytes.fromhex("".join(HEX)))\n'
            output = subprocess.run([sys.executable, "-c", python.decode()], capture_output=True, check=True)
            self.assertEqual(output.stdout, window)

    def test_comments_do_not_change_literal_bytes(self) -> None:
        samples = {
            "literal.rs": b'const TEXT: &str = r##"/* keep */ // keep"##; /* remove\n block */\n',
            "literal.cpp": b'const char *TEXT = R"tag(/* keep */ // keep)tag"; /* remove\n block */\n',
            "literal.py": b'TEXT = r"""# keep /* keep */"""\n# remove\n',
            "literal.js": b'const rx = /\\/\\* keep \\*\\//; const t = `/* keep */ ${ [1, /* remove */ 2] }`;\n',
            "literal.ts": b'const rx: RegExp = /\\/\\* keep \\*\\//; const t = `/* keep */ ${ [1, /* remove */ 2] }`;\n',
        }
        for path, data in samples.items():
            with self.subTest(path=path):
                context = audit.SourceContext.build(path, data)
                self.assertNotIn(b"remove", context.code)
                self.assertIn(b"keep", context.code)
                for kind, value, start in context.tokens:
                    if kind in ("string", "char", "regex", "literal_part"):
                        self.assertEqual(context.code[start:start + len(value)], value)

    def test_literal_only_concat_is_bounded_and_benign_text_stays_clean(self) -> None:
        source = b'''const MESSAGE: &str = concat!("Hello", " ", r#"world"#, '!', 1, true, 1.5);'''
        context = audit.SourceContext.build("message.rs", source)
        self.assertEqual(list(audit.literal_payloads(context, audit.DecodeBudget())), [b"Hello world!1true1.5"])
        self.assertEqual(audit.content_reasons("message.rs", source, None), [])
        unresolved = b'const DATA: &str = concat!("prefix", env!("UNREVIEWED_INPUT"));'
        self.assertTrue(audit.content_reasons("unresolved.rs", unresolved, None))
        metadata = b'const VERSION: &str = concat!("tje-session/", env!("CARGO_PKG_VERSION"));'
        context = audit.SourceContext.build("version.rs", metadata)
        self.assertEqual(list(audit.literal_payloads(context, audit.DecodeBudget())), [b"tje-session/"])
        self.assertEqual(audit.content_reasons("version.rs", metadata, None), [])
        with mock.patch.object(audit, "MAX_JSON_DEPTH", 1):
            nested = b'const DATA: &str = concat!(concat!("a", "b"), "c");'
            self.assertTrue(audit.content_reasons("nested.rs", nested, None))

    def test_python_unicode_literal_composition_preserves_text_semantics(self) -> None:
        source = rb'TEXT = ("\xE9" "\u3042" "\N{LATIN SMALL LETTER A}")'
        context = audit.SourceContext.build("text.py", source)
        self.assertEqual(list(audit.literal_payloads(context, audit.DecodeBudget())), ["éあa".encode()])
        self.assertEqual(audit.content_reasons("text.py", source, None), [])

    def test_parser_missing_fails_closed_without_a_legacy_lexer(self) -> None:
        real_import = __import__
        def blocked(name, *args, **kwargs):
            if name == "tree_sitter_language_pack":
                raise ImportError("synthetic missing grammar package")
            return real_import(name, *args, **kwargs)
        audit.source_parser.cache_clear()
        try:
            with mock.patch("builtins.__import__", side_effect=blocked), self.assertRaises(audit.AuditError):
                audit.SourceContext.build("missing.rs", b"const X: u8 = 1;")
        finally:
            audit.source_parser.cache_clear()

    def test_crlf_and_cr_comment_boundaries(self) -> None:
        window = IMAGES[0][OFFSET:OFFSET + WINDOW_SIZE]
        for ending in (b"\n", b"\r\n", b"\r"):
            for suffix in ("rs", "c", "py", "js"):
                source = commented_array(window, suffix).replace(b"\n", ending)
                self.assertTrue(audit.content_reasons(f"line-endings.{suffix}", source, None))

    @unittest.skipUnless(shutil.which("rustc"), "Rust compiler absent; grammar regressions still run")
    def test_review_comment_and_concat_proofs_emit_exact_synthetic_windows(self) -> None:
        window = IMAGES[0][OFFSET:OFFSET + WINDOW_SIZE]
        forms = {name: value for name, value in grammar_forms(window).items() if value[0] == "rs"}
        with tempfile.TemporaryDirectory(prefix="tje-audit-grammar-proof-") as directory:
            for name, (_, source) in forms.items():
                with self.subTest(form=name):
                    if "concat" in name:
                        main = (b"fn main() { let bytes: Vec<u8> = HEX.as_bytes().chunks(2).map(|pair| "
                                b"u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap()).collect(); "
                                b"std::io::Write::write_all(&mut std::io::stdout(), &bytes).unwrap(); }\n")
                    else:
                        main = b"fn main() { std::io::Write::write_all(&mut std::io::stdout(), DATA).unwrap(); }\n"
                    self.assertEqual(compile_rust(Path(directory), name, source + main), window)


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

    def test_forwarded_inclusion_builtins_in_all_git_states_and_modes(self) -> None:
        forms = {}
        for builtin in ("include_bytes", "include_str", "include"):
            for raw in (False, True):
                name = ("r#" if raw else "") + builtin
                filename = b"private-input.rs" if builtin == "include" else b"private-input.bin"
                definition = b'macro_rules! load { ($m:ident) => { $m! { "../roms/' + filename + b'" } }; }\n'
                if builtin == "include":
                    call = f"load!({name});\n".encode()
                else:
                    target = b"&str" if builtin == "include_str" else b"&[u8]"
                    call = b"const FORWARDED: " + target + b" = load!(" + name.encode() + b");\n"
                forms[builtin + ("-raw" if raw else "")] = ("rs", definition + call)
        forms["review-forward-bytes"] = ("rs", b'macro_rules! load { ($m:ident) => { $m!("../roms/rev02.bin") }; }\nconst DATA: &[u8] = load!(include_bytes);\n')
        cases = self.matrix(forms)
        self.assertEqual(len(cases), 21)
        for private in (False, True):
            self.assert_matrix(cases, private)

    def test_direct_include_of_tracked_public_rust_source_remains_allowed(self) -> None:
        put(self.repo, "main.rs", b'include!("public.rs");\n')
        put(self.repo, "public.rs", b"pub const MESSAGE: &str = \"Public source\";\n")
        self.git("add", "main.rs", "public.rs")
        self.git("commit", "-m", "Synthetic direct public include")
        for private in (False, True):
            self.assertEqual(self.inspect(private), [])

    @unittest.skipUnless(shutil.which("rustc"), "Rust compiler absent; forwarding gate tests still run")
    def test_forwarding_proofs_compile_with_only_synthetic_private_inputs(self) -> None:
        with tempfile.TemporaryDirectory(prefix="tje-audit-forward-proof-") as directory:
            root = Path(directory)
            binary = (IMAGES[0] * 64)
            text = (b"Synthetic private UTF-8 input.\\n" * 40000)[:1024 * 1024]
            (root / "input.bin").write_bytes(binary)
            (root / "input.txt").write_bytes(text)
            window = IMAGES[0][OFFSET:OFFSET + WINDOW_SIZE]
            (root / "input.rs").write_bytes(declaration(b"[" + b",".join(str(value).encode() for value in window) + b"]"))
            for builtin, filename, expected, value in (
                ("include_bytes", "input.bin", binary, b"DATA"),
                ("include_str", "input.txt", text, b"DATA.as_bytes()"),
                ("include", "input.rs", window, b"DATA"),
            ):
                definition = b'macro_rules! load { ($m:ident) => { $m! { "' + filename.encode() + b'" } }; }\n'
                if builtin == "include":
                    declaration_source = b"load!(include);\n"
                else:
                    target = b"&str" if builtin == "include_str" else b"&[u8]"
                    declaration_source = b"const DATA: " + target + b" = load!(" + builtin.encode() + b");\n"
                main = b"fn main() { std::io::Write::write_all(&mut std::io::stdout(), " + value + b").unwrap(); }\n"
                source = definition + declaration_source + main
                self.assertTrue(audit.rust_inclusions("proof.rs", source)[1])
                self.assertEqual(compile_rust(root, builtin, source), expected)

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
            "table.rs": required_forms(IMAGES[0][OFFSET:OFFSET + WINDOW_SIZE])["rust-byte-array"][1].replace(b"const DATA:", b"const UPSTREAM_DATA:"),
            "include.rs": b'const TEXT: &[u8] = include_bytes!(\n    "public.txt"\n);\n',
            "public.txt": b"Public upstream text.\n",
            "assembly.rs": b'const HEX: &str = std::concat!(\n"public",\n" baseline text"\n);\n',
            "projected.rs": commented_array(bytes(range(WINDOW_SIZE))).replace(b"annotation text", b"old annotation text"),
            "unresolved.rs": b'const TEXT: &str = concat!(\nenv!("CARGO_PKG_NAME"),\n/* old comment */\n" public"\n);\n',
        }
        for revision, image in zip(("rev02", "rev00"), IMAGES):
            window = image[OFFSET:OFFSET + WINDOW_SIZE]
            for name, (suffix, _) in (grammar_forms(window) | encoded_table_forms(window)).items():
                benign = b"# Synthetic upstream.\n" if suffix == "py" else b"// Synthetic upstream.\n"
                if suffix == "html":
                    benign = b"<!-- Synthetic upstream. -->\n"
                for state in STATES:
                    self.baselines[f"cases/{state}/{revision}-{name}.{suffix}"] = benign
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

    def test_manifest_generation_ignores_unpublished_prototypes(self) -> None:
        self.git("checkout", "-b", "unpublished-prototype")
        put(self.repo, audit.VENDOR_PREFIX + "prototype.rs", b"pub fn prototype() {}\n")
        self.git("add", audit.VENDOR_PREFIX + "prototype.rs")
        self.git("commit", "-m", "Synthetic unpublished vendor prototype")
        self.git("checkout", "main")
        self.assertEqual(audit.generate_vendor_manifest(self.repo), self.document)

    def test_published_history_recovers_verified_provenance_for_removed_files(self) -> None:
        path = audit.VENDOR_PREFIX + "table.rs"
        (self.repo / path).unlink()
        self.git("add", "-u", path)
        self.document["files"].pop("table.rs")
        self.write_manifest()
        self.git("commit", "-m", "Remove synthetic upstream file from current closure")
        for private in (False, True):
            with mock.patch.object(audit, "reference_images", return_value=IMAGES):
                findings, _ = audit.audit(self.repo, no_rom=not private, publish_refs=["HEAD"])
            self.assertEqual(findings, [])

    def test_vendor_split_literal_tables_all_languages_states_and_modes(self) -> None:
        forms = {}
        for revision, image in zip(("rev02", "rev00"), IMAGES):
            window = image[OFFSET:OFFSET + WINDOW_SIZE]
            forms.update({f"{revision}-{name}": value for name, value in encoded_table_forms(window).items()})
        cases = self.matrix(forms, prefix=audit.VENDOR_PREFIX + "cases")
        self.assertEqual(len(cases), 24)
        for private in (False, True):
            self.assert_matrix(cases, private)

    def test_vendor_grammar_boundaries_both_revisions_all_states_and_modes(self) -> None:
        forms = {}
        for revision, image in zip(("rev02", "rev00"), IMAGES):
            window = image[OFFSET:OFFSET + WINDOW_SIZE]
            forms.update({f"{revision}-{name}": value for name, value in grammar_forms(window).items()})
        cases = self.matrix(forms, prefix=audit.VENDOR_PREFIX + "cases")
        self.assertEqual(len(cases), 72)
        for private in (False, True):
            self.assert_matrix(cases, private)

    def test_vendor_comment_normalization_precedes_changed_line_projection(self) -> None:
        path = audit.VENDOR_PREFIX + "projected.rs"
        source = commented_array(IMAGES[1][OFFSET:OFFSET + WINDOW_SIZE]).replace(b"annotation text", b"new annotation text")
        provenance = audit.VendorAudit(self.repo, (self.repo / audit.VENDOR_MANIFEST).read_bytes())
        delta, _ = provenance.content(path, source, "100644")
        self.assertNotIn(b"/*", delta)
        self.assertIn(b"new annotation text", delta)
        put(self.repo, path, source)
        for private in (False, True):
            self.assertTrue(any(f.path == path and f.source == "worktree" for f in self.inspect(private)))

    def test_vendor_unchanged_concat_frame_still_groups_changed_literals(self) -> None:
        path = audit.VENDOR_PREFIX + "assembly.rs"
        source = concat_hex(IMAGES[1][OFFSET:OFFSET + WINDOW_SIZE])
        provenance = audit.VendorAudit(self.repo, (self.repo / audit.VENDOR_MANIFEST).read_bytes())
        delta, _ = provenance.content(path, source, "100644")
        self.assertNotIn(b"concat!", delta)
        put(self.repo, path, source)
        for private in (False, True):
            self.assertTrue(any(f.path == path and f.source == "worktree" for f in self.inspect(private)))

    def test_vendor_comment_only_change_does_not_reaudit_old_unresolved_macros(self) -> None:
        path = audit.VENDOR_PREFIX + "unresolved.rs"
        put(self.repo, path, self.baselines["unresolved.rs"].replace(b"old comment", b"new comment"))
        for private in (False, True):
            self.assertEqual(self.inspect(private), [])

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
