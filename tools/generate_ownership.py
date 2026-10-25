#!/usr/bin/env python3
"""Generate the conservative world-ownership closure, never ROM table contents.

The metadata graph is augmented with C address references, indirect call sites,
assembly indirect transfers, both IRQ roots and public descriptor *schemas*.
An unresolved indirect target conservatively reaches EVERY catalogued function.
Runtime resource admission must resolve each actual callback to an explicitly
implemented typed service; a complete inventory is not gameplay admission.
"""
from __future__ import annotations

import argparse
from dataclasses import asdict
import hashlib
import json
from pathlib import Path
import re
from typing import Any, Iterator

from trace_functions import (
    CATALOGUED_ENTRIES, Sources, TraceError, array_entries, fields, integer, text,
)

ROOT = Path(__file__).resolve().parents[1]
DESTINATION = Path("crates/tje-simulation/src/ownership.generated.json")
MATH = {"ABS", "LMODT", "LDIVT", "LMULT", "ULMODT", "ULDIVT"}
IRQ = {"LEVEL6_ISR", "LEVEL4_ISR"}
ROOTS = {
    "SETUP_PLAYER", "SETUP_THE_MAIN_GAME", "SETUP_INITIAL_INVENTORY",
    "SETUP_INITIAL_EARTHLINGS", "SETUP_INITIAL_ELEVATOR", "GENERATE_LEVEL_SEEDS",
    "PROCESS_INPUT", "PROCESS_PLAYERS", "PROCESS_MOVEMENT", "PROCESS_EARTHLINGS",
    "PROCESS_ELEVATORS", "MAP_UPDATE_ITEMS_AND_PRESENTS", "PROCESS_LAND",
    "PROCESS_TOMATO_RAIN", "UPDATE_AND_QUEUE_ENTITY_SPRITES",
    "PROCESS_ENTITY_ANIMATION_MOTION", "UPDATE_SPEECH_AND_DIALOGUE",
    "HANDLE_V_BLANK", "HANDLE_H_BLANK", "MAP_LOAD_PLAYER_LEVEL", "CHANGE_LEVEL",
} | IRQ


def walk(node: Any) -> Iterator[Any]:
    yield node
    for child in node.named_children:
        yield from walk(child)


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def metadata(root: Path) -> list[dict[str, Any]]:
    sources = Sources(root)
    catalogue = sources.catalog(CATALOGUED_ENTRIES)
    values: dict[int, dict[str, Any]] = {}
    for module, nodes in sources.modules.items():
        if not module.startswith("metadata::part_"):
            continue
        for item in nodes:
            if item.type != "static_item":
                continue
            for node, _ in array_entries(item):
                value = fields(node)
                index = integer(value["id"].child_by_field_name("arguments").named_children[0])
                callees = value["callees"].child_by_field_name("value")
                abi_fields = fields(value["abi"])
                values[index] = {
                    "callees": [integer(child.child_by_field_name("arguments").named_children[0])
                                for child in callees.named_children],
                    "abi": json.loads(json.loads(text(abi_fields["details_json"]))),
                    "signature": json.loads(text(abi_fields["signature"])),
                }
    result = []
    for entry in catalogue:
        if entry.constant in MATH:
            disposition, service, abi = "read_only", "math", "registers"
        elif entry.constant in IRQ:
            disposition, service, abi = "device_audio_original", "irq-wrapper", "exception-frame"
        elif entry.subsystem == "audio":
            disposition, service, abi = "device_audio_original", "audio", "checked-audio-resource"
        else:
            disposition, service, abi = "mode_owned", entry.subsystem, None
        if disposition == "mode_owned":
            details = values[entry.index]["abi"]
            values[entry.index]["abi"] = {
                "kind": details["kind"],
                "details_source": f"crates/tje-game/src/metadata/part_{entry.index // 108}.rs",
                "original_calls": "forbidden;typed-service-only",
            }
        result.append(asdict(entry) | values[entry.index] | {
            "disposition": disposition, "service": service, "body_version": 1,
            "original_boundary": abi,
        })
    unknown = ROOTS - {entry["constant"] for entry in result}
    if unknown:
        raise TraceError(f"ownership roots absent from metadata: {sorted(unknown)}")
    return result


