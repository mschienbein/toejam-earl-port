"""Refuse private material in tracked worktree files, the index, HEAD, and optionally all history.

    python tools/audit_public.py [--history] [--refs REF ...] [--no-rom] [--repo DIR]

No Git objects or files are changed. Reports never echo the offending bytes. --no-rom is suitable
for ROM-free CI; otherwise both pinned reference ROMs are required and verified before indexing.
--refs checks only selected committed ancestry for publication; without it the index/worktree
remain in scope, and --history additionally includes every local ref for hygiene reporting.

Adapted from the GPL-3.0 toejam-earl-decomp public audit. No decompilation-specific
path, header-text, source-literal or historical-blob exceptions are carried over.
"""
from __future__ import annotations

import argparse
import base64
import binascii
import csv
import difflib
import hashlib
import io
import json
import math
import posixpath
import quopri
import string
import re
import os
import subprocess
import sys
import time
import tokenize
import unicodedata
import zlib
from collections import Counter
from dataclasses import dataclass
from functools import lru_cache
from pathlib import Path, PurePosixPath
from urllib.parse import unquote_to_bytes

ROOT = Path(__file__).resolve().parents[1]
UPSTREAM_URL = "https://github.com/jsgroth/jgenesis"
UPSTREAM_REVISION = "d41e2d975771c78ee2efe80f8ec5768b87e3f7eb"
VENDOR_PREFIX = "vendor/jgenesis/"
VENDOR_MANIFEST = VENDOR_PREFIX + "UPSTREAM.json"
MAX_BYTES = 1_000_000
MAX_DECODED_BYTES = 16 * MAX_BYTES
MAX_JSON_NODES = 400_000
MIN_TEXT_MATCH = 64
MAX_JSON_DEPTH = 64
MAX_NON_TEXT_ESCAPES = 8
MAX_NUMERIC_RUN = 64
MIN_BINARY_PAYLOAD = 64
MAX_SIGNATURE_OUTPUT = 4096
MAX_ENCODING_DEPTH = 4
DATA_URI = re.compile(
    rb"data:([^,\s\"'<>]*),"
    rb"([^\s\"'<>]+)", re.I)
PRIVATE_DIRS = {"build", "asm", "orig", ".cache", "scratch", "target", "roms",
                "__pycache__", ".venv", "node_modules"}
ASSET_SUFFIXES = {
    ".bin", ".gen", ".smd", ".sgd", ".rom", ".zip", ".7z", ".gz", ".xz", ".rar", ".bz2", ".tar", ".zlib", ".lib",
    ".o", ".obj", ".a", ".so", ".dll", ".exe", ".elf", ".pyc", ".pdf", ".ieee", ".695",
    ".png", ".jpg", ".jpeg", ".gif", ".bmp", ".webp", ".ico", ".svg",
    ".wav", ".mp3", ".ogg", ".flac", ".aiff", ".aif", ".vgm", ".vgz", ".mid",
    ".mp4", ".webm", ".mov", ".ttf", ".woff", ".woff2",
}
# Whitespace-only windows are also ordinary source indentation.
PADDING = b" " * 16
MARKER = re.compile(
    rb"(?m)^\s*(?:[0-9a-fA-F]+\s+<)?"
    rb"\.Li_[0-9a-fA-F]{6,8}>?\s*:")
INCBIN = re.compile(rb"(?im)^\s*\.?incbin\s+[\"'][^\"'\r\n{}]*baserom(?:\.bin|[/\\])")
CONDITION = rb"(?:ra|sr|hi|ls|cc|cs|ne|eq|vc|vs|pl|mi|ge|lt|gt|le|f|t)"
ASM_WORD = (
    rb"(?:move(?:a|m|p|q)?|lea|pea|link|unlk|rts|rte|rtr|nop|reset|stop|trapv?|illegal|"
    rb"add[aiqx]?|sub[aiqx]?|cmp[aim]?|andi?|ori?|eori?|clr|tst|not|negx?|ext|swap|exg|"
    rb"asl|asr|lsl|lsr|rol|ror|roxl|roxr|div[su]|mul[su]|abcd|sbcd|nbcd|"
    rb"btst|bset|bclr|bchg|chk|tas|b" + CONDITION + rb"|db" + CONDITION + rb"|s" + CONDITION + rb")"
    rb"(?:\.[bwlst]|[bwls])?(?=[ \t\r\n]|$)"
)
LISTING = re.compile(
    rb"(?im)^[ \t]*(?:0x)?[0-9a-f]{1,16}:?[ \t]+"
    rb"(?:(?:[0-9a-f]{2}|[0-9a-f]{4})[ \t]+){1,10}" + ASM_WORD
)
ADDRESS_ASM = re.compile(
    rb"(?im)^[ \t]*(?:0x)?[0-9a-f]{1,16}(?::[ \t]+|\t+)" + ASM_WORD
)
ASM_DIFFER_HEADER = re.compile(rb"(?m)^[ \t]*TARGET[ \t]{2,}CURRENT(?:[ \t]+\([^)\r\n]*\))?[ \t]*$")
OBJDUMP_HEADER = re.compile(rb"(?im)^\s*(?:[0-9a-f]{1,16}\s+<[^>\r\n]+>:\s*$|Disassembly of section [^\r\n]+:\s*$)")
# Contiguous hex, byte/word arrays, and adjacent quoted literals. Odd-length tokens are excluded.
HEX_TOKEN = rb"(?<![\w])(?:0[xX]|\$)?(?:[0-9a-fA-F]{2})+(?![\w])"
HEX_RUN = re.compile(HEX_TOKEN + rb"(?:(?:[\s,]+|[\"']\s*[\"'])" + HEX_TOKEN + rb")*")
HEX_PART = re.compile(rb"(?:0[xX]|\$)?([0-9a-fA-F]+)")
HEX_ESCAPE = re.compile(rb"(?:\\x[0-9a-fA-F]{2}){6,}")
BASE64_TOKEN = re.compile(rb"(?<![A-Za-z0-9+/_=-])[A-Za-z0-9+/_-]{8,}={0,2}(?![A-Za-z0-9+/_=-])")
BASE64_ATOM = re.compile(rb"(?<![A-Za-z0-9+/_=-])(?:[A-Za-z0-9+/_-]+={0,2}|={1,2})(?![A-Za-z0-9+/_=-])")
MARKDOWN = re.compile(r"(?m)^\s*(?:#{1,6}\s|```|~~~|[-*+]\s|\d+\.\s|>\s|\|)|\[[^\n]+\]\([^\n]+\)")
CONTROL_BYTES = re.compile(rb"[\x00-\x08\x0b\x0e-\x1f\x7f]")
GZIP_MAGIC = b"\x1f\x8b\x08"
PY_BYTES_START = re.compile(rb"(?i)(?<![\w\"'])(?:br|rb|b)(?=[\"'])")
PY_BYTES_TOKEN = re.compile(r"(?i)^(?:br|rb|b)[\"']")
C_LEXEME = re.compile(rb"""//[^\r\n]*|/\*[\s\S]*?\*/|(?:u8|[uUL])?"(?:\\[\s\S]|[^"\\])*"|'(?:\\[\s\S]|[^'\\])*'""")
C_STRING = re.compile(rb'(?:u8|[uUL])?"(?:\\[\s\S]|[^"\\])*"')
C_GAP = re.compile(rb"(?:\s|/\*[\s\S]*?\*/|//[^\r\n]*(?:\n|$))*")
C_ESCAPE = re.compile(
    rb"\\(?:x[0-9a-fA-F]|[0-7]|"
    rb"u(?:[0-9a-fA-F]|\{)|U[0-9a-fA-F])")
CHARACTER = rb"(?:u8|[uULb])?'(?:\\[\s\S]|[^'\\])*'"
BYTE_STRING = (
    rb'b"(?:\\[\s\S]|[^"\\])*"|'
    rb'br(?P<raw_hashes>#{0,255})"[\s\S]*?"(?P=raw_hashes)')
BYTE_CAST_ATOM = rb"(?:[ \t]*\)*\s+as\s+[ui]8)"
BYTE_CAST = BYTE_CAST_ATOM + rb"*"
BYTE_CAST_SUFFIX = re.compile(BYTE_CAST_ATOM + rb"+$")
RUST_RAW_START = re.compile(rb'(?:br|cr|r)(#{0,255})"')
RUST_IDENTIFIER = re.compile(rb"[A-Za-z_][A-Za-z0-9_]*")
INTEGER = rb"(?:0[xX]_?[0-9a-fA-F](?:_?[0-9a-fA-F])*|\$[0-9a-fA-F](?:_?[0-9a-fA-F])*|0[oO]_?[0-7](?:_?[0-7])*|0[bB]_?[01](?:_?[01])*|[0-9](?:_?[0-9])*|[0-9a-fA-F]{2,})(?:_?[uUiI](?:8|16|32|64|128|size)|[uUlL]*)"
NUMBER = re.compile(
    rb"(?<![\w.\\])[+-]?(?:" + BYTE_STRING + rb"|" + CHARACTER + rb"|" + INTEGER + rb")"
    + BYTE_CAST + rb"(?![\w.])")
INTEGER_NUMBER = re.compile(
    rb"(?<![\w.\\])[+-]?(?:" + INTEGER + rb")" + BYTE_CAST + rb"(?![\w.])")
BYTE_SEPARATORS = re.compile(rb"""^[\s,;:{}\[\]()=:+*/\\|<>?&!%~^`"'-]*$""")
INT_SUFFIX = re.compile(rb"(?:_?[uUiI](?:8|16|32|64|128|size)|[uUlL]+)$")
ROW_LABEL = re.compile(
    rb"(?m)^[ \t]*[A-Za-z_]\w*:"
    rb"[ \t]*(?=\r?$)")
ARRAY_DECLARATION = re.compile(
    rb"(?m)(?:^|(?<=[;{}]))[ \t]*"
    rb"(?:[A-Za-z_]\w*(?:[ \t]+|\*[ \t]*)){1,8}"
    rb"[A-Za-z_]\w*[ \t]*\[[^\]\r\n]*\]"
    rb"[ \t]*=[ \t]*\{")
MAX_OPAQUE_RUN = 48
OPAQUE_ALPHABETS = {
    "Base32": b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567=",
    "Base64": (string.ascii_letters + string.digits + "+/=").encode(),
    "URL-safe Base64": (string.ascii_letters + string.digits + "-_=").encode(),
    "Base85": (string.digits + string.ascii_uppercase + string.ascii_lowercase + "!#$%&()*+-;<=>?@^_`{|}~").encode(),
    "Z85": (string.digits + string.ascii_lowercase + string.ascii_uppercase + ".-:+=^!/*?&<>()[]{}@%$#").encode(),
    "Ascii85": bytes(range(33, 118)),
    "uuencode": bytes(range(33, 97)),
}
OPAQUE_RUNS = {name: re.compile(b"[" + re.escape(alphabet) + b"]{" + str(MAX_OPAQUE_RUN).encode() + b",}")
               for name, alphabet in OPAQUE_ALPHABETS.items()}
OPAQUE_ENTROPY = {
    name: 4.0 if name == "Base32" else 4.45 if name in ("Base64", "URL-safe Base64") else 4.5
    for name in OPAQUE_RUNS
}
HASH_WORD = re.compile(rb"(?<![A-Za-z0-9_+/=-])(?:[0-9a-fA-F]{40}|[0-9a-fA-F]{64})(?![A-Za-z0-9_+/=-])")
URI_REFERENCE = re.compile(rb"""(?:https?|ftp|ssh|git)://[^\s"'<>]+|(?<![\w.@-])[\w.-]+@[\w.-]+:[^\s"'<>]+""")
PATH_REFERENCE = re.compile(rb"(?<![\w.@-])(?:\.{0,2}/)?[\w.@-]+(?:/[\w.@-]+)+(?:[?#][\w.,:=%&+@/?#~-]*)?")
CITATION_REFERENCE = re.compile(rb"(?<![\w.@$+-])[\w.-]+:(?:[\w.@-]+/)*[\w.@-]+\.[A-Za-z0-9]{1,12}(?::[\w.@$+/-]+)*")
NAMESPACE_REFERENCE = re.compile(rb"(?<!\w)[a-z][a-z0-9_]{0,31}:[a-z][a-z0-9_]{0,31}(?:-[a-z][a-z0-9_]{0,31}){2,}")
HASH_COMPONENT = re.compile(rb"""(?<=/)(?:[0-9a-fA-F]{40}|[0-9a-fA-F]{64})(?=[/?.#"'`]|$)""")
PACKAGE_HASH = re.compile(rb"(?<=/packages/)[0-9a-fA-F]{2}/[0-9a-fA-F]{2}/[0-9a-fA-F]{60}(?=/)")
QUOTED_IDENTIFIER = re.compile(rb"""(["'])([A-Za-z_]\w*)\1""")
REFERENCE_EXPRESSION = re.compile(rb"[A-Za-z_]\w*(?:[.\[\](),;]+(?:[A-Za-z_]\w*|[0-9]+)?)+")
INTEL_LINE = re.compile(rb"^[ \t]*:[0-9A-Fa-f]{10,}[ \t]*$")
SRECORD_LINE = re.compile(rb"^[ \t]*S[0-9][0-9A-Fa-f]{4,}[ \t]*$")
UU_LINE = re.compile(rb"^M[ -`]{60}[ \t]*$")
QP_ESCAPE = re.compile(rb"=[0-9A-Fa-f]{2}")


