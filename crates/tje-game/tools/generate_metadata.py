#!/usr/bin/env python3
"""Generate public function identities/ABIs, never source listings or ROM contents."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re

from tree_sitter import Node, Parser
from tree_sitter_language_pack import get_parser



def constant(name: str) -> str:
    name = re.sub(r"([A-Z]+)([A-Z][a-z])", r"\1_\2", name)
    name = re.sub(r"([a-z0-9])([A-Z])", r"\1_\2", name)
    return re.sub(r"[^A-Za-z0-9]+", "_", name).strip("_").upper()


def quoted(value: str) -> str:
    return json.dumps(value, ensure_ascii=True)


def declarator_chain(declarator: Node) -> list[Node]:
    chain = []
    while declarator is not None:
        chain.append(declarator)
        if declarator.type == "parenthesized_declarator":
            declarator = declarator.named_children[0]
        else:
            declarator = declarator.child_by_field_name("declarator")
    return chain


def array_parameter_types(header: bytes, name: str, parser: Parser) -> dict[str, dict[str, str]]:
    """Apply C's array-parameter adjustment, not an array object's element width."""
    # Isolate the inventory-identified header. Parsing an entire K&R unit lets
    # tree-sitter mistake later identifier-list formals for typedef names.
    tree = parser.parse(header + b"{}")
    definitions = [
        node for node in tree.root_node.named_children if node.type == "function_definition"
    ]
    if tree.root_node.has_error or len(definitions) != 1:
        raise ValueError(f"cannot parse C parameter declarations for {name}")
    definition = definitions[0]
    function = definition.child_by_field_name("declarator")
    while function is not None and function.type != "function_declarator":
        function = function.child_by_field_name("declarator")
    if function is None or function.child_by_field_name("declarator").text.decode() != name:
        raise ValueError(f"C definition does not match inventory function {name}")
    parameters = function.child_by_field_name("parameters")
    declarations = [
        node for node in definition.named_children if node.type == "declaration"
    ] + [
        node for node in parameters.named_children if node.type == "parameter_declaration"
    ]
    adjusted = {}
    for declaration in declarations:
        base_nodes = [
            node for node in declaration.named_children
            if node == declaration.child_by_field_name("type") or node.type == "type_qualifier"
        ]
        base = " ".join(node.text.decode() for node in base_nodes)
        for declarator in declaration.children_by_field_name("declarator"):
            # Parentheses group declarators; only pointer/array/function nodes
            # are semantic operations adjacent to the parameter identifier.
            chain = [
                node for node in declarator_chain(declarator)
                if node.type != "parenthesized_declarator"
            ]
            if chain[-1].type != "identifier" and any(
                node.type in ("array_declarator", "abstract_array_declarator") for node in chain
            ):
                # The C grammar can interpret char (text)[4] as an unnamed
                # array of functions taking typedef text. Never silently use
                # the stale inventory scalar ABI when the named shape is lost.
                raise ValueError(f"ambiguous C array parameter declarator in {name}")
            if len(chain) < 2 or chain[-2].type != "array_declarator":
                continue
            # The operation adjacent to the identifier determines whether this
            # is an array parameter. A pointer-to-array parameter stays a pointer.
            array, identifier = chain[-2:]
            if identifier.type != "identifier":
                raise ValueError(f"unsupported array parameter declarator in {name}")
            outer_arrays = any(node.type == "array_declarator" for node in chain[:-2])
            pointer = b"(*)" if outer_arrays else b"*"
            start = array.start_byte - declarator.start_byte
            end = array.end_byte - declarator.start_byte
            abstract = declarator.text[:start] + pointer + declarator.text[end:]
            adjusted[identifier.text.decode()] = {
                "type": f"{base} {abstract.decode()}".strip(),
                "source_declaration": f"{base} {declarator.text.decode()}".strip(),
            }
    return adjusted


def source_abis(functions: list[dict], decomp: Path) -> tuple[list[dict], str]:
    parser = get_parser("c")
    sources: dict[str, list[bytes]] = {}
    declaration_hash = hashlib.sha256()
    abis = []
    for entry in functions:
        abi = entry["calling_convention"]
        if abi["kind"] == "mcc68k_c":
            path = entry["source_file"]
            if path not in sources:
                sources[path] = (decomp / path).read_bytes().splitlines(keepends=True)
            tail = b"".join(sources[path][entry["source_line"] - 1:])
            header, body, _ = tail.partition(b"{")
            if not body:
                raise ValueError(f"missing C definition body for {entry['name']}")
            arrays = array_parameter_types(header, entry["name"], parser)
            declaration_hash.update(path.encode() + b"\0" + header + b"\0")
            if arrays:
                arguments = abi["arguments"]
                unknown = arrays.keys() - {argument["name"] for argument in arguments}
                if unknown:
                    raise ValueError(f"array parameters missing from {entry['name']} ABI: {unknown}")
                abi = abi | {"arguments": [
                    argument | arrays[argument["name"]] | {
                        "declared_width_bits": 32,
                        "entry_sp_value_offset": argument["entry_sp_slot_offset"],
                    } if argument["name"] in arrays else argument
                    for argument in arguments
                ]}
        abis.append(abi)
    return abis, declaration_hash.hexdigest()




def generate(inventory: Path, decomp: Path) -> dict[str, str]:
    raw = inventory.read_bytes()
    data = json.loads(raw)
    functions = data["functions"]
    abis, declaration_hash = source_abis(functions, decomp)
    indices = {entry["id"]: index for index, entry in enumerate(functions)}
    names = {entry["name"]: index for index, entry in enumerate(functions)}
    constants = [constant(entry["name"]) for entry in functions]
    if len(set(constants)) != len(functions):
        raise ValueError("function constant name collision")
    header = (
        "// Generated by tools/generate_metadata.py; metadata only.\n"
        f"// Inventory SHA-256: {hashlib.sha256(raw).hexdigest()}\n"
        f"// C declaration SHA-256: {declaration_hash}\n"
    )
    ids = header + "use crate::FunctionId;\n\n"
    for index, name in enumerate(constants):
        ids += f"pub const {name}: FunctionId = FunctionId({index});\n"
    outputs = {"ids.rs": ids}
    chunks: list[str] = []
    for chunk_start in range(0, len(functions), 108):
        chunk_name = f"part_{chunk_start // 108}"
        chunks.append(chunk_name)
        text = header + "use crate::{Abi, FunctionId, FunctionMetadata};\n\n"
        text += "#[rustfmt::skip]\n"
        text += f"pub static FUNCTIONS: [FunctionMetadata; {len(functions[chunk_start:chunk_start + 108])}] = [\n"
        for index, entry in enumerate(functions[chunk_start:chunk_start + 108], chunk_start):
            abi = abis[index]
            result = abi.get("return", {}).get("declared_type", "register-defined")
            arguments = abi.get("arguments", [])
            signature = f"{result} {entry['name']}(" + ", ".join(
                f"{argument.get('type', argument.get('declared_type', 'register'))} {argument['name']}"
                for argument in arguments
            ) + ")"
            dependencies = []
            for dependency in entry["all_dependency_callees"]:
                if isinstance(dependency, dict):
                    dependency = dependency.get("id", dependency.get("name"))
                target = indices.get(dependency, names.get(dependency))
                if target is None:
                    raise ValueError(f"unknown dependency: {dependency}")
                dependencies.append(f"FunctionId({target})")
            preserved = abi.get("saved_registers", abi.get("preserved_registers", []))
            data_mask = sum(1 << int(register[1:]) for register in preserved if re.fullmatch(r"D[0-7]", register))
            address_mask = sum(1 << int(register[1:]) for register in preserved if re.fullmatch(r"A[0-6]", register))
            returns_value = result not in ("void",) and (abi.get("return", {}).get("register") == "D0" or "D0" in abi.get("register_outputs", {}))
            fields = {
                "id": f"FunctionId({index})",
                "inventory_id": quoted(entry["id"]),
                "name": quoted(entry["name"]),
                "addresses": "[" + ", ".join(entry["addresses"][revision] for revision in ("rev02", "rev00")) + "]",
                "sizes": "[" + ", ".join(str(entry["sizes"][revision]) for revision in ("rev02", "rev00")) + "]",
                "classification": quoted(entry["classification"]),
                "kind": quoted(entry["kind"]),
                "subsystem": quoted(entry["subsystem"]),
                "source": quoted(entry["source_file"]),
                "source_line": str(entry["source_line"]),
                "wave": str(entry["leaf_first"]["wave"]),
                "callees": "&[" + ", ".join(dependencies) + "]",
                "abi": "Abi { " + ", ".join([
                    "kind: " + quoted(abi["kind"]),
                    "signature: " + quoted(signature),
                    "details_json: " + quoted(json.dumps(abi, sort_keys=True, separators=(",", ":"))),
                    "argument_count: " + str(abi.get("argument_count", 0)),
                    "returns_value: " + str(returns_value).lower(),
                    "preserved_data: " + str(data_mask),
                    "preserved_address: " + str(address_mask),
                    "strict_registers: " + str(entry["classification"] == "handwritten_asm").lower(),
                ]) + " }",
            }
            text += "    FunctionMetadata {\n" + "".join(f"        {key}: {value},\n" for key, value in fields.items()) + "    },\n"
        text += "];\n"
        if len(text.encode()) >= 1_000_000:
            raise ValueError(f"generated module exceeds public size limit: {chunk_name}")
        outputs[f"{chunk_name}.rs"] = text
    index = header + "pub mod ids;\n" + "".join(f"mod {chunk};\n" for chunk in chunks)
    index += "\npub static FUNCTION_CHUNKS: &[&[super::FunctionMetadata]] = &[\n"
    index += "".join(f"    &{chunk}::FUNCTIONS,\n" for chunk in chunks) + "];\n"
    outputs["mod.rs"] = index
    return outputs


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inventory", type=Path, required=True)
    parser.add_argument("--decomp", type=Path, required=True)
    parser.add_argument("--check", action="store_true")
    arguments = parser.parse_args()
    destination = Path(__file__).resolve().parents[1] / "src" / "metadata"
    outputs = generate(arguments.inventory, arguments.decomp)
    if arguments.check:
        differences = [name for name, text in outputs.items() if not (destination / name).exists() or (destination / name).read_text() != text]
        if differences:
            raise SystemExit("metadata differs: " + ", ".join(differences))
    else:
        destination.mkdir(parents=True, exist_ok=True)
        for name, text in outputs.items():
            (destination / name).write_text(text)
    print(f"{'checked' if arguments.check else 'generated'} 648 entries in {len(outputs)} modules")


if __name__ == "__main__":
    main()
