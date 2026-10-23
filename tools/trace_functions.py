#!/usr/bin/env python3
"""Generate docs/FUNCTIONS.md from public Rust metadata and translation registries.

No ROM, private inventory JSON, decomp checkout, compiler or game execution is
needed. Uses the Rust grammar already pinned in tools/audit_requirements.txt.
"""

from __future__ import annotations

import argparse
import html
import json
import re
import sys
from collections import Counter, defaultdict
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterator

ROOT = Path(__file__).resolve().parents[1]
CATALOGUED_ENTRIES = 648
GAME_SOURCE = Path("crates/tje-game/src")
DECOMP_URL = "https://github.com/mschienbein/toejam-earl-decomp/blob/main/"
KINDS = {
    "compiled_c": "compiled C",
    "handwritten_asm": "hand-written asm",
    "runtime_helper": "runtime helper",
    "not_reproducible_c": "collision query",
}


class TraceError(ValueError):
    """Public source cannot be mapped without an ambiguous or missing identity."""


def rust_parser() -> Any:
    try:
        from tree_sitter_language_pack import get_parser
    except ImportError as error:
        raise TraceError(
            "Rust grammar required; install tools/audit_requirements.txt, or run "
            "uv run --python 3.12 --with-requirements tools/audit_requirements.txt "
            "python tools/trace_functions.py"
        ) from error
    return get_parser("rust")


def text(node: Any) -> str:
    return node.text.decode("utf-8")


def integer(node: Any) -> int:
    return int(text(node).replace("_", ""), 0)


def fields(node: Any) -> dict[str, Any]:
    body = node.child_by_field_name("body")
    if body is None:
        raise TraceError(f"expected a struct body: {text(node)}")
    return {
        text(child.child_by_field_name("field")): child.child_by_field_name("value")
        for child in body.named_children if child.type == "field_initializer"
    }


def comment(node: Any) -> str:
    raw = text(node)
    if node.type == "line_comment":
        return re.sub(r"^//[/!]?\s?", "", raw).strip()
    return " ".join(re.sub(r"^\s*\* ?", "", line).strip()
                    for line in raw.removeprefix("/*").removesuffix("*/").splitlines())


def preceding_comments(node: Any) -> str:
    parts = []
    previous = node.prev_named_sibling
    while previous is not None and previous.type in {
        "line_comment", "block_comment", "attribute_item"
    }:
        if previous.type != "attribute_item":
            parts.append(comment(previous))
        previous = previous.prev_named_sibling
    return " ".join(reversed(parts))


def attributes(node: Any) -> str:
    parts = []
    previous = node.prev_named_sibling
    while previous is not None and previous.type in {
        "line_comment", "block_comment", "attribute_item"
    }:
        if previous.type == "attribute_item":
            parts.append(text(previous))
        previous = previous.prev_named_sibling
    return " ".join(parts)


def is_test_only(node: Any) -> bool:
    return bool(re.search(r"#\[\s*(?:test\s*\]|cfg\s*\(\s*test\s*\))", attributes(node)))


def array(node: Any) -> Any:
    value = node.child_by_field_name("value")
    if value is not None and value.type == "reference_expression":
        value = value.child_by_field_name("value")
    if value is None or value.type != "array_expression":
        raise TraceError(f"expected literal registry array: {text(node)[:100]}")
    return value


def array_entries(node: Any) -> Iterator[tuple[Any, str]]:
    """Attach inline/continuation or persistent preceding group comments to entries."""
    children = array(node).named_children
    group = preceding_comments(node)
    index = 0
    while index < len(children):
        child = children[index]
        if child.type in {"line_comment", "block_comment"}:
            parts = []
            while index < len(children) and children[index].type in {
                "line_comment", "block_comment"
            }:
                parts.append(comment(children[index]))
                index += 1
            group = " ".join(parts)
            continue
        index += 1
        inline = []
        last_row = child.end_point.row
        while index < len(children) and children[index].type in {
            "line_comment", "block_comment"
        } and (children[index].start_point.row == last_row
               or inline and children[index].start_point.row == last_row + 1):
            inline.append(comment(children[index]))
            last_row = children[index].end_point.row
            index += 1
        yield child, " ".join(inline) if inline else group


def identity(node: Any) -> str:
    match = re.fullmatch(r"(?:crate::)?ids::([A-Z][A-Z0-9_]*)", text(node))
    if match is None:
        raise TraceError(f"expected catalogued ids:: identity, got {text(node)}")
    return match[1]