def ordered_string_chunks(mapping: dict):
    """Only an explicit, consecutive numbered-part schema defines a joined object stream."""
    groups = {}
    for key, value in mapping.items():
        match = re.fullmatch(r"([A-Za-z_][A-Za-z_.-]*)([0-9]+)", str(key))
        if match and isinstance(value, str):
            groups.setdefault(match[1], []).append((int(match[2]), value))
    for entries in groups.values():
        entries.sort()
        if len(entries) < 2 or [entry[0] for entry in entries] != list(range(len(entries))):
            continue
        values = [value.encode("utf-8", "surrogatepass") for _, value in entries]
        if all(re.fullmatch(rb"(?:[0-9a-fA-F]{40}|[0-9a-fA-F]{64})", value) for value in values):
            continue
        if all(re.fullmatch(rb"0[xX][0-9a-fA-F]{4,8}", value) and int(value, 16) > 255 for value in values):
            continue  # Numbered address fields are facts, not an encoded byte stream.
        yield b"".join(values)


def opaque_metrics(value: bytes) -> tuple[float, int]:
    """Shannon bits per character and the number of digit/upper/lower/symbol classes."""
    counts = Counter(value)
    if not counts:
        return 0.0, 0
    classes = {0 if 48 <= byte <= 57 else 1 if 65 <= byte <= 90 else 2 if 97 <= byte <= 122 else 3
               for byte in counts}
    size = len(value)
    entropy = math.log2(size) - sum(count * math.log2(count) for count in counts.values()) / size
    return entropy, len(classes)


def opaque_component(value: bytes) -> bool:
    if len(value) < MAX_OPAQUE_RUN:
        return False
    entropy, classes = opaque_metrics(value)
    return classes >= 2 and any(pattern.fullmatch(value) and entropy >= OPAQUE_ENTROPY[name]
                                for name, pattern in OPAQUE_RUNS.items())


def structured_text_view(text: bytes) -> bytes:
    """Remove complete reviewed fact shapes, never a language, file or string category."""
    spans = [match.span() for pattern in (HASH_WORD, HASH_COMPONENT, PACKAGE_HASH)
             for match in pattern.finditer(text)]
    for pattern in (URI_REFERENCE, CITATION_REFERENCE, PATH_REFERENCE, NAMESPACE_REFERENCE):
        for match in pattern.finditer(text):
            token = match.group()
            if pattern is PATH_REFERENCE and b"." not in token and b"-" not in token:
                continue
            components = HASH_COMPONENT.sub(b"", PACKAGE_HASH.sub(b"", token))
            if not any(opaque_component(part) for part in re.split(rb"[/?:=&]", components)):
                spans.append(match.span())
    # Short/low-entropy identifier atoms do not become secrets just because JSON/code framing
    # supplies extra punctuation. High-entropy literal identifiers remain candidates.
    for match in QUOTED_IDENTIFIER.finditer(text):
        if not opaque_component(match[2]):
            spans.append(match.span())
    for match in re.finditer(rb"\S+", text):
        word = match.group().strip(b"();,")
        if REFERENCE_EXPRESSION.fullmatch(word) and not any(
                opaque_component(atom) for atom in re.findall(rb"[A-Za-z_]\w*|[0-9]+", word)):
            spans.append(match.span())
    view = bytearray(text)
    for start, end in spans:
        view[start:end] = b" " * (end - start)
    return bytes(view)


def opaque_reason(path: str, data: bytes) -> str | None:
    # This raw pass is unconditional and path-independent: no source-language parser may
    # discard single quotes, templates, raw strings, comments or malformed source snippets.
    view = structured_text_view(data)
    for line in view.splitlines():
        if INTEL_LINE.fullmatch(line):
            return "Intel HEX record-line representation is not public source"
        if SRECORD_LINE.fullmatch(line):
            return "Motorola S-record representation is not public source"
        if UU_LINE.fullmatch(line):
            return "uuencode M-record representation is not public source"
        escapes = list(QP_ESCAPE.finditer(line))
        if len(escapes) >= 4 and 3 * len(escapes) >= len(line.strip()) // 4:
            return "dense quoted-printable record-line representation is not public source"
    metrics = {}
    for name, pattern in OPAQUE_RUNS.items():
        for match in pattern.finditer(view):
            key = match.span()
            if key not in metrics:
                metrics[key] = opaque_metrics(match.group())
            entropy, classes = metrics[key]
            if classes >= 2 and entropy >= OPAQUE_ENTROPY[name]:
                return (f"opaque {name} run of {len(match.group())} characters; "
                        f"entropy {entropy:.3f} >= {OPAQUE_ENTROPY[name]:.2f} bits/char, "
                        f"{classes} character classes")
    return None


def printable_text(data: bytes) -> bool:
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError:
        return False
    return all(char.isprintable() or char in "\n\r\t\f" for char in text)


def looks_binary(data: bytes) -> bool:
    """Do not turn short ordinary English/Unicode overlaps into ROM evidence."""
    if CONTROL_BYTES.search(data):
        return True
    try:
        data.decode("utf-8")
        return False
    except UnicodeDecodeError:
        return any(0x80 <= byte <= 0x9F for byte in data) or sum(byte >= 0x80 for byte in data) * 3 >= len(data)


class AuditError(RuntimeError):
    pass


@dataclass(frozen=True)
class Finding:
    source: str
    path: str
    reason: str


@dataclass
class CLiteral:
    parts: int
    raw: bytes
    escaped_nontext: int


def git(repo: Path, *args: str) -> bytes:
    result = subprocess.run(["git", "-C", str(repo), *args], capture_output=True)
    if result.returncode:
        raise AuditError(result.stderr.decode("utf-8", "replace").strip())
    return result.stdout


class RomIndex:
    """Python's hash tables index exact windows (not collision-prone truncated digests)."""

    def __init__(self, images: list[bytes]):
        self.images = len(images)
        self.roms = images
        self.windows = {
            width: {rom[pos:pos + width] for rom in images for pos in range(len(rom) - width + 1)}
            for width in (12, 16)
        }

    def contains(self, data: bytes, width: int, allowed: tuple[bytes, ...] = (), *, evidence: bool = False) -> int | None:
        windows = self.windows[width]
        for pos in range(len(data) - width + 1):
            window = data[pos:pos + width]
            if window not in windows or (width == 16 and window == PADDING):
                continue
            # Header fields may touch space padding; require at least twelve field characters.
            unpadded = window.strip(b" ")
            if any(window in text or (width == 16 and len(unpadded) >= 12 and unpadded in text) for text in allowed):
                continue
            if evidence and not looks_binary(window):
                # A prose overlap of 12/16 bytes is not evidence. Require a long contiguous ROM
                # quotation instead; ordinary prose is not cartridge evidence.
                text = data[pos:pos + MIN_TEXT_MATCH]
                if len(text) < MIN_TEXT_MATCH or not any(text in rom for rom in self.roms):
                    continue
            return pos
        return None


class DecodeLimitError(AuditError):
    pass


class LiteralDecodeError(DecodeLimitError):
    """A speculative literal spelling is invalid, unlike an exhausted decode budget."""


class DecodeBudget:
    def __init__(self):
        self.used = 0
        self.structural_failure = False
        self.numeric_scanned = set()
        self.opaque_scanned = set()
        self.identifier_facts = False

    def charge(self, size: int) -> None:
        self.used += size
        if self.used > MAX_DECODED_BYTES:
            raise DecodeLimitError("encoded-content decoding exceeds the per-blob byte budget")


def decode_c_literal(token: bytes, *, rust: bool = False,
                     python: bool = False, unicode_text: bool = False,
                     csharp: bool = False) -> tuple[bytes, int]:
    """Decode literal escapes without invoking a source-language compiler."""
    body = token[token.index(b'"') + 1:-1]
    out, escaped_nontext = bytearray(), 0
    simple = {ord("a"): 7, ord("b"): 8, ord("f"): 12, ord("n"): 10, ord("r"): 13,
              ord("t"): 9, ord("v"): 11, ord("\\"): 92, ord("'"): 39, ord('"'): 34, ord("?"): 63}
    position = 0
    while position < len(body):
        value = body[position]
        position += 1
        if value != 92:
            out.append(value)
            continue
        if position == len(body):
            raise LiteralDecodeError("unterminated C escape")
        value = body[position]
        position += 1
        if value in (10, 13):
            if value == 13 and body[position:position + 1] == b"\n":
                position += 1
            if rust:
                while position < len(body) and body[position] in b" \t\r\n":
                    position += 1
            continue
        if (rust or csharp) and value == ord("0"):
            decoded = bytes(1)
        elif value in simple and not (python and value == ord("?")):
            decoded = bytes([simple[value]])
        elif not rust and not csharp and 48 <= value <= 55:
            digits = bytearray([value])
            while len(digits) < 3 and position < len(body) and 48 <= body[position] <= 55:
                digits.append(body[position])
                position += 1
            number = int(digits, 8)
            decoded = chr(number).encode() if unicode_text else bytes([number & 0xFF])
        elif value == ord("x"):
            start = position
            if rust or python:
                position += 2
                if not re.fullmatch(rb"[0-9a-fA-F]{2}", body[start:position]):
                    raise LiteralDecodeError("invalid two-digit hexadecimal byte escape")
            else:
                while (position < len(body) and body[position] in b"0123456789abcdefABCDEF"
                       and (not csharp or position - start < 4)):
                    position += 1
                if position == start:
                    raise LiteralDecodeError("invalid C hexadecimal escape")
            number = int(body[start:position], 16)
            decoded = chr(number).encode() if unicode_text else bytes([number & 0xFF])
        elif python and unicode_text and value == ord("N"):
            end = body.find(b"}", position + 1)
            if body[position:position + 1] != b"{" or end < 0:
                raise LiteralDecodeError("invalid named Unicode escape")
            try:
                decoded = unicodedata.lookup(body[position + 1:end].decode("ascii")).encode()
            except (KeyError, UnicodeError) as error:
                raise LiteralDecodeError("invalid named Unicode escape") from error
            position = end + 1
        elif python and not unicode_text and value in b"uUN":
            decoded = b"\\" + bytes([value])
        elif value == ord("u") and body[position:position + 1] == b"{":
            end = body.find(b"}", position + 1)
            digits = body[position + 1:end].replace(b"_", b"")
            if end < 0 or not re.fullmatch(rb"[0-9a-fA-F]{1,6}", digits):
                raise LiteralDecodeError("invalid Rust Unicode escape")
            try:
                decoded = chr(int(digits, 16)).encode("utf-8")
            except (ValueError, UnicodeError) as error:
                raise LiteralDecodeError("invalid Rust Unicode scalar") from error
            position = end + 1
        elif value in (ord("u"), ord("U")):
            length = 4 if value == ord("u") else 8
            try:
                decoded = chr(int(body[position:position + length], 16)).encode("utf-8")
            except (ValueError, UnicodeError) as error:
                raise LiteralDecodeError("invalid C universal character escape") from error
            position += length
        else:
            if rust or csharp:
                raise LiteralDecodeError("invalid source string escape")
            decoded = (b"\\" if python else b"") + bytes([value])
        out.extend(decoded)
        escaped_nontext += sum(byte < 32 and byte not in b"\n\r\t\f" or byte >= 127 for byte in decoded)
    return bytes(out), escaped_nontext


def c_literals(data: bytes, budget: DecodeBudget):
    """Join adjacent tokens across whitespace/comments; inspect commented-out literals too."""
    tokens = list(C_LEXEME.finditer(data))

    def groups(matches, region: bytes):
        parts, escaped = [], 0
        previous = 0
        for match in matches:
            token = match.group()
            if token.startswith((b"//", b"/*", b"'")):
                continue
            if parts and C_GAP.fullmatch(region[previous:match.start()]) is None:
                raw = b"".join(parts)
                budget.charge(len(raw))
                yield CLiteral(len(parts), raw, escaped)
                parts, escaped = [], 0
            raw, count = decode_c_literal(token)
            parts.append(raw)
            escaped += count
            previous = match.end()
        if parts:
            raw = b"".join(parts)
            budget.charge(len(raw))
            yield CLiteral(len(parts), raw, escaped)

    yield from groups(tokens, data)
    for token in tokens:
        if token.group().startswith((b"//", b"/*")):
            end = -2 if token.group().startswith(b"/*") else None
            region = token.group()[2:end]
            yield from groups(C_STRING.finditer(region), region)