def source_edges(decomp: Path, entries: list[dict[str, Any]]) -> tuple[list[dict], list[dict], list[dict]]:
    from tree_sitter_language_pack import get_parser

    parser = get_parser("c")
    by_name = {entry["name"]: entry["index"] for entry in entries}
    by_file: dict[str, list[dict]] = {}
    for entry in entries:
        by_file.setdefault(entry["source"], []).append(entry)
    edges: set[tuple[int, int, str, int, str]] = set()
    indirect: set[tuple[int, str, int, str]] = set()
    hashes = []
    for source, functions in sorted(by_file.items()):
        raw = (decomp / source).read_bytes()
        hashes.append({"source": source, "sha256": digest(raw)})
        functions.sort(key=lambda entry: entry["source_line"])

        def owner(line: int) -> int | None:
            before = [entry for entry in functions if entry["source_line"] <= line]
            return before[-1]["index"] if before else None

        if source.endswith(".c"):
            tree = parser.parse(raw)
            # K&R declarations are accepted by the same grammar as metadata generation.
            for node in walk(tree.root_node):
                line = node.start_point.row + 1
                caller = owner(line)
                if caller is None:
                    continue
                if node.type == "call_expression":
                    function = node.child_by_field_name("function")
                    if function.type == "identifier" and text(function) in by_name:
                        edges.add((caller, by_name[text(function)], source, line, "source-call"))
                    else:
                        indirect.add((caller, source, line, "c-indirect-or-unresolved"))
                elif node.type == "identifier" and text(node) in by_name:
                    parent = node.parent
                    if parent is not None and parent.type == "call_expression" and parent.child_by_field_name("function") == node:
                        continue
                    target = by_name[text(node)]
                    if caller != target:
                        # Includes installed entity/IRQ callbacks, not merely calls.
                        edges.add((caller, target, source, line, "function-reference"))
        elif source.endswith(".s"):
            for line, code in enumerate(raw.decode().splitlines(), 1):
                code = code.partition("|")[0]
                caller = owner(line)
                if caller is None:
                    continue
                transfer = re.search(r"\b(jsr|jmp)\s+(.*)", code)
                if transfer and re.search(r"%a[0-7]", transfer[2]):
                    indirect.add((caller, source, line, "asm-indirect-transfer"))
                for name in re.findall(r"[A-Za-z_][A-Za-z_0-9]*", code):
                    if name in by_name and by_name[name] != caller:
                        edges.add((caller, by_name[name], source, line, "assembly-reference"))
    return ([{"caller": caller, "target": target, "source": source, "line": line, "kind": kind}
             for caller, target, source, line, kind in sorted(edges)],
            [{"caller": caller, "source": source, "line": line, "kind": kind,
              "targets": "all-catalogued-functions", "unknown_runtime_target": "reject"}
             for caller, source, line, kind in sorted(indirect)], hashes)


def descriptor_schemas(decomp: Path) -> list[dict[str, Any]]:
    result = []
    for revision in ("rev02", "rev00"):
        document = json.loads((decomp / "config" / revision / "data_units.json").read_text())
        for unit in document["units"]:
            kind = unit["kind"]
            if kind not in {"vectors", "records", "pointer_table", "animation", "music"}:
                continue
            parameters = unit.get("params", {})
            offset = 0
            pointers = []
            for field in parameters.get("fields", []):
                if field.get("type") == "pointer":
                    pointers.append({"field": field["name"], "offset": offset, "width": field["width"]})
                offset += field["width"]
            if kind == "records" and not pointers:
                continue
            result.append({"revision": revision, "symbol": unit["name"], "kind": kind,
                           "address": unit["address"], "extent": unit["size"],
                           "record_width": offset or parameters.get("entry_width", 0),
                           "pointer_fields": pointers,
                           "admission": "decode-bounded-at-runtime;resolve-code-to-typed-service;reject-unknown"})
    return sorted(result, key=lambda entry: (entry["revision"], int(entry["address"], 0), entry["symbol"]))


def closure(entries: list[dict], references: list[dict], indirect: list[dict], roots: list[int]) -> list[int]:
    graph = {entry["index"]: set(entry["callees"]) for entry in entries}
    for edge in references:
        graph[edge["caller"]].add(edge["target"])
    for site in indirect:
        graph[site["caller"]].update(graph)
    seen: set[int] = set()
    pending = list(roots)
    while pending:
        current = pending.pop()
        if current not in graph:
            raise TraceError(f"unknown ownership identity: {current}")
        if current not in seen:
            seen.add(current)
            pending.extend(sorted(graph[current] - seen, reverse=True))
    return sorted(seen)