@dataclass(frozen=True)
class CatalogEntry:
    index: int
    constant: str
    inventory_id: str
    name: str
    kind: str
    subsystem: str
    source: str
    source_line: int
    addresses: tuple[int, int]
    fragment: bool


@dataclass(frozen=True)
class RustFunction:
    qualified: str
    path: Path
    line: int
    documentation: str
    entry_signature: bool


@dataclass(frozen=True)
class Registration:
    constant: str
    function: str | None
    blocker: str
    module: str


@dataclass(frozen=True)
class TraceRow:
    entry: CatalogEntry
    execution_class: str
    rust: RustFunction | None
    status: str
    blocker: str


class Sources:
    """Only declared non-test modules; imports resolve registry aliases/re-exports."""

    def __init__(self, root: Path) -> None:
        self.root = root
        self.parser = rust_parser()
        self.modules: dict[str, list[Any]] = {}
        self.paths: dict[str, Path] = {}
        self.functions: dict[str, RustFunction] = {}
        self.imports: dict[str, dict[str, str]] = defaultdict(dict)
        self.globs: dict[str, list[str]] = defaultdict(list)
        self.load(root / GAME_SOURCE / "lib.rs", "")

    def parse(self, path: Path) -> Any:
        tree = self.parser.parse(path.read_bytes())
        if tree.root_node.has_error:
            raise TraceError(f"cannot parse Rust source: {path.relative_to(self.root)}")
        return tree.root_node

    @staticmethod
    def qualify(module: str, reference: str) -> str:
        parts = module.split("::") if module else []
        tail = reference.split("::")
        if tail[0] == "crate":
            parts = []
            tail.pop(0)
        elif tail[0] == "self":
            tail.pop(0)
        while tail and tail[0] == "super":
            if not parts:
                raise TraceError(f"super outside parent module: {reference}")
            parts.pop()
            tail.pop(0)
        return "::".join(parts + tail)

    def use(self, module: str, node: Any, prefix: str = "") -> None:
        if node.type == "scoped_use_list":
            path = text(node.child_by_field_name("path"))
            self.use(module, node.child_by_field_name("list"), prefix + path + "::")
        elif node.type == "use_list":
            for child in node.named_children:
                if child.type not in {"line_comment", "block_comment"}:
                    self.use(module, child, prefix)
        elif node.type == "use_as_clause":
            path = prefix + text(node.child_by_field_name("path"))
            alias = text(node.child_by_field_name("alias"))
            self.imports[module][alias] = self.qualify(module, path)
        elif node.type == "use_wildcard":
            path = (prefix + text(node)).removesuffix("::*").removesuffix("::")
            self.globs[module].append(self.qualify(module, path))
        elif node.type in {"identifier", "scoped_identifier", "crate", "self", "super"}:
            path = prefix + text(node)
            if path.endswith("::self"):
                path = path.removesuffix("::self")
            self.imports[module][path.split("::")[-1]] = self.qualify(module, path)
        else:
            raise TraceError(f"unsupported Rust import: {text(node)}")

    def load(self, path: Path, module: str, body: Any = None) -> None:
        if module in self.modules:
            raise TraceError(f"duplicate non-test module: {module}")
        children = (self.parse(path) if body is None else body).named_children
        self.modules[module] = children
        self.paths[module] = path
        for node in children:
            if is_test_only(node):
                continue
            if node.type == "function_item":
                name = text(node.child_by_field_name("name"))
                qualified = self.qualify(module, name)
                parameters = text(node.child_by_field_name("parameters"))
                result = node.child_by_field_name("return_type")
                signature = ("Context" in parameters and result is not None
                             and re.sub(r"\s+", "", text(result)).endswith("Result<Option<u32>>"))
                self.functions[qualified] = RustFunction(
                    qualified, path.relative_to(self.root), node.start_point.row + 1,
                    preceding_comments(node), signature,
                )
            elif node.type == "use_declaration":
                self.use(module, node.child_by_field_name("argument"))
            elif node.type == "mod_item":
                name = text(node.child_by_field_name("name"))
                nested = self.qualify(module, name)
                inline = node.child_by_field_name("body")
                if inline is not None:
                    self.load(path, nested, inline)
                    continue
                # An explicit #[path] on an out-of-line module is relative to the
                # directory of the declaring file (mod-rs or not); the implicit
                # name.rs / name/mod.rs lookup uses the module's own directory.
                base = path.parent if path.stem in {"lib", "mod"} else path.with_suffix("")
                explicit = re.search(r'#\[\s*path\s*=\s*("[^"\n]+")\s*\]', attributes(node))
                candidates = ([path.parent / json.loads(explicit[1])] if explicit else
                              [base / f"{name}.rs", base / name / "mod.rs"])
                existing = [candidate for candidate in candidates if candidate.is_file()]
                if len(existing) != 1:
                    raise TraceError(f"expected one source file for module {nested}")
                self.load(existing[0], nested)

    def resolve(self, module: str, reference: str, seen: frozenset[str] = frozenset()) -> RustFunction:
        qualified = self.qualify(module, reference)
        if qualified in seen:
            raise TraceError(f"cyclic or unresolved Rust function: {qualified}")
        if qualified in self.functions:
            return self.functions[qualified]
        owner, _, leaf = qualified.rpartition("::")
        if "::" in reference and owner in self.modules:
            return self.resolve(owner, leaf, seen)
        seen = seen | {qualified}
        first, separator, rest = reference.partition("::")
        alias = self.imports[module].get(first)
        if alias is not None:
            target = alias + (separator + rest if separator else "")
            owner, _, leaf = target.rpartition("::")
            return self.resolve(owner, leaf, seen)
        matches = []
        for target in self.globs[module]:
            try:
                matches.append(self.resolve(target, reference, seen))
            except TraceError:
                continue
        matches = list({match.qualified: match for match in matches}.values())
        if len(matches) != 1:
            raise TraceError(f"expected one function for {qualified}, found {len(matches)}")
        return matches[0]

    def catalog(self, expected_count: int) -> list[CatalogEntry]:
        constants = {}
        for node in self.modules["metadata::ids"]:
            if node.type == "const_item":
                value = node.child_by_field_name("value")
                arguments = value.child_by_field_name("arguments")
                index = integer(arguments.named_children[0])
                if index in constants:
                    raise TraceError(f"duplicate metadata numeric identity: {index}")
                constants[index] = text(node.child_by_field_name("name"))
        entries = []
        for module, nodes in self.modules.items():
            if not module.startswith("metadata::part_"):
                continue
            for item in nodes:
                if item.type != "static_item" or text(item.child_by_field_name("name")) != "FUNCTIONS":
                    continue
                for node, _ in array_entries(item):
                    values = fields(node)
                    index = integer(values["id"].child_by_field_name("arguments").named_children[0])
                    string = lambda key: json.loads(text(values[key]))
                    classification = string("classification")
                    if classification not in KINDS:
                        raise TraceError(f"unknown metadata classification: {classification}")
                    entries.append(CatalogEntry(
                        index, constants[index], string("inventory_id"), string("name"),
                        KINDS[classification], string("subsystem"), string("source"),
                        integer(values["source_line"]),
                        tuple(integer(child) for child in values["addresses"].named_children),
                        string("kind") == "fragment",
                    ))
        entries.sort(key=lambda entry: entry.index)
        if ([entry.index for entry in entries] != list(range(expected_count))
                or set(constants) != set(range(expected_count))
                or len({entry.inventory_id for entry in entries}) != expected_count):
            raise TraceError(f"expected exactly {expected_count} unique contiguous catalogue entries")
        return entries

    def rows(self, expected_count: int = CATALOGUED_ENTRIES) -> list[TraceRow]:
        catalog = self.catalog(expected_count)
        known = {entry.constant for entry in catalog}
        production: dict[str, Registration] = {}
        pending: dict[str, Registration] = {}
        blockers: dict[str, list[str]] = defaultdict(list)
        classes = {}
        production_tables = {"TRANSLATIONS", "FRAMED_TRANSLATIONS"}
        pending_tables = {"PENDING_TRANSLATIONS", "PENDING_FRAMED_TRANSLATIONS", "PENDING"}
        registry_tables = production_tables | pending_tables | {"ATOMIC_CAPABLE", "EXECUTION_CONTRACTS"}
        for module, nodes in self.modules.items():
            if module.startswith("metadata"):
                continue
            for node in nodes:
                if node.type not in {"static_item", "const_item"} or is_test_only(node):
                    continue
                name = text(node.child_by_field_name("name"))
                if name not in registry_tables and not name.endswith("_BLOCKED"):
                    continue
                for value, note in array_entries(node):
                    function = None
                    execution_class = None
                    if name == "EXECUTION_CONTRACTS":
                        parts = [child for child in value.named_children
                                 if child.type not in {"line_comment", "block_comment"}]
                        constant = identity(parts[0])
                        execution_class = text(parts[1]).split("::")[-1]
                        if execution_class not in {"Atomic", "Functional", "TimeAware"}:
                            raise TraceError(f"unknown execution class: {execution_class}")
                    elif value.type == "struct_expression":
                        values = fields(value)
                        constant = identity(values["id"])
                        function = text(values["function"])
                    else:
                        constant = identity(value)
                    if constant not in known:
                        raise TraceError(f"uncatalogued registry identity: {constant}")
                    if name == "ATOMIC_CAPABLE":
                        execution_class = "Atomic"
                    if execution_class is not None:
                        if constant in classes:
                            raise TraceError(f"duplicate execution classification: {constant}")
                        classes[constant] = execution_class
                    elif name.endswith("_BLOCKED"):
                        if note:
                            blockers[constant].append(note)
                    else:
                        registration = Registration(constant, function, note, module)
                        target = production if name in production_tables else pending
                        if constant in target and (name in production_tables or function is not None
                                                  and target[constant].function is not None):
                            raise TraceError(f"duplicate translation registration: {constant}")
                        if constant not in target or function is not None:
                            target[constant] = registration
                        if name not in production_tables and note:
                            blockers[constant].append(note)
        overlap = production.keys() & pending.keys()
        if overlap:
            raise TraceError(f"production/pending overlap: {', '.join(sorted(overlap))}")
        rows = []
        for entry in catalog:
            registration = production.get(entry.constant) or pending.get(entry.constant)
            rust = None
            if registration is not None and registration.function is not None:
                rust = self.resolve(registration.module, registration.function)
            else:
                candidates = [function for function in self.functions.values()
                              if function.entry_signature
                              and function.qualified.startswith(entry.subsystem + "::")
                              and (f"`{entry.name}`" in function.documentation
                                   or re.search(re.escape(entry.source) + r":" + str(entry.source_line)
                                                + r"\b", function.documentation))]
                if len(candidates) > 1:
                    raise TraceError(f"ambiguous source-only translation: {entry.name}")
                if candidates:
                    rust = candidates[0]
            if entry.constant in production:
                status = "production"
            elif rust is not None:
                status = "pending"
            else:
                status = "not yet written"
            notes = list(dict.fromkeys(blockers[entry.constant]))
            if status == "pending" and not notes:
                notes = ["Source body is not registered for production; no blocker comment recorded."]
            rows.append(TraceRow(entry, classes.get(entry.constant, "unclassified"), rust,
                                 status, " ".join(notes) if status != "production" else ""))
        return rows