def json_payloads(text: str, budget: DecodeBudget):
    """Decode JSON strings/keys and adjacent array strings; never join unrelated object fields."""
    if not text.lstrip().startswith(("{", "[", '"')):
        return
    try:
        document = json.loads(text)
    except json.JSONDecodeError:
        return
    except RecursionError as error:
        raise DecodeLimitError("JSON exceeds the bounded decoding depth") from error

    def variants(value: str, literal: bool = True):
        raw = value.encode("utf-8", "surrogatepass")
        budget.charge(len(raw))
        yield raw, literal
        try:
            latin = value.encode("latin1")
        except UnicodeEncodeError:
            return
        if latin != raw:
            budget.charge(len(latin))
            yield latin, literal

    work = [(document, 0)]
    nodes = 0
    while work:
        value, depth = work.pop()
        nodes += 1
        if nodes > MAX_JSON_NODES or depth > MAX_JSON_DEPTH:
            raise DecodeLimitError("JSON exceeds the bounded decoding node/depth budget")
        if isinstance(value, str):
            yield from variants(value)
        elif isinstance(value, dict):
            for joined in ordered_string_chunks(value):
                budget.charge(len(joined))
                yield joined, False
            work.extend((item, depth + 1) for pair in value.items() for item in pair)
        elif isinstance(value, list):
            # Preserve adjacency, including runs separated by non-string elements.
            run = []
            for item in value + [None]:
                if isinstance(item, str):
                    run.append(item)
                else:
                    if len(run) > 1:
                        yield from variants("".join(run), literal=False)
                    run = []
            work.extend((item, depth + 1) for item in value)


def python_byte_literal(token: str) -> bytes | None:
    """Decode a tokenized Python bytes literal, preserving unknown byte escapes."""
    prefix = PY_BYTES_TOKEN.match(token)
    if prefix is None:
        return None
    quote = prefix.end() - 1
    delimiter = token[quote] * (3 if token.startswith(token[quote] * 3, quote) else 1)
    if not token.endswith(delimiter):
        return None
    try:
        body = token[quote + len(delimiter):-len(delimiter)]
        raw = body.replace("\r\n", "\n").replace("\r", "\n").encode("ascii")
        if "r" in token[:quote].lower():
            return raw
        return decode_c_literal(b'"' + raw + b'"', python=True)[0]
    except (UnicodeError, DecodeLimitError):
        return None


def python_byte_payloads(data: bytes, budget: DecodeBudget):
    """Tokenize, never execute: repr(bytes), adjacent literals and simple literal '+' runs."""
    if not PY_BYTES_START.search(data):
        return
    parts = []

    def flush():
        if parts:
            raw = b"".join(parts)
            parts.clear()
            budget.charge(len(raw))
            return raw
        return None

    try:
        for token in tokenize.tokenize(io.BytesIO(data).readline):
            if token.type == tokenize.STRING and PY_BYTES_TOKEN.match(token.string):
                value = python_byte_literal(token.string)
                if isinstance(value, bytes):
                    parts.append(value)
                    continue
            elif token.type in (tokenize.ENCODING, tokenize.NL, tokenize.COMMENT, tokenize.INDENT, tokenize.DEDENT):
                continue
            elif token.type == tokenize.OP and token.string == "+" and parts:
                continue
            raw = flush()
            if raw is not None:
                yield raw
    except (tokenize.TokenError, IndentationError, SyntaxError, UnicodeError):
        pass  # Snippets/Markdown need not be complete Python; already tokenized literals count.
    raw = flush()
    if raw is not None:
        yield raw


def rust_byte_string(token: bytes) -> bytes:
    """Decode one lexer-validated ordinary or raw Rust byte-string atom."""
    if token.startswith(b'br'):
        quote = token.index(b'"')
        hashes = quote - 2
        return token[quote + 1:len(token) - hashes - 1]
    return decode_c_literal(token[1:], rust=True)[0]


def literal_values(token: bytes) -> tuple[tuple[int | None, ...], bytes]:
    """One number/escape decoder: canonical, decimal-dump, hex-dump, packed-hex views.

    Do not discard a minus sign or silently truncate a non-byte integer. All byte views are
    mapped only for -128..255; the packed view preserves explicit hexadecimal word dumps.
    """
    cast = BYTE_CAST_SUFFIX.search(token)
    cast_type = token[cast.start():].split()[-1] if cast else None
    token = token[:cast.start()] if cast else token

    def cast_value(value: int) -> int:
        if cast_type is None:
            return value
        value &= 255
        return value - 256 if cast_type == b"i8" and value >= 128 else value

    sign = -1 if token.startswith(b"-") else 1
    unsigned = token[1:] if token[:1] in (b"+", b"-") else token
    if unsigned.startswith((b'b"', b'br')):
        try:
            raw = rust_byte_string(unsigned)
        except DecodeLimitError:
            return (None, None, None, None), b""  # E.g. a regex, not a valid Rust atom.
        value = cast_value(raw[0] * sign) if len(raw) == 1 else None
        return (value, value, value, value), raw
    if b"'" in unsigned:
        body = unsigned[unsigned.index(b"'") + 1:-1]
        try:
            raw, _ = decode_c_literal(b'"' + body + b'"', rust=unsigned.startswith(b"b'"))
        except DecodeLimitError:
            return (None, None, None, None), b""  # A template is not a character literal.
        if len(raw) == 1:
            value = raw[0] * sign
        else:
            try:
                text = raw.decode("utf-8")
            except UnicodeDecodeError:
                return (None, None, None, None), b""
            if len(text) != 1:
                return (None, None, None, None), b""
            value = ord(text) * sign
        value = cast_value(value)
        return (value, value, value, value), bytes([value & 255]) if -128 <= value <= 255 else b""
    spelling = INT_SUFFIX.sub(b"", unsigned).replace(b"_", b"")
    lower = spelling.lower()
    explicit_hex = lower.startswith(b"0x") or spelling.startswith(b"$")
    if explicit_hex:
        digits = spelling[2:] if lower.startswith(b"0x") else spelling[1:]
        value = int(digits, 16) * sign
    elif re.fullmatch(rb"0[oO][0-7]+|0[bB][01]+", spelling):
        value = int(spelling[2:], 8 if lower.startswith(b"0o") else 2) * sign
    elif any(byte in b"abcdefABCDEF" for byte in spelling):
        value = int(spelling, 16) * sign
    else:
        base = 8 if len(spelling) > 1 and spelling.startswith(b"0") and all(byte in b"01234567" for byte in spelling) else 10
        value = int(spelling, base) * sign
    decimal = int(spelling, 10) * sign if spelling.isdigit() else value
    hexadecimal = int(spelling, 16) if token[:1] not in (b"+", b"-") and not explicit_hex and len(spelling) <= 2 else value
    octal = int(spelling, 8) * sign if spelling.isdigit() and all(byte in b"01234567" for byte in spelling) else value
    if cast_type is not None:
        values = tuple(map(cast_value, (value, decimal, hexadecimal, octal)))
        return values, bytes([values[0] & 255])
    packed = b""
    if explicit_hex and sign > 0 and len(digits) <= 8:
        packed = bytes.fromhex(digits.zfill(len(digits) + len(digits) % 2).decode())
    elif -128 <= value <= 255:
        packed = bytes([value & 255])
    return (value, decimal, hexadecimal, octal), packed


def numeric_variants(data: bytes, matches: list[re.Match]) -> tuple[bytes, ...]:
    """Decode one lexical run, retaining lossless and optional address-filtered views."""
    streams = [bytearray() for _ in range(10)]
    line_start = 0
    previous_end = 0
    for ordinal, match in enumerate(matches):
        newline = data.rfind(b"\n", previous_end, match.start())
        if newline >= 0:
            line_start = newline + 1
        first_on_line = ordinal == 0 or matches[ordinal - 1].end() <= line_start
        token = match.group()
        spelling = INT_SUFFIX.sub(b"", BYTE_CAST_SUFFIX.sub(b"", token.lstrip(b"+-"))).replace(b"_", b"")
        digits = spelling[2:] if spelling.lower().startswith(b"0x") else spelling[1:] if spelling.startswith(b"$") else spelling
        offset = not any(quote in spelling for quote in (b"'", b'"'))
        offset = offset and token[:1] not in (b"+", b"-") and not spelling.lower().startswith((b"0o", b"0b"))
        offset = offset and first_on_line and ordinal + 1 < len(matches) and len(digits) >= 4
        if offset:
            prefix = data[line_start:match.start()].strip()
            gap = data[match.end():matches[ordinal + 1].start()]
            offset = not prefix and bool(gap) and b"\n" not in gap and not gap.strip(b": \t\r")
        values, packed = literal_values(token)
        chunks = tuple(bytes([value & 255]) if value is not None and -128 <= value <= 255 else b"" for value in values) + (packed,)
        for stream, chunk in zip(streams[:5], chunks):
            stream.extend(chunk)
        if not offset:
            for stream, chunk in zip(streams[5:], chunks):
                stream.extend(chunk)
        previous_end = match.end()
    return tuple(bytes(stream) for stream in streams)




def numeric_blocks(data: bytes, budget: DecodeBudget, *,
                   path: str = "", source: SourceContext | None = None):
    """Every maximal numeric-token run, in any lexical context or container syntax."""
    if CONTROL_BYTES.search(data):
        return
    try:
        data.decode("utf-8")
    except UnicodeDecodeError:
        return

    source = source if source is not None else SourceContext.build(path, data)
    literal_spans = [(start, start + len(value)) for kind, value, start in source.tokens
                     if kind in ("string", "char", "regex", "literal_part")]

    def annotation(match):
        if any(start < match.end() and match.start() < end for start, end in literal_spans):
            return match.group()
        return bytes(byte if byte in (10, 13) else 32 for byte in match.group())

    def views():
        yield data  # Raw/comment contents never disappear from the audit.
        normalized = source.code
        for pattern in (ROW_LABEL, ARRAY_DECLARATION):
            normalized = pattern.sub(annotation, normalized)
        if normalized != data:
            yield normalized

    seen = set()
    for view in views():
        # Preserve complete byte-character/string atoms as well as the integer-only
        # view: neither a prefix nor a quoted string may hide a neighbouring run.
        lexers = (INTEGER_NUMBER, NUMBER)
        for lexer in lexers:
            run = []
            first_line = 0
            current_line = 1
            previous_end = 0

            def flush():
                nonlocal run, first_line
                variants = numeric_variants(view, run)
                result = (first_line, variants)
                run = []
                if variants in seen:
                    return None
                seen.add(variants)
                for value in set(variants):
                    budget.charge(len(value))
                return result

            for match in lexer.finditer(view):
                gap = view[previous_end:match.start()]
                current_line += gap.count(b"\n")
                if run and BYTE_SEPARATORS.fullmatch(gap) is None:
                    result = flush()
                    if result is not None:
                        yield result
                if not run:
                    first_line = current_line
                run.append(match)
                current_line += match.group().count(b"\n")
                previous_end = match.end()
            if run:
                result = flush()
                if result is not None:
                    yield result


def numeric_reason(data: bytes, index: RomIndex | None, allowed: tuple[bytes, ...],
                   budget: DecodeBudget, origin: str, path: str, *,
                   source: SourceContext | None = None) -> str | None:
    key = hashlib.sha256(data).digest()
    if key in budget.numeric_scanned:
        return None
    budget.numeric_scanned.add(key)
    for line, variants in numeric_blocks(data, budget, path=path, source=source):
        for raw in dict.fromkeys(variants):
            reason = payload_reason(raw, index, allowed, f"{origin}: numeric run at line {line}", budget)
            if reason is not None:
                return reason
        count = max(map(len, variants[:4]), default=0)
        if count >= MAX_NUMERIC_RUN:
            return f"{origin}: numeric run contains {count} byte values (limit {MAX_NUMERIC_RUN - 1}; no exemptions)"
    return None


def base64_candidates(data: bytes, *, binary_shapes: set[bytes] | None = None) -> set[bytes]:
    """Keep broad private-ROM probes, but distinguish framed data from joined prose."""
    tokens = set()
    for match in BASE64_TOKEN.finditer(data):
        token = match.group()
        tokens.add(token)
        before, after = data[match.start() - 1:match.start()], data[match.end():match.end() + 1]
        if binary_shapes is not None and before in (b'"', b"'", b"`") and after == before:
            binary_shapes.add(token)
    # Formatting headers delimit a run; their language names/chomping sigils are not payload.
    markup = re.sub(rb"""(?m)^[ \t]*(?:(?:[\w.-]+|"[^"\r\n]*"|'[^'\r\n]*'):[ \t]*)?(?:![\w!:/.-]+[ \t]+)?[|>](?:[1-9][+-]?|[+-][1-9]?)?[ \t]*(?:#[^\r\n]*)?$""", b"@", data)
    markup = re.sub(rb"(?m)^[ \t]*(?:>[ \t]*)+", b"", markup)
    markup = re.sub(rb"(?m)^[ \t]*(?:`{3,}|~{3,})[^\r\n]*", b"@", markup)
    markup = re.sub(rb"""(?m)^[ \t]*["']|["'][ \t]*$""", b"", markup)
    for view in dict.fromkeys((data, markup)):
        if binary_shapes is not None:
            framed = []
            for line in [*view.splitlines(), b""]:
                line = line.strip()
                if line and re.fullmatch(rb"[A-Za-z0-9+/_-]+={0,2}", line):
                    framed.append(line)
                else:
                    if framed:
                        binary_shapes.add(b"".join(framed))
                        framed.clear()
        chunks = []
        previous = None

        def flush() -> None:
            if len(chunks) > 1:
                value = b"".join(chunks)
                if len(value) >= 8:
                    tokens.add(value)
            chunks.clear()

        for atom in BASE64_ATOM.finditer(view):
            if previous is not None and not view[previous:atom.start()].isspace():
                flush()
            chunks.append(atom.group())
            previous = atom.end()
            if b"=" in atom.group():
                flush()
                previous = None
        flush()
    return tokens