def generate(root: Path, decomp: Path) -> str:
    entries = metadata(root)
    references, indirect, hashes = source_edges(decomp, entries)
    roots = [entry["index"] for entry in entries if entry["constant"] in ROOTS]
    irq_roots = [entry["index"] for entry in entries if entry["constant"] in IRQ]
    result = {"schema": "tje-simulation/ownership/1", "body_manifest_version": 1,
              "scope": "conservative transitive closure; indirect calls include all catalogued functions",
              "runtime_callback_policy": "unknown, interior, unimplemented or ABI-incompatible targets reject before mutation",
              "roots": roots, "irq_roots": irq_roots,
              "closure": [entries[index]["constant"] for index in closure(entries, references, indirect, roots)],
              "irq_closure": [entries[index]["constant"] for index in closure(entries, references, indirect, irq_roots)],
              "functions": entries, "source_references": references,
              "indirect_sites": indirect, "descriptor_schemas": descriptor_schemas(decomp),
              "source_identities": hashes}
    identity = digest(json.dumps(result, sort_keys=True, separators=(",", ":")).encode())
    document = {"identity_sha256": identity} | result
    # One metadata record per line: auditable without duplicating the full
    # already-public ABI documentation for functions originals may never call.
    lines = ["{"]
    for position, (key, value) in enumerate(document.items()):
        comma = "," if position + 1 < len(document) else ""
        if isinstance(value, list):
            lines.append(f"  {json.dumps(key)}: [")
            lines.extend("    " + json.dumps(entry, separators=(",", ":"), ensure_ascii=True)
                         + ("," if index + 1 < len(value) else "")
                         for index, entry in enumerate(value))
            lines.append("  ]" + comma)
        else:
            lines.append(f"  {json.dumps(key)}: {json.dumps(value, ensure_ascii=True)}{comma}")
    lines.append("}")
    return "\n".join(lines) + "\n"


def render_rust(document: dict[str, Any]) -> str:
    entries = document["functions"]
    names = {entry["index"]: entry["constant"] for entry in entries}
    references: dict[int, set[int]] = {index: set() for index in names}
    for edge in document["source_references"]:
        references[edge["caller"]].add(edge["target"])
    indirect = {site["caller"] for site in document["indirect_sites"]}
    rows = ["// Generated by tools/generate_ownership.py; metadata only.",
            "use super::{Disposition, Node, OriginalAbi};",
            "use tje_game::ids;", "", "#[rustfmt::skip]",
            "pub const IDENTITY: [u8; 32] = ["
            + ", ".join(str(byte) for byte in bytes.fromhex(document["identity_sha256"])) + "];",
            "#[rustfmt::skip]", "pub static NODES: &[Node] = &["]
    for entry in entries:
        index = entry["index"]
        if entry["disposition"] == "mode_owned":
            disposition = f"Disposition::ModeOwned({json.dumps(entry['service'])})"
        elif entry["disposition"] == "read_only":
            disposition = "Disposition::ReadOnly(OriginalAbi::Registers)"
        else:
            abi = "ExceptionFrame" if entry["constant"] in IRQ else "CheckedAudioResource"
            disposition = f"Disposition::DeviceAudio(OriginalAbi::{abi})"
        callees = ", ".join(f"ids::{names[target]}" for target in sorted(references[index]))
        rows.append(f"    Node {{ function: ids::{entry['constant']}, body_version: 1, "
                    f"disposition: {disposition}, references: &[{callees}], "
                    f"indirect: {str(index in indirect).lower()} }},")
    rows.extend(["];", "", "#[rustfmt::skip]", "pub static ROOTS: &[tje_game::FunctionId] = &["
                 + ", ".join(f"ids::{names[index]}" for index in document["roots"]) + "];",
                 "#[rustfmt::skip]", "pub static IRQ_ROOTS: &[tje_game::FunctionId] = &["
                 + ", ".join(f"ids::{names[index]}" for index in document["irq_roots"]) + "];", ""])
    return "\n".join(rows)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--decomp", type=Path, required=True)
    parser.add_argument("--check", action="store_true")
    arguments = parser.parse_args()
    generated = generate(ROOT, arguments.decomp)
    document = json.loads(generated)
    outputs = {
        ROOT / DESTINATION: generated,
        ROOT / "crates/tje-simulation/src/ownership_generated.rs": render_rust(document),
    }
    for destination, content in outputs.items():
        if arguments.check:
            if not destination.exists() or destination.read_text() != content:
                parser.error(f"ownership manifest is stale: {destination.relative_to(ROOT)}")
        else:
            destination.write_text(content)
    print(f"ownership: {len(document['closure'])} functions, {len(document['indirect_sites'])} indirect sites, "
          f"{len(document['descriptor_schemas'])} descriptor schemas; gameplay remains fail-closed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