def cell(value: str) -> str:
    return html.escape(" ".join(value.split()), quote=False).replace("|", r"\|")


def render(rows: list[TraceRow]) -> str:
    totals = Counter(row.status for row in rows)
    kinds = Counter(row.entry.kind for row in rows)
    lines = [
        "# Function traceability", "",
        "<!-- Generated by tools/trace_functions.py; edit the Rust registries/metadata, not this table. -->",
        "",
        f"All **{len(rows)} catalogued entries** are included, including the three assembly fragments.",
        "The public `crates/tje-game/src/metadata/` records are the catalogue; the private",
        "inventory JSON, decomp checkout and user ROM are not generator inputs.", "",
        "Regenerate with `python tools/trace_functions.py`; CI uses",
        "`python tools/trace_functions.py --check` (exit 1 for a missing/stale table).",
        "The Rust grammar is already pinned in `tools/audit_requirements.txt`; without an",
        "installed grammar, use `uv run --python 3.12 --with-requirements tools/audit_requirements.txt python tools/trace_functions.py`.",
        "Regenerate once after integrating translation lanes, not in each lane.", "",
        "## Reading the table", "",
        "- **Production** means present in `TRANSLATIONS` or `FRAMED_TRANSLATIONS`,",
        "  not a claim that the entire game is complete.",
        "- **Pending** means a registered or source-identified Rust ABI body exists outside",
        "  production (`PENDING_TRANSLATIONS`/`PENDING_FRAMED_TRANSLATIONS`/`PENDING`",
        "  are not production approval). Blockers",
        "  quote the applicable registry/group comments; an absent comment is explicitly identified.",
        "- **Not yet written** means no Rust ABI body was found, even if an ID-only `PENDING`",
        "  list records an unwritten prerequisite. Any recorded blocker is retained.",
        "- Rust locations resolve registry paths (including aliases/re-exports) to the defining",
        "  `module::function` and link to its source line. Pending ID-only lists use original-name",
        "  or original-source-line provenance comments on ABI-shaped Rust functions, never a guessed snake-case name.",
        "- Execution classes come from `ATOMIC_CAPABLE` and `EXECUTION_CONTRACTS`, independently",
        "  of translation status: **Atomic** needs no peripheral/IRQ progress; **Functional** preserves",
        "  ordered device transactions while quotienting transaction latency; **TimeAware** advances",
        "  real clocks/interrupts. `unclassified` is not an implicit Atomic classification.",
        "- Decomp paths/lines are metadata provenance, relative to the matching decomp repository.",
        "  Addresses are hexadecimal ROM offsets, **REV 02 first, REV 00 second**; fragments remain",
        "  part of the 648-entry denominator, not newly claimed C functions. See [REVISIONS.md](REVISIONS.md).",
        "", "## Totals by subsystem", "",
        "| Subsystem | Total | Production | Pending | Not yet written | Atomic | Functional | TimeAware | Unclassified |",
        "|---|---:|---:|---:|---:|---:|---:|---:|---:|",
    ]
    subsystems = sorted({row.entry.subsystem for row in rows})
    for subsystem in subsystems:
        subset = [row for row in rows if row.entry.subsystem == subsystem]
        statuses = Counter(row.status for row in subset)
        classes = Counter(row.execution_class for row in subset)
        values = [len(subset), *(statuses[key] for key in ("production", "pending", "not yet written")),
                  *(classes[key] for key in ("Atomic", "Functional", "TimeAware", "unclassified"))]
        lines.append(f"| [{subsystem}](#{subsystem}) | " + " | ".join(map(str, values)) + " |")
    classes = Counter(row.execution_class for row in rows)
    values = [len(rows), *(totals[key] for key in ("production", "pending", "not yet written")),
              *(classes[key] for key in ("Atomic", "Functional", "TimeAware", "unclassified"))]
    lines.extend([
        "| **Total** | " + " | ".join(f"**{value}**" for value in values) + " |", "",
        "Kinds: " + "; ".join(f"{kind}: **{kinds[kind]}**" for kind in KINDS.values()) + ".", "",
        "## Catalogue", "",
    ])
    for subsystem in subsystems:
        lines.extend([
            f"### {subsystem}", "",
            "| Name | Kind | Decomp source | REV 02 | REV 00 | Execution class | Rust location | Status / blocker |",
            "|---|---|---|---|---|---|---|---|",
        ])
        for row in rows:
            entry = row.entry
            if entry.subsystem != subsystem:
                continue
            source = entry.source + (f":{entry.source_line}" if entry.source_line else "")
            anchor = f"#L{entry.source_line}" if entry.source_line else ""
            kind = entry.kind + (" (fragment)" if entry.fragment else "")
            rust = (f"[`{row.rust.qualified}`](../{row.rust.path.as_posix()}#L{row.rust.line})"
                    if row.rust is not None else "—")
            status = row.status + (f" — {cell(row.blocker)}" if row.blocker else "")
            lines.append(
                f"| `{entry.name}` | {kind} | [`{source}`]({DECOMP_URL}{entry.source}{anchor}) "
                f"| `0x{entry.addresses[0]:06X}` | `0x{entry.addresses[1]:06X}` "
                f"| {row.execution_class} | {rust} | {status} |"
            )
        lines.append("")
    return "\n".join(lines)


def generate(root: Path = ROOT, expected_count: int = CATALOGUED_ENTRIES) -> str:
    return render(Sources(root).rows(expected_count))


def main(argv: list[str] | None = None, *, root: Path = ROOT) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="exit 1 if docs/FUNCTIONS.md is missing/stale")
    arguments = parser.parse_args(argv)
    try:
        document = generate(root)
        destination = root / "docs/FUNCTIONS.md"
        if arguments.check:
            if not destination.is_file() or destination.read_text(encoding="utf-8") != document:
                print("docs/FUNCTIONS.md is stale; run python tools/trace_functions.py", file=sys.stderr)
                return 1
            print("docs/FUNCTIONS.md is current (648 catalogued entries)")
        else:
            destination.write_text(document, encoding="utf-8", newline="\n")
            print("Generated docs/FUNCTIONS.md (648 catalogued entries)")
    except (OSError, TraceError, KeyError) as error:
        print(f"trace_functions: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