def embedded_kind(raw: bytes, budget: DecodeBudget) -> str | None:
    """Recognize binary formats, not fixture paths. Bare short magic constants are not files."""
    # A complete zlib stream can be smaller than twelve bytes (unlike a bare two-byte magic).
    if len(raw) >= 6 and raw[0] & 15 == 8 and raw[0] >> 4 <= 7 and int.from_bytes(raw[:2], "big") % 31 == 0:
        try:
            stream = zlib.decompressobj()
            output = stream.decompress(raw, MAX_SIGNATURE_OUTPUT + 1)
            budget.charge(len(output))
            if stream.eof or len(output) > MAX_SIGNATURE_OUTPUT:
                return "zlib compressed data"
        except zlib.error:
            pass
    if len(raw) < 12:
        return None
    signatures = (
        ("PNG image", b"\x89PNG\r\n\x1a\n"), ("GIF image", b"GIF87a"), ("GIF image", b"GIF89a"),
        ("JPEG image", b"\xff\xd8\xff"), ("TIFF image", b"II*\0"), ("TIFF image", b"MM\0*"),
        ("Photoshop image", b"8BPS"), ("ZIP archive", b"PK\x03\x04"), ("ZIP archive", b"PK\x05\x06"),
        ("ZIP archive", b"PK\x07\x08"), ("7z archive", b"7z\xbc\xaf'\x1c"),
        ("xz compressed data", b"\xfd7zXZ\0"), ("bzip2 compressed data", b"BZh"),
        ("RAR archive", b"Rar!\x1a\x07"), ("Unix archive", b"!<arch>\n"),
        ("zstd compressed data", b"\x28\xb5\x2f\xfd"), ("lz4 compressed data", b"\x04\x22\x4d\x18"),
        ("CAB archive", b"MSCF"), ("RPM package", b"\xed\xab\xee\xdb"),
        ("ELF object", b"\x7fELF"), ("PDF document", b"%PDF-"), ("Ogg audio", b"OggS"),
        ("FLAC audio", b"fLaC"), ("MP3 audio", b"ID3"), ("MIDI audio", b"MThd"),
        ("TrueType font", b"\0\1\0\0"), ("OpenType font", b"OTTO"), ("WOFF font", b"wOFF"),
        ("WOFF2 font", b"wOF2"), ("CPIO archive", b"070701"), ("CPIO archive", b"070702"), ("CPIO archive", b"070707"),
    )
    for name, signature in signatures:
        if raw.startswith(signature):
            return name
    if raw.startswith(GZIP_MAGIC) and not raw[3] & 0xE0:
        return "gzip-encoded archive"
    if raw.startswith(b"RIFF"):
        return "RIFF media"
    if raw.startswith(b"FORM") and raw[8:12] in (b"AIFF", b"AIFC"):
        return "AIFF audio"
    if raw.startswith(b"BM") and len(raw) >= 14 and raw[6:10] == bytes(4):
        size, offset = int.from_bytes(raw[2:6], "little"), int.from_bytes(raw[10:14], "little")
        if 14 <= offset <= size:
            return "BMP image"
    if raw.startswith((bytes([0, 0, 1, 0]), bytes([0, 0, 2, 0]))) and len(raw) >= 22:
        return "icon/cursor image"
    if raw[4:8] == b"ftyp":
        return "ISO media"
    if raw.startswith(b"SQLite format 3" + bytes(1)):
        return "SQLite database"
    if raw.lstrip().startswith(b"<svg") or raw.lstrip().startswith(b"<?xml") and b"<svg" in raw[:512]:
        return "SVG image"
    if len(raw) >= 262 and raw[257:262] == b"ustar":
        return "tar archive"
    if len(raw) >= 272 and raw[256:260] == b"SEGA" and (raw[272:275] == b"(C)" or not raw[:256].isascii()
                                                       or CONTROL_BYTES.search(raw[:256])):
        # A cartridge header: the system name at 0x100 with the copyright field at 0x110 or a binary
        # vector table before it; "SEGA" merely landing at offset 256 of a text file is not one.
        return "Genesis ROM"
    if raw[0] == 0xE0:
        # IEEE-695 MB: two length-prefixed ASCII identifiers, then the AD record.
        position = 1
        try:
            for ordinal in range(2):
                length = raw[position]
                position += 1
                if length in (0xDE, 0xDF):
                    width = 1 if length == 0xDE else 2
                    length = int.from_bytes(raw[position:position + width], "big")
                    position += width
                elif length > 0x7F:
                    break
                identifier = raw[position:position + length]
                if len(identifier) != length or ordinal == 0 and not identifier or not all(32 <= byte < 127 for byte in identifier):
                    break
                position += length
            else:
                if raw[position:position + 1] == bytes([0xEC]):
                    return "IEEE-695 object/library"
        except IndexError:
            pass
    return None


def payload_reason(raw: bytes, index: RomIndex | None, allowed: tuple[bytes, ...], origin: str,
                   budget: DecodeBudget, *, binary_limit: bool = True) -> str | None:
    kind = embedded_kind(raw, budget)
    if kind is not None:
        return f"{origin}: embedded {kind} payload"
    if index is not None and index.contains(raw, 12, allowed, evidence=True) is not None:
        return f"{origin}: ROM window in decoded bytes (binary >=12 bytes or text >={MIN_TEXT_MATCH})"
    if binary_limit and not budget.identifier_facts and len(raw) >= MIN_BINARY_PAYLOAD and not printable_text(raw):
        return (f"{origin}: decoded non-text payload contains {len(raw)} bytes "
                f"(limit {MIN_BINARY_PAYLOAD - 1}; no entropy exemption)")
    return None


def inspect_decoded(raw: bytes, index: RomIndex | None, allowed: tuple[bytes, ...],
                    budget: DecodeBudget, origin: str, path: str, depth: int,
                    *, binary_limit: bool = True) -> str | None:
    reason = payload_reason(raw, index, allowed, origin, budget, binary_limit=binary_limit)
    if reason is not None:
        return reason
    if len(raw) >= 12 and printable_text(raw):
        if depth >= MAX_ENCODING_DEPTH:
            raise DecodeLimitError("encoded-content nesting exceeds the bounded decoding depth")
        return encoded_reason(raw, index, allowed, budget, origin, path=path, depth=depth + 1)
    return None


def table_payloads(data: bytes, budget: DecodeBudget):
    """Project rectangular numeric columns; a schema-labelled index/address is annotation."""
    try:
        lines = data.decode("utf-8").splitlines()
    except UnicodeDecodeError:
        return
    rows, header, width = [], [], 0

    def flush():
        nonlocal rows, width
        if not rows:
            return []
        emitted = []
        for column in range(width):
            if column < len(header) and header[column] in ("index", "address", "offset", "row", "line"):
                continue
            for mode in range(4):
                values = [row[column][mode] for row in rows]
                raw = bytes(value & 255 for value in values if value is not None and -128 <= value <= 255)
                if raw:
                    emitted.append(raw)
        rows, width = [], 0
        for raw in dict.fromkeys(emitted):
            budget.charge(len(raw))
        return list(dict.fromkeys(emitted))

    for line in lines:
        delimiter = "\t" if "\t" in line else "," if "," in line else None
        if delimiter is None:
            yield from flush()
            continue
        try:
            fields = next(csv.reader([line], delimiter=delimiter))
        except csv.Error:
            yield from flush()
            continue
        fields = [field.strip() for field in fields]
        if fields and not fields[-1]:
            fields.pop()
        if not 2 <= len(fields) <= 32:
            yield from flush()
            continue
        if all(re.fullmatch(r"[A-Za-z_][\w -]*", field) for field in fields):
            yield from flush()
            header = [field.lower() for field in fields]
            continue
        if not all(NUMBER.fullmatch(field.encode()) for field in fields):
            yield from flush()
            continue
        if width and width != len(fields):
            yield from flush()
        width = len(fields)
        rows.append([literal_values(field.encode())[0] for field in fields])
    yield from flush()


def conventional_payloads(data: bytes, budget: DecodeBudget):
    """Bounded standard-library encodings plus addressed firmware records, not format guesses."""
    for match in re.finditer(rb"(?<![A-Za-z2-7=])[A-Za-z2-7]{20,}={0,6}(?![A-Za-z2-7=])", data):
        token = match.group()
        try:
            raw = base64.b32decode(token + b"=" * (-len(token) % 8), casefold=True)
        except (ValueError, binascii.Error):
            continue
        budget.charge(len(raw))
        yield "Base32", raw
    candidates = {match.group() for match in re.finditer(rb"\S{15,}", data)}
    candidates.update(match.group() for match in re.finditer(rb"<~[\s!-uz]+?~>", data))
    z85 = {byte: value for value, byte in enumerate(OPAQUE_ALPHABETS["Z85"])}
    for token in candidates:
        for name, decoder in (("Base85", base64.b85decode),
                              ("Ascii85", lambda value: base64.a85decode(value, adobe=value.startswith(b"<~")))):
            try:
                raw = decoder(token)
            except (ValueError, binascii.Error):
                continue
            budget.charge(len(raw))
            yield name, raw
        if len(token) % 5 == 0 and all(byte in z85 for byte in token):
            output = bytearray()
            for start in range(0, len(token), 5):
                value = 0
                for byte in token[start:start + 5]:
                    value = value * 85 + z85[byte]
                if value > 0xFFFFFFFF:
                    break
                output.extend(value.to_bytes(4, "big"))
            else:
                raw = bytes(output)
                budget.charge(len(raw))
                yield "Z85", raw
    if QP_ESCAPE.search(data) or b"=\n" in data or b"=\r\n" in data:
        raw = quopri.decodestring(data)
        budget.charge(len(raw))
        yield "quoted-printable", raw

    records, base, uu_parts = [], 0, []

    def finish_uu():
        if not uu_parts:
            return None
        raw = b"".join(uu_parts)
        uu_parts.clear()
        budget.charge(len(raw))
        return raw

    for line in data.splitlines():
        stripped = line.strip()
        if stripped.startswith(b":") and re.fullmatch(rb":[0-9a-fA-F]+", stripped):
            try:
                record = bytes.fromhex(stripped[1:].decode())
            except ValueError:
                continue
            if len(record) >= 5 and len(record) == record[0] + 5 and sum(record) & 255 == 0:
                kind, payload = record[3], record[4:-1]
                if kind == 0:
                    records.append((base + int.from_bytes(record[1:3], "big"), payload))
                elif kind in (2, 4) and len(payload) == 2:
                    base = int.from_bytes(payload, "big") << (4 if kind == 2 else 16)
        elif re.fullmatch(rb"S[123][0-9a-fA-F]+", stripped):
            try:
                record = bytes.fromhex(stripped[2:].decode())
            except ValueError:
                continue
            address_size = int(chr(stripped[1])) + 1
            if len(record) >= address_size + 2 and record[0] + 1 == len(record) and sum(record) & 255 == 255:
                records.append((int.from_bytes(record[1:1 + address_size], "big"), record[1 + address_size:-1]))
        uu_line = line.rstrip(b"\r")
        count = (uu_line[0] - 32) & 63 if uu_line else 0
        if 1 <= count <= 63 and len(uu_line) == 1 + ((count + 2) // 3) * 4 and all(32 <= byte <= 96 for byte in uu_line):
            try:
                raw = binascii.a2b_uu(uu_line)
            except binascii.Error:
                raw = None
            if raw is not None and len(raw) == count:
                uu_parts.append(raw)
                continue
        raw = finish_uu()
        if raw is not None:
            yield "uuencode", raw
    raw = finish_uu()
    if raw is not None:
        yield "uuencode", raw
    current, end = bytearray(), None
    for address, payload in sorted(records):
        if end is not None and address != end:
            raw = bytes(current)
            budget.charge(len(raw))
            yield "addressed firmware records", raw
            current.clear()
        current.extend(payload)
        end = address + len(payload)
    if current:
        raw = bytes(current)
        budget.charge(len(raw))
        yield "addressed firmware records", raw


def encoded_reason(data: bytes, index: RomIndex | None, allowed: tuple[bytes, ...],
                   budget: DecodeBudget, origin: str, decoded: bool = False, path: str = "", depth: int = 0,
                   check_opaque: bool = True) -> str | None:
    key = hashlib.sha256(data).digest()
    if check_opaque and not budget.identifier_facts and key not in budget.opaque_scanned:
        budget.opaque_scanned.add(key)
        reason = opaque_reason(path, data)
        if reason is not None:
            budget.structural_failure = True
            return reason
    if decoded:
        reason = payload_reason(data, index, allowed, origin, budget)
        if reason is not None:
            return reason
    reason = numeric_reason(data, index, allowed, budget, origin, path)
    if reason is not None:
        return reason
    if not budget.structural_failure:
        for raw in table_payloads(data, budget):
            reason = inspect_decoded(raw, index, allowed, budget, origin + ": numeric table column", path, depth)
            if reason is not None:
                return reason
            if len(raw) >= MAX_NUMERIC_RUN:
                return f"{origin}: numeric table column contains {len(raw)} byte values (limit {MAX_NUMERIC_RUN - 1})"
        for kind, raw in conventional_payloads(data, budget):
            # Bare source expressions are also valid Base85/quoted-printable. These
            # guesses retain every opaque-shape/private-ROM check, not a length veto.
            binary_limit = kind not in ("Base85", "Ascii85", "Z85", "quoted-printable")
            reason = inspect_decoded(raw, index, allowed, budget, origin + ": " + kind,
                                     path, depth, binary_limit=binary_limit)
            if reason is not None:
                return reason
    for uri in DATA_URI.finditer(data):
        raw = unquote_to_bytes(uri[2])
        if b";base64" in uri[1].lower():
            try:
                raw = base64.b64decode(raw + b"=" * (-len(raw) % 4), altchars=b"-_", validate=True)
            except (ValueError, binascii.Error):
                continue
        budget.charge(len(raw))
        reason = inspect_decoded(raw, index, allowed, budget, origin + ": data URI", path, depth)
        if reason is not None:
            return reason
    for match in HEX_RUN.finditer(data):
        parts = [m.group(1) for m in HEX_PART.finditer(match.group())]
        if sum(map(len, parts)) < 12:
            continue
        raw = bytes.fromhex(b"".join(parts).decode("ascii"))
        budget.charge(len(raw))
        reason = inspect_decoded(raw, index, allowed, budget, origin + ": hex-encoded", path, depth)
        if reason is not None:
            return reason
    for match in HEX_ESCAPE.finditer(data):
        raw = bytes.fromhex(match.group().replace(b"\\x", b"").decode("ascii"))
        budget.charge(len(raw))
        reason = inspect_decoded(raw, index, allowed, budget, origin + ": hex-escaped", path, depth)
        if reason is not None:
            return reason
    for raw in python_byte_payloads(data, budget):
        reason = inspect_decoded(raw, index, allowed, budget, origin + ": Python bytes literal", path, depth)
        if reason is not None:
            return reason
    if C_ESCAPE.search(data):
        try:
            for group in c_literals(data, budget):
                reason = inspect_decoded(group.raw, index, allowed, budget, origin + ": C string literal", path, depth)
                if reason is not None:
                    return reason
        except LiteralDecodeError:
            # Decoded text need not be C source (e.g. a parser's malformed-input
            # fixture). Raw/hex/numeric/real-language views remain checked; a
            # budget/depth failure is never caught here.
            pass
    binary_shapes: set[bytes] = set()
    for token in base64_candidates(data, binary_shapes=binary_shapes):
        if len(token) % 4 == 1:
            continue
        try:
            raw = base64.b64decode(token + b"=" * (-len(token) % 4), altchars=b"-_", validate=True)
        except (binascii.Error, ValueError):
            continue
        budget.charge(len(raw))
        reason = inspect_decoded(raw, index, allowed, budget, origin + ": Base64/URL-safe Base64",
                                 path, depth, binary_limit=token in binary_shapes)
        if reason is not None:
            return reason
    return None


def content_reasons(path: str, data: bytes, index: RomIndex | None, *,
                    inclusions: tuple[list[str], list[str]] | None = None,
                    fragment: bool = False, source: SourceContext | None = None) -> list[str]:
    reasons = []
    try:
        text = data.decode("utf-8")
        is_text = CONTROL_BYTES.search(data) is None
    except UnicodeDecodeError:
        text, is_text = "", False
    if not is_text:
        reasons.append("non-text blob (not UTF-8 source/Markdown)")
    if not fragment and PurePosixPath(path).suffix.lower() == ".md" and (not is_text or not MARKDOWN.search(text)):
        reasons.append(".md file is not recognisable Markdown (possible ROM)")
    allowed: tuple[bytes, ...] = ()
    budget = DecodeBudget()
    source = source if source is not None else SourceContext.build(path, data)
    code_source = source_is_code(path)
    structural = None if code_source else opaque_reason(path, data)
    budget.opaque_scanned.add(hashlib.sha256(data).digest())
    if structural is not None:
        reasons.append(structural)
        budget.structural_failure = True
    try:
        numeric = numeric_reason(data, index, allowed, budget, "blob", path, source=source)
        if numeric is not None:
            reasons.append(numeric)
            budget.structural_failure = True
        raw_kind = embedded_kind(data, budget)
        if raw_kind is not None:
            reasons.append(f"embedded {raw_kind} payload")
        payloads = literal_payloads(source, budget, individual=code_source)
        if code_source:
            payloads = source_payload_regions(source, budget, payloads)
        for payload in payloads:
            if payload in source.fact_payloads:
                reason = payload_reason(payload, index, allowed, "literal identifier facts", budget)
                if reason is None:
                    budget.identifier_facts = True
                    try:
                        reason = encoded_reason(payload, index, allowed, budget,
                                                "literal identifier facts", path=path, check_opaque=False)
                    finally:
                        budget.identifier_facts = False
                if reason is not None:
                    reasons.append(reason)
                continue
            reason = inspect_decoded(payload, index, allowed, budget,
                                     "source literal/comment payload", path, 0)
            if reason is not None:
                reasons.append(reason)
                break
        is_c = PurePosixPath(path).suffix.lower() in (".c", ".h")
        literals = list(c_literals(data, budget)) if is_c else []
        for group in literals:
            is_text_literal = printable_text(group.raw)
            if not is_text_literal and group.escaped_nontext > MAX_NON_TEXT_ESCAPES:
                reasons.append(f"suspicious C literal: {group.escaped_nontext} non-text escaped bytes (limit {MAX_NON_TEXT_ESCAPES})")
            reason = payload_reason(group.raw, index, allowed, "C string literal", budget)
            if reason is None:
                reason = encoded_reason(group.raw, index, allowed, budget, "C string literal",
                                        path=path, check_opaque=group.parts == 1)
            if reason is not None and reason not in reasons:
                reasons.append(reason)
        view = data
        if MARKER.search(view):
            reasons.append("generated assembly instruction marker")
        if INCBIN.search(view):
            reasons.append("assembly includes a private baserom")
        if LISTING.search(view) or ADDRESS_ASM.search(view) or OBJDUMP_HEADER.search(view) or ASM_DIFFER_HEADER.search(view):
            reasons.append("address/instruction disassembly listing")
        if index is not None:
            offset = index.contains(view, 16, allowed, evidence=True)
            if offset is not None:
                reasons.append(f"raw ROM window (binary >=16 bytes or text >={MIN_TEXT_MATCH}) at blob offset 0x{offset:x}")
        reason = None if code_source else encoded_reason(view, index, allowed, budget, "blob", path=path)
        if reason is None and is_text and not code_source:
            for payload, literal in json_payloads(text if view is data else view.decode("utf-8"), budget):
                reason = encoded_reason(payload, index, allowed, budget, "JSON string/adjacent array",
                                        decoded=True, path=path, check_opaque=literal)
                if reason is not None:
                    break
        if reason is not None:
            reasons.append(reason)
    except DecodeLimitError as error:
        reasons.append(str(error))
    reasons.extend((inclusions if inclusions is not None else rust_inclusions(path, source.data, source=source))[1])
    return reasons


def source_language(path: str) -> str:
    suffix = PurePosixPath(path).suffix.lower()
    if suffix == ".py":
        return "python"
    if suffix in (".c", ".h", ".cc", ".cpp", ".cxx", ".hpp", ".java", ".go", ".css"):
        return "c"
    if suffix == ".cs":
        return "csharp"
    if suffix in (".js", ".mjs", ".cjs", ".jsx", ".ts", ".tsx", ".html", ".json"):
        return "javascript"
    if suffix in (".toml", ".lock", ".yaml", ".yml", ".sh", ".bash"):
        return "config"
    return "rust"  # Also inspect Rust-like data presentations in text/Markdown.


SOURCE_GRAMMARS = {
    ".rs": "rust", ".py": "python", ".c": "c", ".h": "c", ".cc": "cpp", ".cpp": "cpp",
    ".cxx": "cpp", ".hpp": "cpp", ".js": "javascript", ".mjs": "javascript",
    ".cjs": "javascript", ".jsx": "javascript", ".ts": "typescript", ".tsx": "tsx",
    ".cs": "csharp",
    ".json": "json", ".toml": "toml", ".lock": "toml", ".yaml": "yaml", ".yml": "yaml",
    ".css": "css", ".html": "html", ".sh": "bash", ".bash": "bash", ".wgsl": "wgsl",
}


def parser_language(path: str) -> str:
    return SOURCE_GRAMMARS.get(PurePosixPath(path).suffix.lower(), "rust")


def source_is_code(path: str) -> bool:
    language = SOURCE_GRAMMARS.get(PurePosixPath(path).suffix.lower())
    return language is not None and language not in ("json", "toml", "yaml")


@lru_cache(maxsize=20)
def source_parser(language: str):
    try:
        from tree_sitter_language_pack import get_parser
    except ImportError as error:
        raise AuditError("source grammars are required; install tools/audit_requirements.txt with uv") from error
    try:
        return get_parser(language)
    except (LookupError, TypeError, ValueError) as error:
        raise AuditError(f"cannot load pinned source grammar {language!r}: {error}") from error


def source_tokens(data: bytes, path: str, *,
                  literal_groups: list[list[tuple[str, bytes, int]]] | None = None) -> list[tuple[str, bytes, int]]:
    """Tokenize real grammars: comments never consume literal or interpolation text."""
    literal_types = {
        "string", "string_literal", "raw_string_literal", "char_literal", "byte_literal",
        "template_string", "interpreted_string_literal", "string_value", "quoted_attribute_value",
        "double_quote_scalar", "single_quote_scalar", "ansi_c_string", "verbatim_string_literal",
        "utf8_string_literal",
    }
    interpolation_types = {"interpolation", "template_substitution", "format_expression"}
    literal_parts = {"string_content", "string_fragment", "string_start", "string_end", "regex_pattern"}
    number_types = {"integer", "float", "integer_literal", "float_literal", "number", "number_literal"}
    containers = {
        "array_expression", "tuple_expression", "list", "tuple", "expression_list",
        "array", "initializer_list", "initializer_expression", "block_sequence", "flow_sequence",
    }
    wrappers = {"flow_node", "block_node", "block_sequence_item", "argument", "parenthesized_expression"}
    member_cache = {}

    def is_comment(node) -> bool:
        return node.type == "comment" or node.type.endswith("_comment") or node.type == "shebang"

    def items(node):
        if node.type in containers:
            return [child for child in node.named_children if not is_comment(child)]
        if node.type == "token_tree" and node.children and node.children[0].type == "[":
            # A bracketed macro sequence (e.g. vec!), never a brace-delimited
            # record/object token tree containing unrelated string keys/values.
            if any(child.type in (":", "=", "=>", ";") for child in node.children):
                return None
            return [child for child in node.named_children if not is_comment(child)]
        return None

    def members(node, base: int, region: bytes, depth: int):
        if depth > MAX_JSON_DEPTH:
            raise DecodeLimitError("literal container exceeds the bounded nesting depth")
        key = (base, node.start_byte, node.end_byte, node.type)
        if key in member_cache:
            return member_cache[key]
        result = None
        children = [child for child in node.named_children if not is_comment(child)]
        if node.type in wrappers and len(children) == 1:
            result = members(children[0], base, region, depth + 1)
        elif node.type in literal_types and node.type not in ("char_literal", "byte_literal"):
            if not any(child.type in interpolation_types for child in node.children):
                value = region[node.start_byte:node.end_byte]
                result = [("string", value, base + node.start_byte)]
        else:
            sequence = items(node)
            if sequence is not None:
                combined, category = [], None
                for child in sequence:
                    child_members = members(child, base, region, depth + 1)
                    if child_members is None:
                        break
                    kinds = {literal_category(value, path) for _, value, _ in child_members}
                    if len(kinds) > 1 or (category is not None and kinds and category not in kinds):
                        break
                    category = next(iter(kinds), category)
                    combined.extend(child_members)
                else:
                    result = combined
        member_cache[key] = result
        return result

    groups = []

    def capture(node, base: int, region: bytes) -> None:
        sequence = items(node)
        if sequence is None:
            return
        run, category = [], None
        for child in sequence:
            child_members = members(child, base, region, 0)
            kinds = {literal_category(value, path) for _, value, _ in child_members or []}
            if child_members is None or len(kinds) > 1 or (category is not None and kinds and category not in kinds):
                if len(run) > 1:
                    groups.append(run)
                run, category = [], None
                if child_members is None or len(kinds) > 1:
                    continue
            category = next(iter(kinds), category)
            run.extend(child_members)
        if len(run) > 1:
            groups.append(run)

    def parse(region: bytes, language: str):
        # Python and the other grammars accept CRLF but do not all treat a bare
        # CR as a physical newline. Canonicalize it only for parsing, one byte
        # for one byte; all token values/spans still come from original input.
        canonical = re.sub(rb"\r(?!\n)", b"\n", region)
        return source_parser(language).parse(canonical).root_node

    tokens, pending = [], [(parse(data, parser_language(path)), 0, data, 0)]
    visited = 0
    while pending:
        node, base, region, depth = pending.pop()
        visited += 1
        if visited > MAX_JSON_NODES or depth > MAX_ENCODING_DEPTH:
            raise DecodeLimitError("source grammar exceeds the bounded node/injection budget")
        value = region[node.start_byte:node.end_byte]
        start = base + node.start_byte
        if not value:
            continue
        if literal_groups is not None:
            capture(node, base, region)
        if is_comment(node):
            tokens.append(("comment", value, start))
            continue
        if node.type == "regex":
            tokens.append(("regex", value, start))
            continue
        if node.type in literal_types and not any(child.type in interpolation_types for child in node.children):
            tokens.append(("char" if node.type in ("char_literal", "byte_literal") else "string", value, start))
            continue
        if node.type == "line_continuation":
            continue
        # C preprocessor argument bodies and HTML script/style bodies are source,
        # not quoted data. Re-tokenize them without expanding or executing macros.
        embedded = None
        if node.type == "preproc_arg" and len(value) < len(region):
            embedded = parser_language(path)
        elif node.type == "raw_text" and node.parent is not None:
            embedded = {"script_element": "javascript", "style_element": "css"}.get(node.parent.type)
        if embedded is not None:
            pending.append((parse(value, embedded), start, value, depth + 1))
            continue
        if node.child_count:
            pending.extend((child, base, region, depth) for child in reversed(node.children))
            continue
        if node.type in literal_parts:
            kind = "literal_part"
        elif node.type == "plain_value" or (node.type == "word" and node.parent is not None
                                           and node.parent.type != "command_name"):
            kind = "data"
        elif node.type in number_types:
            kind = "number"
        elif node.type.endswith("identifier") or RUST_IDENTIFIER.fullmatch(value.removeprefix(b"r#")):
            kind = "identifier"
        else:
            kind = "punctuation"
        tokens.append((kind, value, start))
    if literal_groups is not None:
        # A fully literal outer container already covers its nested containers;
        # do not repeatedly decode the same bytes or grow the budget quadratically.
        last_end = -1
        for group in sorted(groups, key=lambda group: (group[0][2], -(group[-1][2] + len(group[-1][1])))):
            end = group[-1][2] + len(group[-1][1])
            if end <= last_end:
                continue
            literal_groups.append(group)
            last_end = end
    return tokens


def source_lines(data: bytes) -> list[bytes]:
    """Git diffs and changed-line provenance count LF, not every control byte."""
    parts = data.split(b"\n")
    return [part + b"\n" for part in parts[:-1]] + ([parts[-1]] if parts[-1] else [])


@dataclass
class SourceContext:
    path: str
    data: bytes
    tokens: list[tuple[str, bytes, int]]
    changed_lines: set[int] | None
    code: bytes
    changed_ranges: tuple[tuple[int, int], ...]
    literal_groups: list[list[tuple[str, bytes, int]]]
    fact_payloads: set[bytes]

    @classmethod
    def build(cls, path: str, data: bytes, changed_lines: set[int] | None = None) -> SourceContext:
        try:
            data.decode("utf-8")
            groups = []
            tokens = source_tokens(data, path, literal_groups=groups) if CONTROL_BYTES.search(data) is None else []
        except UnicodeError:
            tokens = []
            groups = []
        code = bytearray(data)
        for kind, value, start in tokens:
            if kind == "comment":
                code[start:start + len(value)] = bytes(byte if byte in (10, 13) else 32 for byte in value)
        ranges = []
        if changed_lines is not None:
            start = 0
            for line, value in enumerate(source_lines(bytes(code)), 1):
                if line in changed_lines:
                    if ranges and ranges[-1][1] == start:
                        ranges[-1] = (ranges[-1][0], start + len(value))
                    else:
                        ranges.append((start, start + len(value)))
                if line not in changed_lines:
                    code[start:start + len(value)] = bytes(byte if byte in (10, 13) else 32 for byte in value)
                start += len(value)
        return cls(path, data, tokens, changed_lines, bytes(code), tuple(ranges), groups, set())

    def touched(self, start: int, end: int) -> bool:
        if self.changed_lines is None:
            return True
        return any(start < last and first < end for first, last in self.changed_ranges)

    def code_touched(self, start: int, end: int) -> bool:
        return self.changed_lines is None or bool(self.code[start:end].strip())


def literal_category(value: bytes, path: str) -> str:
    """Only ordered homogeneous text/byte literal runs are aggregated."""
    quote = min((position for marker in (b'"', b"'") if (position := value.find(marker)) >= 0), default=0)
    prefix = value[:quote].lower()
    return "bytes" if prefix.startswith((b"b", b"rb")) or (path.endswith(".cs") and value.endswith(b"u8")) else "text"


def source_literal(value: bytes, path: str) -> bytes | None:
    """Decode one lexical string/character literal without evaluating source code."""
    language = source_language(path)
    if language == "csharp" and value.endswith(b"u8"):
        value = value[:-2]
    raw = RUST_RAW_START.match(value) if language == "rust" else None
    if raw:
        closing = b'"' + raw[1]
        return value[raw.end():-len(closing)] if value.endswith(closing) else None
    positions = [(position, quote) for quote in (b'"', b"'", b"`")
                 if (position := value.find(quote)) >= 0]
    if not positions:
        return None
    position, quote = min(positions)
    prefix = value[:position]
    if language == "c" and prefix.endswith(b"R"):
        opening = value.find(b"(", position + 1)
        closing = b")" + value[position + 1:opening] + b'"'
        return value[opening + 1:-len(closing)] if opening >= 0 and value.endswith(closing) else None
    triple = value.startswith(quote * 3, position)
    delimiter = quote * (3 if triple else 1)
    if not value.endswith(delimiter):
        return None
    body = value[position + len(delimiter):-len(delimiter)]
    if language == "csharp":
        if b"$" in prefix:
            return None
        if b"@" in prefix:
            return body.replace(b'""', b'"')
        if triple:
            return body
    if language == "python":
        if b"f" in prefix.lower():
            return None  # Interpolated expressions are not literal assembly.
        if b"b" in prefix.lower():
            return python_byte_literal(value.decode("utf-8"))
        body = body.replace(b"\r\n", b"\n").replace(b"\r", b"\n")
        if b"r" in prefix.lower():
            return body
    if language == "config" and quote == b"'":
        return body.replace(b"''", b"'") if PurePosixPath(path).suffix in (".yaml", ".yml") else body
    if quote == b"`" and b"${" in body:
        return None
    if b"\\" not in body:
        return body
    try:
        return decode_c_literal(
            b'"' + body + b'"', rust=language == "rust",
            python=language in ("python", "javascript"),
            unicode_text=language in ("python", "javascript", "csharp"),
            csharp=language == "csharp")[0]
    except DecodeLimitError:
        return None


def literal_payloads(source: SourceContext, budget: DecodeBudget, *, individual: bool = False):
    """Fold adjacent literals and bounded literal-only concat! trees."""
    for group in source.literal_groups:
        parts = []
        for _, value, start in group:
            if not source.touched(start, start + len(value)):
                continue
            payload = source_literal(value, source.path)
            if payload is None:
                break
            budget.charge(len(payload))
            parts.append(payload)
        else:
            if parts:
                budget.charge(sum(map(len, parts)))
                combined = b"".join(parts)
                # Names such as d0/a0 are hex-shaped only by coincidence inside
                # an otherwise ordinary field-name list. Reject whole hex streams
                # and substantial hex/byte fragments, not each tiny name in isolation.
                hex_parts = [part for part in parts if re.fullmatch(rb"[0-9a-fA-F]+", part)]
                text_literals = all(literal_category(value, source.path) == "text"
                                    for _, value, _ in group)
                if (text_literals and len(hex_parts) != len(parts)
                        and not any(len(part) >= 4 for part in hex_parts)
                        and all(re.fullmatch(rb"[A-Za-z_][A-Za-z0-9_.-]*", part)
                                and not opaque_component(part) for part in parts)):
                    source.fact_payloads.add(combined)
                yield combined
    tokens = [token for token in source.tokens if token[0] != "comment"]
    pairs, stack = {}, []
    closers = {b"(": b")", b"[": b"]", b"{": b"}"}
    strict = PurePosixPath(source.path).suffix.lower() == ".rs"
    for index, (kind, value, _) in enumerate(tokens):
        if kind != "punctuation":
            continue
        if value in closers:
            stack.append((index, closers[value]))
        elif stack and value == stack[-1][1]:
            opening, _ = stack.pop()
            pairs[opening] = index

    def join(parts: list[bytes | None]) -> bytes:
        known = [part for part in parts if part is not None]
        budget.charge(sum(map(len, known)))
        return b"".join(known)

    def number_text(value: bytes) -> bytes:
        spelling = re.sub(rb"f(?:32|64)$", b"", INT_SUFFIX.sub(b"", value)).replace(b"_", b"")
        if spelling.lower().startswith((b"0x", b"0o", b"0b")):
            return str(int(spelling, 0)).encode()
        return spelling

    def atom(index: int, depth: int, numbers: bool = False):
        if depth > MAX_JSON_DEPTH:
            raise DecodeLimitError("source literal assembly exceeds the bounded nesting depth")
        if index >= len(tokens):
            return None
        kind, value, start = tokens[index]
        if kind in ("string", "char"):
            if not source.touched(start, start + len(value)):
                return index + 1, b""
            payload = source_literal(value, source.path)
            if payload is None:
                return None
            budget.charge(len(payload))
            return index + 1, payload
        if numbers and (kind == "number" or (kind == "identifier" and value in (b"true", b"false"))):
            payload = number_text(value) if kind == "number" else value
            return index + 1, payload if source.touched(start, start + len(value)) else b""
        if (numbers and kind == "punctuation" and value in (b"+", b"-")
                and index + 1 < len(tokens) and tokens[index + 1][0] == "number"):
            _, spelling, following = tokens[index + 1]
            payload = value + number_text(spelling)
            return index + 2, payload if source.touched(start, following + len(spelling)) else b""
        if (numbers and kind == "identifier" and value.removeprefix(b"r#") == b"env"
                and index + 2 < len(tokens) and tokens[index + 1][:2] == ("punctuation", b"!")):
            end = pairs.get(index + 2)
            if end is not None:
                arguments = tokens[index + 3:end]
                if arguments and arguments[-1][:2] == ("punctuation", b","):
                    arguments = arguments[:-1]
                metadata = {
                    b"CARGO_PKG_VERSION", b"CARGO_PKG_VERSION_MAJOR", b"CARGO_PKG_VERSION_MINOR",
                    b"CARGO_PKG_VERSION_PATCH", b"CARGO_PKG_VERSION_PRE", b"CARGO_PKG_NAME",
                    b"CARGO_PKG_AUTHORS", b"CARGO_PKG_DESCRIPTION", b"CARGO_PKG_HOMEPAGE",
                    b"CARGO_PKG_REPOSITORY", b"CARGO_PKG_LICENSE", b"CARGO_PKG_LICENSE_FILE",
                    b"CARGO_PKG_RUST_VERSION", b"CARGO_PKG_README", b"CARGO_CRATE_NAME",
                }
                if (len(arguments) == 1 and arguments[0][0] == "string"
                        and source_literal(arguments[0][1], source.path) in metadata):
                    # Cargo manifest metadata is an audited public fact, not an
                    # evaluated environment value. None marks the fact explicitly;
                    # literal bytes on either side still form one checked stream.
                    return end + 1, None
        macro = (kind == "identifier" and value.removeprefix(b"r#") == b"concat"
                 and index + 2 < len(tokens) and tokens[index + 1][:2] == ("punctuation", b"!")
                 and tokens[index + 2][1] in closers)
        if not macro:
            return None
        opening = index + 2
        end = pairs.get(opening)
        if end is None:
            if strict and source.code_touched(start, len(source.data)):
                raise DecodeLimitError("unresolved concat! literal assembly")
            return None
        if not source.code_touched(start, tokens[end][2] + 1):
            return end + 1, b""
        parts, cursor = [], opening + 1
        while cursor < end:
            parsed = atom(cursor, depth + 1, numbers=True)
            if parsed is None:
                if strict and source.code_touched(start, tokens[end][2] + 1):
                    raise DecodeLimitError("concat! must contain only bounded literal arguments")
                return end + 1, b""
            cursor, payload = parsed
            parts.append(payload)
            if cursor < end:
                if tokens[cursor][:2] != ("punctuation", b","):
                    if strict and source.code_touched(start, tokens[end][2] + 1):
                        raise DecodeLimitError("concat! must contain only bounded literal arguments")
                    return end + 1, b""
                cursor += 1
        return end + 1, join(parts)

    index = 0
    while index < len(tokens):
        composed = tokens[index][0] == "identifier" and tokens[index][1].removeprefix(b"r#") == b"concat"
        parsed = atom(index, 0)
        if parsed is None:
            index += 1
            continue
        index, payload = parsed
        parts = [payload]
        while index < len(tokens):
            following = index + 1 if tokens[index][:2] == ("punctuation", b"+") else index
            parsed = atom(following, 0)
            if parsed is None:
                break
            index, payload = parsed
            parts.append(payload)
        if individual or composed or len(parts) > 1:
            payload = join(parts)
            if payload:
                yield payload

def source_payload_regions(source: SourceContext, budget: DecodeBudget, literals):
    """Encoding guesses apply to literal/comment data, never identifier/punctuation code."""
    yield from literals
    comments, previous_end = [], None

    def comment_body(value: bytes) -> bytes:
        if value.startswith(b"<!--"):
            return value[4:-3]
        if value.startswith(b"/*"):
            body = value[2:-2]
            if body[:1] in (b"*", b"!"):
                body = body[1:]
            return re.sub(rb"(?m)^[ \t]*\*(?:[ \t]?)", b"", body)
        if value.startswith(b"//"):
            body = value[2:]
            return body[1:] if body[:1] in (b"/", b"!") else body
        return value[1:] if value.startswith(b"#") else value

    for kind, value, start in source.tokens:
        if kind == "comment":
            if previous_end is not None and source.data[previous_end:start].strip():
                if comments:
                    yield b"\n".join(comments)
                    comments.clear()
            previous_end = start + len(value)
            if source.touched(start, previous_end):
                body = comment_body(value)
                budget.charge(len(body))
                comments.append(body)
        elif kind in ("literal_part", "data") and source.touched(start, start + len(value)):
            budget.charge(len(value))
            yield value
    if comments:
        yield b"\n".join(comments)



def rust_inclusions(path: str, data: bytes, *, changed_lines: set[int] | None = None,
                    source: SourceContext | None = None) -> tuple[list[str], list[str]]:
    """Every inclusion builtin token must be a direct tracked-public literal call."""
    targets, reasons = [], []
    if PurePosixPath(path).suffix.lower() != ".rs":
        return targets, reasons
    source = source if source is not None else SourceContext.build(path, data, changed_lines)
    tokens = [token for token in source.tokens if token[0] != "comment"]
    closing = {b"(": b")", b"[": b"]", b"{": b"}"}
    for index, (kind, name, start) in enumerate(tokens):
        name = name.removeprefix(b"r#")
        if kind != "identifier" or name not in (b"include_bytes", b"include_str", b"include"):
            continue
        macro = name.decode() + "!"
        if index + 2 >= len(tokens) or tokens[index + 1][:2] != ("punctuation", b"!"):
            if source.touched(start, start + len(name)):
                reasons.append(f"{macro}: unresolved inclusion builtin identifier is forbidden "
                               "(including forwarding, aliases and macro arguments)")
            continue
        opening = tokens[index + 2][1]
        stack = [closing[opening]] if opening in closing else []
        end = index + 3
        while stack and end < len(tokens):
            token_kind, value, _ = tokens[end]
            if token_kind == "punctuation":
                if value in closing:
                    stack.append(closing[value])
                elif value == stack[-1]:
                    stack.pop()
            if stack:
                end += 1
        last = tokens[end][2] + len(tokens[end][1]) if end < len(tokens) else len(data)
        if not source.code_touched(start, last):
            continue
        arguments = tokens[index + 3:end]
        if arguments and arguments[-1][:2] == ("punctuation", b","):
            arguments = arguments[:-1]
        if stack or opening not in closing or len(arguments) != 1 or arguments[0][0] != "string":
            reasons.append(f"{macro}: compile-time inclusion requires a direct tracked public path; "
                           "indirect or unresolved expressions are forbidden")
            continue
        literal = arguments[0][1]
        try:
            if literal.startswith(b'"'):
                raw = decode_c_literal(literal, rust=True)[0]
            elif literal.startswith(b"r"):
                quote = literal.index(b'"')
                raw = literal[quote + 1:len(literal) - quote]
            else:
                raise ValueError("path must be a Rust string literal")
            target = raw.decode("utf-8").replace("\\", "/")
            if not target or "\0" in target or target.startswith("/") or re.match(r"^[A-Za-z]:", target):
                raise ValueError("path must be relative, nonempty and NUL-free")
            target = posixpath.normpath(str(PurePosixPath(path).parent / target))
            if target == ".." or target.startswith("../"):
                raise ValueError("path escapes the repository")
        except (DecodeLimitError, UnicodeError, ValueError) as error:
            reasons.append(f"{macro}: invalid compile-time inclusion path: {error}")
            continue
        if path_reasons(target, 0, ""):
            reasons.append(f"{macro}: compile-time inclusion of a private/binary path is forbidden: {target!r}")
        else:
            targets.append(target)
    return targets, reasons


def inclusion_context_reasons(targets: list[str], tracked: set[str]) -> list[str]:
    return [f"compile-time Rust inclusion targets an untracked, missing or symlink input: {target!r}"
            for target in targets if target not in tracked]


def path_reasons(path: str, size: int, mode: str) -> list[str]:
    p = PurePosixPath(path)
    reasons = []
    if any(part.lower() in PRIVATE_DIRS for part in p.parts[:-1]):
        reasons.append("file under a private/generated directory")
    if p.suffix.lower() in ASSET_SUFFIXES or p.name.lower().startswith("baserom"):
        reasons.append("ROM/binary/archive/asset filename")
    limit = MAX_BYTES
    if size > limit:
        reasons.append(f"blob exceeds {limit // 1_000_000} MB ({size} bytes)")
    if mode == "160000":
        reasons.append("gitlink cannot be audited as a public text blob")
    return reasons


class BlobReader:
    """One persistent cat-file process; do not materialise giant historical blobs."""

    def __init__(self, repo: Path):
        self.proc = subprocess.Popen(["git", "-C", str(repo), "cat-file", "--batch"],
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE)

    def read(self, oid: str) -> bytes:
        self.proc.stdin.write(oid.encode() + b"\n")
        self.proc.stdin.flush()
        header = self.proc.stdout.readline().split()
        if len(header) != 3 or header[1] != b"blob":
            raise AuditError(f"cannot read Git blob {oid}")
        size = int(header[2])
        data = self.proc.stdout.read(size)
        if len(data) != size or self.proc.stdout.read(1) != b"\n":
            raise AuditError(f"truncated Git blob {oid}")
        return data

    def close(self) -> None:
        self.proc.stdin.close()
        self.proc.stdout.close()
        if self.proc.wait() != 0:
            raise AuditError("git cat-file failed")


def reference_images(repo: Path) -> list[bytes]:
    """Private mode requires the same complete cartridge identities as tje-rom."""
    references = (
        ("REV 02", "rev02.bin", "85e8d0a4fac591b25b77c35680ac4175976f251b"),
        ("REV 00", "rev00.bin", "7f82d8b57fff88bdca5d8aff85b01e231dc1239a"),
    )
    images = []
    for revision, filename, digest in references:
        path = repo / "roms" / filename
        try:
            if path.stat().st_size != 1024 * 1024:
                raise AuditError(f"{revision}: reference ROM must be 1048576 bytes: {path}")
            image = path.read_bytes()
        except OSError as error:
            raise AuditError(
                f"{revision}: cannot load required private reference ROM {path}: {error}; "
                "only --no-rom skips comparison"
            ) from error
        if hashlib.sha1(image).hexdigest() != digest:
            raise AuditError(f"{revision}: reference ROM SHA-1 differs from the pinned checksum: {path}")
        images.append(image)
    return images


def git_blob_id(data: bytes) -> str:
    """Upstream uses Git SHA-1, independently of the audited repo's object format."""
    return hashlib.sha1(f"blob {len(data)}\0".encode() + data).hexdigest()


def safe_vendor_path(value: object) -> str:
    if not isinstance(value, str):
        raise AuditError("vendor manifest paths must be strings")
    path = PurePosixPath(value)
    if (not value or value == "." or path.is_absolute() or ".." in path.parts
            or str(path) != value or "\\" in value or ":" in value
            or not all(character.isprintable() for character in value)):
        raise AuditError("vendor manifest contains a noncanonical relative path")
    return value


class UpstreamRepository:
    """Fetch and verify the one pinned public tree; cache only under ignored .cache."""

    def __init__(self, repo: Path):
        self.cache = repo / ".cache" / "public-audit" / "jgenesis.git"
        self.command = ["git", "--no-replace-objects", "--git-dir", str(self.cache)]
        if not self.cache.exists():
            self.cache.parent.mkdir(parents=True, exist_ok=True)
            self.run("init", "--bare", str(self.cache), initialize=True)
        probe = self.run("cat-file", "-t", UPSTREAM_REVISION, check=False)
        if probe.returncode != 0 or probe.stdout.strip() != b"commit":
            self.run("fetch", "--depth=1", "--no-tags", UPSTREAM_URL, UPSTREAM_REVISION)
            self.run("update-ref", "refs/tje/upstream", UPSTREAM_REVISION)
        # A cached object name alone is not evidence: check the complete pinned tree
        # and object hashes, with replacement objects disabled.
        self.run("fsck", "--strict", "--no-reflogs", UPSTREAM_REVISION)
        self.files: dict[str, str] = {}
        for entry in self.run("ls-tree", "-r", "-z", UPSTREAM_REVISION).stdout.split(b"\0"):
            if not entry:
                continue
            meta, path = entry.split(b"\t", 1)
            mode, kind, oid = meta.decode().split()
            if kind == "blob" and mode in ("100644", "100755"):
                self.files[os.fsdecode(path)] = oid
        self.blobs: dict[str, bytes] = {}

    def run(self, *args: str, check: bool = True,
            initialize: bool = False) -> subprocess.CompletedProcess:
        command = ["git", *args] if initialize else [*self.command, *args]
        try:
            result = subprocess.run(command, capture_output=True, timeout=120)
        except (OSError, subprocess.TimeoutExpired) as error:
            raise AuditError(f"pinned jgenesis upstream is unavailable: {error}") from error
        if check and result.returncode:
            detail = result.stderr.decode("utf-8", "replace").strip()
            raise AuditError(f"cannot verify pinned jgenesis upstream: {detail}")
        return result

    def blob(self, oid: str) -> bytes:
        if oid not in self.blobs:
            data = self.run("cat-file", "blob", oid).stdout
            if git_blob_id(data) != oid:
                raise AuditError("cached upstream blob does not match its Git object id")
            self.blobs[oid] = data
        return self.blobs[oid]


@dataclass(frozen=True)
class VendorEntry:
    upstream_path: str | None
    upstream_blob: str | None
    modified: bool


class VendorAudit:
    """Only verified upstream bytes are exempt; local bytes always remain in scope."""

    def __init__(self, repo: Path, manifest: bytes):
        self.upstream = UpstreamRepository(repo)
        self.entries = self.validate_manifest(manifest)

    def validate_manifest(self, data: bytes) -> dict[str, VendorEntry]:
        if len(data) > MAX_BYTES:
            raise AuditError("vendor manifest exceeds the public blob size limit")
        def unique_object(pairs: list[tuple[str, object]]) -> dict:
            result = {}
            for key, value in pairs:
                if key in result:
                    raise AuditError("vendor manifest contains a duplicate JSON key")
                result[key] = value
            return result

        try:
            document = json.loads(data, object_pairs_hook=unique_object)
        except (ValueError, UnicodeError, RecursionError) as error:
            raise AuditError("vendor manifest is not bounded valid JSON") from error
        if (not isinstance(document, dict)
                or set(document) != {"schema", "url", "revision", "files"}
                or type(document["schema"]) is not int or document["schema"] != 1
                or document["url"] != UPSTREAM_URL or document["revision"] != UPSTREAM_REVISION
                or not isinstance(document["files"], dict)):
            raise AuditError("vendor manifest must describe the pinned jgenesis URL, revision and schema")
        entries = {}
        for relative, spec in document["files"].items():
            relative = safe_vendor_path(relative)
            if (not isinstance(spec, dict)
                    or set(spec) != {"upstream_path", "upstream_blob", "modified"}
                    or type(spec["modified"]) is not bool):
                raise AuditError(f"invalid vendor manifest entry for {relative!r}")
            path, oid = spec["upstream_path"], spec["upstream_blob"]
            if path is None and oid is None:
                # This project-authored notice has no upstream counterpart and gets
                # no exemption: its complete contents are audited normally.
                if relative != "NOTICE.md" or spec["modified"] is not True:
                    raise AuditError("only the local NOTICE.md may lack an upstream baseline")
            else:
                path = safe_vendor_path(path)
                if (not isinstance(oid, str) or not re.fullmatch(r"[0-9a-f]{40}", oid)
                        or self.upstream.files.get(path) != oid):
                    raise AuditError(f"vendor entry {relative!r} does not match the pinned upstream tree")
            entries[relative] = VendorEntry(path, oid, spec["modified"])
        return entries

    def content(self, path: str, data: bytes, mode: str) -> tuple[bytes | None, set[int] | None]:
        relative = path.removeprefix(VENDOR_PREFIX)
        if relative not in self.entries:
            raise AuditError(f"vendored file {relative!r} is absent from the verified manifest")
        if mode not in ("100644", "100755"):
            raise AuditError("vendored inputs must be regular tracked files, not symlinks/gitlinks")
        entry = self.entries[relative]
        if entry.upstream_blob is None:
            return data, None
        if git_blob_id(data) == entry.upstream_blob:
            return None, None
        baseline = self.upstream.blob(entry.upstream_blob)
        before, after = source_lines(baseline), source_lines(data)
        matcher = difflib.SequenceMatcher(None, before, after, autojunk=False)
        additions, lines = [], set()
        for kind, _, _, start, end in matcher.get_opcodes():
            if kind in ("insert", "replace"):
                additions.extend(after[start:end])
                lines.update(range(start + 1, end + 1))
        return b"".join(additions) or None, lines


def resolve_history_refs(repo: Path, publish_refs: list[str] | None) -> list[str]:
    roots = ["--all"]
    if publish_refs is not None:
        if not publish_refs:
            raise AuditError("--refs requires at least one commit reference")
        roots = []
        for ref in publish_refs:
            if not isinstance(ref, str) or not ref or ref.startswith("-"):
                raise AuditError("publication refs must be nonempty commit references, not Git options")
            resolved = git(repo, "rev-parse", "--verify", "--end-of-options", ref + "^{commit}").strip()
            roots.append(resolved.decode("ascii"))
    return roots


def history_tree_ids(repo: Path, publish_refs: list[str] | None = None) -> list[bytes]:
    if git(repo, "rev-parse", "--is-shallow-repository").strip() == b"true":
        raise AuditError("--history requires full history; fetch the shallow clone before auditing")
    roots = resolve_history_refs(repo, publish_refs)
    return git(repo, "log", "--format=%T", *roots).splitlines()


def generate_vendor_manifest(repo: Path) -> dict:
    """Record only the current HEAD/index/worktree vendor closure, never other refs."""
    repo = Path(os.fsdecode(git(repo, "rev-parse", "--show-toplevel").strip()))
    upstream = UpstreamRepository(repo)
    names = {os.fsdecode(path) for path in git(repo, "ls-files", "-z", "--", VENDOR_PREFIX).split(b"\0") if path}
    head = subprocess.run(["git", "-C", str(repo), "rev-parse", "--verify", "HEAD^{tree}"], capture_output=True)
    for tree in [head.stdout.strip()] if head.returncode == 0 else []:
        for entry in git(repo, "ls-tree", "-r", "-z", tree.decode(), "--", VENDOR_PREFIX).split(b"\0"):
            if entry:
                names.add(os.fsdecode(entry.split(b"\t", 1)[1]))
    files = {}
    for name in sorted(names - {VENDOR_MANIFEST}):
        relative = safe_vendor_path(name.removeprefix(VENDOR_PREFIX))
        path = repo / name
        if path.is_symlink():
            raise AuditError("cannot generate provenance for a vendored symlink")
        oid = upstream.files.get(relative)
        if oid is None and relative != "NOTICE.md":
            raise AuditError(f"vendored file {relative!r} has no pinned upstream counterpart")
        modified = oid is None or not path.is_file() or git_blob_id(path.read_bytes()) != oid
        files[relative] = {"upstream_path": relative if oid is not None else None,
                           "upstream_blob": oid, "modified": modified}
    return {"schema": 1, "url": UPSTREAM_URL, "revision": UPSTREAM_REVISION, "files": files}


def audit(repo: Path, history: bool = False, no_rom: bool = False,
          publish_refs: list[str] | None = None) -> tuple[list[Finding], dict]:
    started = time.perf_counter()
    history = history or publish_refs is not None
    repo = Path(os.fsdecode(git(repo, "rev-parse", "--show-toplevel").strip()))
    object_format = git(repo, "rev-parse", "--show-object-format").decode().strip()
    resolved_refs = resolve_history_refs(repo, publish_refs) if publish_refs is not None else None
    images = [] if no_rom else reference_images(repo)
    index = None if no_rom else RomIndex(images)
    refs: set[tuple[str, str, str, str]] = set()
    work_paths = set()
    index_paths: dict[str, set[str]] = {}
    tracked_by_source: dict[str, set[str]] = {}
    staged = git(repo, "ls-files", "--stage", "-z") if publish_refs is None else b""
    for entry in staged.split(b"\0"):
        if not entry:
            continue
        meta, path = entry.split(b"\t", 1)
        mode, oid, stage = meta.decode().split()
        name = os.fsdecode(path)
        work_paths.add(name)
        if set(oid) != {"0"}:
            refs.add((oid, name, f"index:{stage}", mode))
            if mode in ("100644", "100755"):
                index_paths.setdefault(f"index:{stage}", set()).add(name)
    for source, paths in index_paths.items():
        tracked_by_source[source] = paths | index_paths.get("index:0", set())
    if history and git(repo, "rev-parse", "--is-shallow-repository").strip() == b"true":
        raise AuditError("--history requires full history; fetch the shallow clone before auditing")
    # --all includes branches, tags and other refs. Deduplicate root trees, but never paths: the
    # same harmless blob at README.md and scratch/leak.txt must still be rejected at the latter.
    trees = history_tree_ids(repo, resolved_refs) if history else []
    head = subprocess.run(["git", "-C", str(repo), "rev-parse", "--verify", "HEAD^{tree}"], capture_output=True)
    if head.returncode == 0 and publish_refs is None:
        trees.append(head.stdout.strip())
    trees = sorted(set(trees))
    for tree in trees:
        source = f"tree:{tree.decode()}"
        tracked_by_source[source] = set()
        for entry in git(repo, "ls-tree", "-r", "-z", tree.decode()).split(b"\0"):
            if entry:
                meta, path = entry.split(b"\t", 1)
                mode, kind, oid = meta.decode().split()
                name = os.fsdecode(path)
                refs.add((oid, name, source, mode))
                if mode in ("100644", "100755"):
                    tracked_by_source[source].add(name)
    oids = sorted({oid for oid, _, _, mode in refs if mode != "160000"})
    sizes = {}
    if oids:
        result = subprocess.run(["git", "-C", str(repo), "cat-file", "--batch-check=%(objectname) %(objecttype) %(objectsize)"],
                                input=("\n".join(oids) + "\n").encode(), capture_output=True, check=True)
        for row in result.stdout.splitlines():
            oid, kind, size = row.decode().split()
            if kind != "blob":
                raise AuditError(f"expected a blob, got {kind}: {oid}")
            sizes[oid] = int(size)
    vendor = None
    if any(path.startswith(VENDOR_PREFIX) for _, path, _, _ in refs):
        if resolved_refs is None:
            preferred = ["index:0"]
            if head.returncode == 0:
                preferred.append(f"tree:{head.stdout.strip().decode()}")
        else:
            tips = git(repo, "show", "-s", "--format=%T", *resolved_refs).splitlines()
            preferred = [f"tree:{tree.decode()}" for tree in tips]
        manifest_oid = next(
            (oid for source in preferred for oid, path, origin, mode in refs
             if origin == source and path == VENDOR_MANIFEST and mode in ("100644", "100755")),
            None)
        if manifest_oid is None or sizes[manifest_oid] > MAX_BYTES:
            raise AuditError("vendored files require a tracked, bounded vendor/jgenesis/UPSTREAM.json; "
                             "run python tools/generate_vendor_manifest.py and stage the manifest")
        vendor = VendorAudit(repo, git(repo, "cat-file", "blob", manifest_oid))
        manifest_oids = sorted({oid for oid, path, _, _ in refs
                                if path == VENDOR_MANIFEST and oid != manifest_oid and sizes.get(oid, 0) <= MAX_BYTES})
        for oid in manifest_oids:
            try:
                entries = vendor.validate_manifest(git(repo, "cat-file", "blob", oid))
            except AuditError:
                continue  # Its own source remains a finding; invalid metadata grants nothing.
            for name, entry in entries.items():
                vendor.entries.setdefault(name, entry)

    def scan_content(path: str, data: bytes, mode: str) -> tuple[list[str], tuple[list[str], list[str]]]:
        original, changed_lines = data, None
        try:
            if vendor is not None and path.startswith(VENDOR_PREFIX):
                if path == VENDOR_MANIFEST:
                    vendor.validate_manifest(data)
                else:
                    data, changed_lines = vendor.content(path, data, mode)
                    if data is None:
                        return [], ([], [])
        except AuditError as error:
            return [f"vendor provenance: {error}"], ([], [])
        source = SourceContext.build(path, original, changed_lines)
        inclusions = rust_inclusions(path, original, source=source)
        reasons = content_reasons(path, data, index, inclusions=inclusions,
                                  fragment=changed_lines is not None, source=source)
        return reasons, inclusions

    def vendor_context(path: str, source: str, tracked: set[str]) -> list[str]:
        if (vendor is not None and path.startswith(VENDOR_PREFIX)
                and not source.startswith("tree:") and VENDOR_MANIFEST not in tracked):
            return ["vendored files require a regular tracked UPSTREAM.json in this Git state"]
        return []

    findings = []
    cache: dict[tuple[str, str, str], list[str]] = {}
    include_cache: dict[tuple[str, str, str], tuple[list[str], list[str]]] = {}
    reader = BlobReader(repo)
    try:
        for oid, path, source, mode in sorted(refs):
            size = sizes.get(oid, 0)
            reasons = path_reasons(path, size, mode)
            key = (oid, path, mode)
            if mode != "160000" and size <= MAX_BYTES:
                if key not in cache:
                    data = reader.read(oid)
                    cache[key], include_cache[key] = scan_content(path, data, mode)
                reasons += cache[key]
                reasons += inclusion_context_reasons(include_cache[key][0], tracked_by_source.get(source, set()))
                reasons += vendor_context(path, source, tracked_by_source.get(source, set()))
            findings.extend(Finding(source, path, reason) for reason in reasons)
    finally:
        reader.close()
    work_count = 0
    tracked_worktree = {
        path for paths in index_paths.values() for path in paths
        if (repo / path).is_file()
        and not any((repo / part).is_symlink()
                    for part in (PurePosixPath(path), *PurePosixPath(path).parents))
    }
    for path in sorted(work_paths):
        absolute = repo / path
        mode = "120000" if absolute.is_symlink() else "100644"
        if mode == "120000":
            data = os.fsencode(os.readlink(absolute))
        elif absolute.is_file():
            if absolute.stat().st_size > MAX_BYTES:
                findings.extend(Finding("worktree", path, reason) for reason in path_reasons(path, absolute.stat().st_size, ""))
                work_count += 1
                continue
            data = absolute.read_bytes()
        else:
            continue
        work_count += 1
        hasher = hashlib.new(object_format)
        hasher.update(f"blob {len(data)}\0".encode())
        hasher.update(data)
        key = (hasher.hexdigest(), path, mode)
        if key not in cache:
            cache[key], include_cache[key] = scan_content(path, data, mode)
        reasons = path_reasons(path, len(data), mode) + cache[key]
        reasons += inclusion_context_reasons(include_cache[key][0], tracked_worktree)
        reasons += vendor_context(path, "worktree", tracked_worktree)
        findings.extend(Finding("worktree", path, reason) for reason in reasons)
    summary = {"blobs": len(oids), "trees": len(trees), "tracked_worktree_files": work_count,
               "roms": len(images), "rom_check": index is not None, "history": history, "refs": publish_refs,
               "seconds": round(time.perf_counter() - started, 3)}
    return findings, summary


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--repo", type=Path, default=ROOT)
    parser.add_argument("--history", action="store_true", help="include every tree reachable from all Git refs")
    parser.add_argument("--refs", nargs="+", metavar="REF",
                        help="publication history: only these commit refs and their ancestry; implies --history")
    parser.add_argument("--no-rom", action="store_true", help="CI: skip only the private ROM-byte comparison")
    args = parser.parse_args(argv)
    try:
        findings, summary = audit(args.repo, args.history, args.no_rom, args.refs)
    except (AuditError, OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"audit_public: ERROR: {error}", file=sys.stderr)
        return 1
    # A finding may occur in many commits/index/worktree; report it once, retaining all origins.
    grouped: dict[tuple[str, str], set[str]] = {}
    for finding in findings:
        grouped.setdefault((finding.path, finding.reason), set()).add(finding.source)
    for (path, reason), sources in sorted(grouped.items()):
        print(f"FAIL {path!r}: {reason} [{', '.join(sorted(sources))}]")
    status = "FAIL" if findings else "PASS"
    rom_status = f"{summary['roms']} ROM(s) indexed" if summary["rom_check"] else "ROM comparison SKIPPED"
    scope = ("refs " + ", ".join(summary["refs"])) if summary["refs"] is not None else (
        "all local refs" if summary["history"] else "HEAD/index/worktree")
    print(f"audit_public: {status}; {summary['blobs']} unique Git blobs, {summary['trees']} trees, "
          f"{summary['tracked_worktree_files']} tracked worktree files; {rom_status}; "
          f"{len(grouped)} finding(s); history scope: {scope}; {summary['seconds']:.3f}s")
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main())
