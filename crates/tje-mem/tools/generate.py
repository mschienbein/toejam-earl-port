#!/usr/bin/env python3
"""Generate ROM-free views, checking every layout with the original MCC68K.

Run with G/.venv/bin/python -B; G is read-only. Only names, types, addresses,
lengths, source hashes and independently compiled synthetic probe facts leave G.
The compiler's scratch drive and this tool's receipts live outside G.
"""
from __future__ import annotations

import argparse
import ast
from dataclasses import dataclass, replace
import hashlib
import importlib
import json
from pathlib import Path
import re
import sys
import subprocess
from types import SimpleNamespace


PRIMITIVES = {"char": ("i8", 1), "unsigned char": ("u8", 1),
              "short": ("i16", 2), "unsigned short": ("u16", 2),
              "int": ("i32", 4), "unsigned int": ("u32", 4),
              "long": ("i32", 4), "unsigned long": ("u32", 4)}
KEYWORDS = {"type", "match", "loop", "ref", "self", "in", "move", "where", "use"}
C_TYPE_HEAD = re.compile(r"(struct\s+\w+|(?:unsigned\s+)?(?:char|short|int|long)|void)\b\s*(.*)")

# These names select known elements, not inferred capacities of incomplete arrays.
OPEN_ARRAY_ALIASES = {
    "gPlayer2MailboxItems": "gPlayerMailboxItems",
    "gPlayer2MailboxPrices": "gPlayerMailboxPrices",
    "P2_IDLE_COUNTER": "gPlayerIdleTicks",
    "P2_KEYDOWN": "gPlayerSleepPreviousInput",
    "gPlayer2PowerupTimer": "gPlayerPowerupTimers",
    "gPlayer2BurpTimer": "gPlayerBurpTimers",
    "gPlayer2RainCloudState": "gPlayerRainCloudStates",
    "gPlayer2RainCloudTimer": "gPlayerRainCloudTimers",
}


def snake(name: str) -> str:
    name = re.sub(r"([A-Z]+)([A-Z][a-z])", r"\1_\2", name)
    return re.sub(r"([a-z0-9])([A-Z])", r"\1_\2", name).lower()


def pascal(name: str) -> str:
    return "".join(part.capitalize() for part in name.split("_"))


def quote(value: str) -> str:
    return json.dumps(value, ensure_ascii=True)


def without_comments(text: str) -> str:
    return re.sub(r"/\*.*?\*/|//[^\n]*",
                  lambda match: "".join("\n" if ch == "\n" else " " for ch in match[0]), text, flags=re.S)


@dataclass(frozen=True)
class CType:
    base: str
    pointers: int = 0
    dimensions: tuple[int | None, ...] = ()

    def rust(self) -> str:
        name = (PRIMITIVES[self.base][0] if self.base in PRIMITIVES else
                "Function" if self.base == "function" else
                "crate::Opaque" if self.base == "void" else pascal(self.base.removeprefix("struct ")))
        for _ in range(self.pointers):
            name = f"Ptr<{name}>"
        for bound in reversed(self.dimensions):
            name = f"OpenArray<{name}>" if bound is None else f"[{name}; {bound}]"
        return name

    def spelling(self) -> str:
        return self.base + " *" * self.pointers + "".join(
            "[]" if n is None else f"[{n}]" for n in self.dimensions)


@dataclass
class Field:
    name: str
    ctype: CType
    offset: int = 0


@dataclass
class Record:
    name: str
    header: str
    fields: list[Field]
    size: int = 0
    alignment: int = 0


class Model:
    def __init__(self, root: Path) -> None:
        self.root = root
        self.headers = {p.name: p.read_text() for p in sorted((root / "include").glob("*.h"))}
        self.constants: dict[str, str] = {}
        for text in self.headers.values():
            self.constants.update(re.findall(r"^#define[ \t]+(\w+)[ \t]+([^\n]+)", without_comments(text), re.M))
        self.records: dict[str, Record] = {}
        self.declarations: dict[str, tuple[CType, str]] = {}
        for header, text in self.headers.items():
            text = without_comments(text)
            for match in re.finditer(r"\bstruct\s+(\w+)\s*\{(.*?)\};", text, re.S):
                fields = [field for declaration in match[2].split(";") if declaration.strip()
                          for field in self.parse_declaration(declaration)]
                self.records[match[1]] = Record(match[1], header, fields)
            for match in re.finditer(r"\bextern\s+([^;]+);", text, re.S):
                declaration = " ".join(match[1].split())
                head = C_TYPE_HEAD.fullmatch(declaration)
                if head and re.fullmatch(r"(?:\*\s*)*[A-Za-z_]\w*\s*\([^()]*\)", head[2]):
                    continue  # A bare K&R function declarator, not a parenthesized array bound.
                for field in self.parse_declaration(declaration):
                    previous = self.declarations.get(field.name)
                    if previous and previous[0] != field.ctype:
                        raise ValueError(f"conflicting declarations for {field.name}: {previous}, {field}")
                    self.declarations[field.name] = field.ctype, f"include/{header}"
        for record in self.records.values():
            self.layout(record)

    def integer(self, expression: str) -> int:
        def visit(node: ast.AST) -> int:
            if isinstance(node, ast.Constant) and isinstance(node.value, int):
                return node.value
            if isinstance(node, ast.Name):
                return self.integer(self.constants[node.id])
            if isinstance(node, ast.UnaryOp) and isinstance(node.op, ast.USub):
                return -visit(node.operand)
            if isinstance(node, ast.BinOp):
                left, right = visit(node.left), visit(node.right)
                operations = {ast.Add: lambda: left + right, ast.Sub: lambda: left - right,
                              ast.Mult: lambda: left * right, ast.LShift: lambda: left << right,
                              ast.BitOr: lambda: left | right}
                if type(node.op) in operations:
                    return operations[type(node.op)]()
            raise ValueError(f"unsupported array bound: {expression}")
        return visit(ast.parse(expression.strip(), mode="eval").body)

    def parse_declaration(self, declaration: str) -> list[Field]:
        declaration = " ".join(declaration.split())
        function = re.fullmatch(r"void\s*\(\s*\*\s*(\w+)\s*\)\s*\(\s*\)", declaration)
        if function:
            return [Field(function[1], CType("function", 1))]
        match = C_TYPE_HEAD.fullmatch(declaration)
        if not match:
            raise ValueError(f"unsupported C declaration (extend parser, never guess): {declaration}")
        base, declarators = match.groups()
        fields = []
        for declarator in declarators.split(","):
            match = re.fullmatch(r"\s*((?:\*\s*)*)([A-Za-z_]\w*)\s*((?:\[[^\]]*\]\s*)*)", declarator)
            if not match:
                raise ValueError(f"unsupported C declarator: {declaration}")
            dimensions = tuple(self.integer(x) if x.strip() else None
                               for x in re.findall(r"\[([^\]]*)\]", match[3]))
            if None in dimensions[1:]:
                raise ValueError(f"incomplete inner array bound: {declaration}")
            fields.append(Field(match[2], CType(base, match[1].count("*"), dimensions)))
        return fields

    def shape(self, ctype: CType) -> tuple[int, int]:
        if ctype.dimensions:
            size, alignment = self.shape(replace(ctype, dimensions=ctype.dimensions[1:]))
            return size * (ctype.dimensions[0] or 0), alignment
        if ctype.pointers:
            return 4, 2
        if ctype.base in PRIMITIVES:
            size = PRIMITIVES[ctype.base][1]
            return size, min(size, 2)
        record = self.records[ctype.base.removeprefix("struct ")]
        self.layout(record)
        return record.size, record.alignment

    def layout(self, record: Record) -> None:
        if record.alignment:
            return
        # MCC68K aligns even byte-only records to a word (measured by align_probe).
        offset, alignment = 0, 2
        for field in record.fields:
            size, align = self.shape(field.ctype)
            offset = (offset + align - 1) // align * align
            field.offset = offset
            offset += size
            alignment = max(alignment, align)
        record.size = (offset + alignment - 1) // alignment * alignment
        record.alignment = alignment

    def alias_at(self, ctype: CType, offset: int, path: str) -> tuple[CType, str] | None:
        size, _ = self.shape(ctype)
        if not 0 <= offset < size:
            return None
        if offset == 0:
            return ctype, path
        if ctype.dimensions:
            element = replace(ctype, dimensions=ctype.dimensions[1:])
            stride, _ = self.shape(element)
            return self.alias_at(element, offset % stride, f"{path}[{offset // stride}]")
        if not ctype.pointers and ctype.base.startswith("struct "):
            record = self.records[ctype.base[7:]]
            for field in record.fields:
                found = self.alias_at(field.ctype, offset - field.offset, f"{path}.{field.name}")
                if found:
                    return found
        return None


def parse_initializers(module: object, listing: str, name: str) -> list[int]:
    """Read only our synthetic unsigned-long constant array, not executable bytes."""
    result = bytearray()
    active = False
    for _, statement in module.parse(listing):
        if statement.label == name:
            active = True
        elif active and statement.label:
            break
        if not active:
            continue
        if statement.mnemonic.upper() == "DC.B":
            result.extend(int(x.replace("$", "0x"), 0) & 255 for x in statement.operands.split(","))
        elif statement.mnemonic.upper() == "DCB.B":
            count, value = statement.operands.split(",")
            result.extend([int(value, 0) & 255] * int(count, 0))
    if not result or len(result) % 4:
        raise ValueError(f"missing/malformed compiler constants {name}")
    return [int.from_bytes(result[i:i + 4], "big") for i in range(0, len(result), 4)]


def compiler_probes(model: Model, out: Path) -> dict:
    sys.dont_write_bytecode = True
    sys.path.insert(0, str(model.root / "tools"))
    mcc = importlib.import_module("mcc")
    facts = {}
    out.mkdir(parents=True, exist_ok=True)
    for record in model.records.values():
        expressions = [f"sizeof(struct {record.name})",
                       "(unsigned long)&((struct align_probe *)0)->value"]
        for field in record.fields:
            expressions.extend([f"(unsigned long)&((struct {record.name} *)0)->{field.name}",
                                f"sizeof(((struct {record.name} *)0)->{field.name})"])
        code = (f'#include "{record.header}"\n'
                f"struct align_probe {{ char pad; struct {record.name} value; }};\n"
                "unsigned long layout_values[] = {\n    " + ",\n    ".join(expressions) + "\n};\n")
        source = out / f"{record.name}.c"
        source.write_text(code)
        listing = mcc.compile_c(source, [model.root / "include"])
        (out / f"{record.name}.SRC").write_text(listing)
        values = parse_initializers(mcc, listing, "layout_values")
        expected = [record.size, record.alignment]
        for field in record.fields:
            expected.extend([field.offset, model.shape(field.ctype)[0]])
        if values != expected:
            raise ValueError(f"MCC68K layout mismatch {record.name}: model={expected}, compiler={values}")
        facts[record.name] = {"source": f"include/{record.header}", "size": values[0], "alignment": values[1],
                              "fields": [{"name": field.name, "offset": values[2 + i * 2],
                                          "size": values[3 + i * 2], "type": field.ctype.spelling()}
                                         for i, field in enumerate(record.fields)]}
    semantics = Path(__file__).with_name("semantics.c")
    listing = mcc.compile_c(semantics)
    (out / "semantics.SRC").write_text(listing)
    constants = parse_initializers(mcc, listing, "semantic_values")
    expected = [1, 2, 4, 4, 4, 4, 8, 0xffffffff, 0x80000000, 0xfffffffe, 0xffffffff,
                0xfffffffe, 65536, 0xffffffff, 1, 1, 1, 0]
    if constants != expected:
        raise ValueError(f"compiler semantic constants changed: {constants} != {expected}")
    bodies = {}
    for name in ("promote_char", "promote_unsigned_char", "promote_short", "promote_unsigned_short",
                 "shift_signed", "shift_unsigned", "compare_mixed", "call_order", "sum_order",
                 "compare_unsigned_char", "compare_unsigned_short", "signed_divide", "signed_remainder",
                 "unsigned_divide", "unsigned_remainder", "short_circuit", "comma_order",
                 "store_short", "switch_fallthrough", "persistent_counter"):
        start = listing.index(f".{name}:")
        end = listing.index("* COMPILATION SUMMARY", start)
        bodies[name] = listing[start:end]
    requirements = {"promote_char": ["EXT.W", "EXT.L", "7(SP)"],
                    "promote_unsigned_char": ["MOVEQ\t#0", "7(SP)"],
                    "promote_short": ["EXT.L", "ADDQ.L\t#1", "6(SP)"],
                    "promote_unsigned_short": ["MOVEQ\t#0", "ADDQ.L\t#1"],
                    "shift_signed": ["ASR.L"], "shift_unsigned": ["LSR.L"],
                    "compare_mixed": ["SHI"], "store_short": ["ADDQ.W"],
                    "compare_unsigned_char": ["SCS"], "compare_unsigned_short": ["SCS"],
                    "signed_divide": ["JSR\tldivt"], "signed_remainder": ["JSR\tlmodt"],
                    "unsigned_divide": ["JSR\tuldivt"], "unsigned_remainder": ["JSR\tulmodt"],
                    "switch_fallthrough": ["ADDQ.L\t#1", "ADDQ.L\t#2"],
                    "persistent_counter": ["static", "DC.B\t7"]}
    for function, fragments in requirements.items():
        for fragment in fragments:
            if fragment not in bodies[function]:
                raise ValueError(f"semantic probe {function} no longer emits {fragment!r}")
    for name in ("call_order", "sum_order"):
        if bodies[name].index("JSR\t.second") > bodies[name].index("JSR\t.first"):
            raise ValueError(f"observed right-first evaluation order changed: {name}")
    for name in ("short_circuit", "comma_order"):
        if bodies[name].index("JSR\t.first") > bodies[name].index("JSR\t.second"):
            raise ValueError(f"sequenced evaluation order changed: {name}")
    return {"compiler": "Microtec MCC68K 3.3K, default profile + /STRINTEXT", "layouts": facts,
            "semantic_constants": constants,
            "semantic_probe_sha256": hashlib.sha256(semantics.read_bytes()).hexdigest()}


def declaration_levels(ctype: CType) -> list[tuple[str, CType]]:
    """Object, each array row/element, then each measured pointer indirection."""
    levels = [("root", ctype)]
    while True:
        if ctype.dimensions:
            ctype = replace(ctype, dimensions=ctype.dimensions[1:])
            step = "array"
        elif ctype.pointers and not (ctype.pointers == 1 and ctype.base in ("void", "function")):
            ctype = replace(ctype, pointers=ctype.pointers - 1)
            step = "pointer"
        else:
            return levels
        levels.append((step, ctype))


def global_type_probes(model: Model, out: Path) -> dict:
    """Compare every declared global and every sized row/element with MCC68K."""
    mcc = importlib.import_module("mcc")
    groups: dict[str, dict[str, CType]] = {}
    for name, (ctype, source) in model.declarations.items():
        groups.setdefault(Path(source).name, {})[name] = ctype
    fixture = Path(__file__).with_name("declarators.h")
    regression = {}
    for match in re.finditer(r"\bextern\s+([^;]+);", without_comments(fixture.read_text()), re.S):
        regression.update((field.name, field.ctype) for field in model.parse_declaration(match[1]))
    groups[fixture.name] = regression
    facts = {}
    for revision in ("rev02", "rev00"):
        measured = {}
        for header, declarations in sorted(groups.items()):
            expressions, expected, slots = [], [], []
            for name, ctype in sorted(declarations.items()):
                levels = []
                expression = name
                for step, level in declaration_levels(ctype):
                    if step == "array":
                        expression = f"({expression})[0]"
                    elif step == "pointer":
                        expression = f"*({expression})"
                    size = None if level.dimensions and level.dimensions[0] is None else model.shape(level)[0]
                    levels.append({"step": step, "size": size})
                    if size is not None:
                        expressions.append(f"sizeof({expression})")
                        expected.append(size)
                        slots.append(levels[-1])
                measured[name] = {"source": f"include/{header}" if header != fixture.name else "tools/declarators.h",
                                  "c_type": ctype.spelling(), "levels": levels}
            source = out / f"global_{Path(header).stem}_{revision}.c"
            includes = {header}
            includes.update(model.records[ctype.base[7:]].header for ctype in declarations.values()
                            if ctype.base.startswith("struct "))
            source.write_text("\n".join(f'#include "{name}"' for name in sorted(includes)) +
                              "\nunsigned long global_type_values[] = {\n" +
                              ",\n".join(expressions) + "\n};\n")
            listing = mcc.compile_c(source, [model.root / "include", fixture.parent], version=revision)
            source.with_suffix(".SRC").write_text(listing)
            observed = parse_initializers(mcc, listing, "global_type_values")
            if observed != expected:
                raise ValueError(f"{revision} global object/row/pointee size mismatch in {header}: "
                                 f"model={expected}, compiler={observed}")
            for slot, size in zip(slots, observed, strict=True):
                slot["size"] = size
        facts[revision] = measured
    return facts


def check_maps(root: Path, revisions: dict, maps: dict[str, Path], out: Path) -> dict:
    """Check shipped maps and force otherwise unused aliases through G's real linker resolver."""
    build = importlib.import_module("build")
    names = importlib.import_module("names")
    mcc = importlib.import_module("mcc")
    toolchain = importlib.import_module("toolchain")
    facts = {}
    for revision, symbols in revisions.items():
        text = maps[revision].read_text()
        values = {name: int(value, 16) for value, name in
                  re.findall(r"^\s*(0x[0-9a-fA-F]+)\s+(\w+)\s*=", text, re.M)}
        present = 0
        for symbol in symbols:
            if symbol["name"] in values:
                present += 1
                if values[symbol["name"]] != int(symbol["address"], 16):
                    raise ValueError(f"{revision} existing link map mismatch: {symbol['name']}")
        # The game's link only emits referenced names: 60 aliases are unused.
        # Compile relocations for all names, resolve them with G/tools/build.py,
        # and link a synthetic address-only object using the pinned GNU toolchain.
        source = out / f"ram_{revision}.c"
        source.write_text("\n".join(f"extern char {s['name']};" for s in symbols) +
                          "\nchar *ram_addresses[] = {\n" +
                          ",\n".join("&" + s["name"] for s in symbols) + "\n};\n")
        compiled = mcc.compile_to_gas(source, "ramprobe", version=revision)
        obj = out / f"ram_{revision}.o"
        mcc.assemble(compiled.gas, source, obj, "ramprobe")
        (out / f"ram_{revision}.SRC").write_text(compiled.listing)
        references = {s["name"] for s in symbols}
        unit = SimpleNamespace(defined={"ram_addresses"}, undefined=references,
                               c_names=compiled.c_names, address=0, size=0, source=source)
        definitions = build.c_unit_definitions(
            [unit], SimpleNamespace(labels={}, absolute={}), names.load(revision, root))
        script = out / f"ram_{revision}.ld"
        script.write_text("SECTIONS { .data 0 : { *(.data) } }\n" +
                          "\n".join(f"{name} = {address};" for name, address in sorted(definitions.items())) + "\n")
        link_map = out / f"ram_{revision}.map"
        subprocess.run([str(toolchain.tool("m68k-elf", "ld")), "-T", str(script),
                        "-Map", str(link_map), "-o", str(out / f"ram_{revision}.elf"), str(obj)],
                       check=True, capture_output=True, text=True)
        linked = {name: int(value, 16) for value, name in
                  re.findall(r"^\s*(0x[0-9a-fA-F]+)\s+(\w+)\s*=", link_map.read_text(), re.M)}
        for symbol in symbols:
            if linked.get(symbol["name"]) != int(symbol["address"], 16):
                raise ValueError(f"{revision} forced-reference link map mismatch: {symbol['name']}")
        # Also verify relocation contents; not just textual symbol definitions.
        from elftools.elf.elffile import ELFFile
        with (out / f"ram_{revision}.elf").open("rb") as stream:
            data = ELFFile(stream).get_section_by_name(".data").data()
        addresses = [int.from_bytes(data[i:i + 4], "big") for i in range(0, len(data), 4)]
        if addresses != [int(s["address"], 16) for s in symbols]:
            raise ValueError(f"{revision} linked address relocations differ")
        facts[revision] = {"existing_map_symbols_checked": present,
                           "existing_map_unused_aliases": len(symbols) - present,
                           "forced_reference_link_symbols_checked": len(symbols),
                           "existing_map_sha256": hashlib.sha256(text.encode()).hexdigest()}
    return facts


def resolved_rom_symbols(model: Model) -> dict:
    resolver = importlib.import_module("names")
    revisions = {}
    for revision in ("rev02", "rev00"):
        symbols = {}
        for canonical in resolver.load(revision, model.root).values():
            if canonical.kind != "rom_data":
                continue
            for name in (canonical.name, *canonical.aliases):
                declaration = model.declarations.get(name)
                entry = {"address": canonical.address, "canonical": canonical.name,
                         "c_type": declaration[0].spelling() if declaration else None}
                if name in symbols and symbols[name] != entry:
                    raise ValueError(f"{revision} conflicting ROM resolver spelling: {name}")
                symbols[name] = entry
        revisions[revision] = dict(sorted(symbols.items()))
    return revisions


def rom_api_files(model: Model, revisions: dict, symbols: dict, banner: str) -> dict[str, list[str]]:
    """Separate source spellings/typed views from data-unit allocation metadata."""
    files = {}
    catalogue = [banner, "use crate::{Revision, RomSymbol};", ""]
    chunks = {}
    for revision, entries in symbols.items():
        chunks[revision] = []
        items = list(entries.items())
        for offset in range(0, len(items), 256):
            module = f"{revision}_{offset // 256:03d}"
            catalogue += [f"mod {module};"]
            chunks[revision].append(f"{module}::SYMBOLS")
            rows = [banner, "use crate::RomSymbol;", "", "#[rustfmt::skip]",
                    "pub(super) static SYMBOLS: &[RomSymbol] = &["]
            for name, entry in items[offset:offset + 256]:
                c_type = "None" if entry["c_type"] is None else f"Some({quote(entry['c_type'])})"
                rows += [f"    RomSymbol {{ name: {quote(name)}, address: 0x{entry['address']:06X}, "
                         f"canonical: {quote(entry['canonical'])}, c_type: {c_type} }},"]
            rows += ["];", ""]
            files[f"generated/rom_names/{module}.rs"] = rows
    for revision, names in chunks.items():
        catalogue += [f"static {revision.upper()}: &[&[RomSymbol]] = &[" + ", ".join(names) + "];"]
    catalogue += [
        "fn chunks(revision: Revision) -> &'static [&'static [RomSymbol]] {",
        "    match revision { Revision::Rev02 => REV02, Revision::Rev00 => REV00 }", "}",
        "/// Every canonical and alias spelling supplied by G's real ROM resolver.",
        "pub fn rom_symbols(revision: Revision) -> impl Iterator<Item = &'static RomSymbol> {",
        "    chunks(revision).iter().flat_map(|chunk| chunk.iter())", "}",
        "/// Resolve an exact source spelling without inferring an allocation or extent.",
        "pub fn rom_symbol(revision: Revision, name: &str) -> Option<&'static RomSymbol> {",
        "    let chunks = chunks(revision);",
        "    let index = chunks.partition_point(|chunk| chunk.last().is_some_and(|last| last.name < name));",
        "    let chunk = *chunks.get(index)?;",
        "    let index = chunk.binary_search_by(|symbol| symbol.name.cmp(name)).ok()?;",
        "    Some(&chunk[index])", "}",
        "#[cfg(test)]", "mod tests {", "    use super::*;", "    #[test]",
        "    fn every_revision_spelling_round_trips_across_chunk_boundaries() {",
        "        for (revision, count) in [(Revision::Rev02, " + str(len(symbols["rev02"])) +
        "), (Revision::Rev00, " + str(len(symbols["rev00"])) + ")] {",
        "            let mut previous = None;",
        "            for entry in rom_symbols(revision) {",
        "                if let Some(previous) = previous { assert!(previous < entry.name); }",
        "                assert_eq!(rom_symbol(revision, entry.name), Some(entry));",
        "                previous = Some(entry.name);", "            }",
        "            assert_eq!(rom_symbols(revision).count(), count);",
        "            assert!(rom_symbol(revision, \"\").is_none());",
        "            assert!(rom_symbol(revision, \"not_a_rom_symbol\").is_none());",
        "        }", "    }", "}", "",
    ]
    files["generated/rom_names.rs"] = catalogue
    ram_names = {s["name"] for s in revisions["rev02"]}
    declarations = {n: declaration for n, declaration in model.declarations.items() if n not in ram_names}
    views = [banner, "use super::structs::*;", "use crate::{Address, OpenArray, Ptr, Revision};",
             "/// Header-declared ROM views; a bound is a declaration, never an inferred allocation.",
             "pub mod rom {", "    use super::*;"]
    emitted = set()
    for name, (ctype, source) in sorted(declarations.items()):
        rust_name = snake(name)
        if rust_name in emitted:
            raise ValueError(f"Rust ROM declaration name collision: {name}")
        emitted.add(rust_name)
        views += [f"    /// `{name}` — {source}.",
                  f"    pub const fn {rust_name}(revision: Revision) -> Address<{ctype.rust()}> {{",
                  "        Address::new(match revision {"]
        for revision in ("rev02", "rev00"):
            if name not in symbols[revision]:
                raise ValueError(f"{revision} declared ROM global has no resolver spelling: {name}")
            views += [f"            Revision::{revision.capitalize()} => 0x{symbols[revision][name]['address']:06X},"]
        views += ["        })", "    }"]
    views += ["}", ""]
    files["generated/rom_views.rs"] = views
    return files


def global_layout_test_file(model: Model, revisions: dict, facts: dict, banner: str) -> list[str]:
    ram_names = {s["name"] for s in revisions["rev02"]}
    lines = [banner, "use crate::{ram, rom, Address, Ptr, Revision, Target};",
             "fn view_size<T: Target>(_: Address<T>) -> u32 { T::SIZE }",
             "fn pointee<T>(_: Address<Ptr<T>>) -> Address<T> { Address::new(0) }",
             "#[test]", "fn every_declared_global_object_row_and_pointee_matches_mcc68k() {",
             "    for revision in [Revision::Rev02, Revision::Rev00] {"]
    for name in sorted(model.declarations):
        namespace = "ram" if name in ram_names else "rom"
        measured = facts["global_layouts"]["rev02"][name]["levels"]
        if measured != facts["global_layouts"]["rev00"][name]["levels"]:
            raise ValueError(f"revision-dependent global type needs distinct views: {name}")
        lines += [f"        let view = {namespace}::{snake(name)}(revision);"]
        for level in measured:
            if level["step"] == "array":
                lines += [f"        assert_eq!(view.at(1).address().wrapping_sub(view.at(0).address()), {level['size']});",
                          "        let view = view.at(0);"]
            elif level["step"] == "pointer":
                lines += ["        let view = pointee(view);"]
            if level["size"] is not None:
                lines += [f"        assert_eq!(view_size(view), {level['size']});"]
    lines += ["    }", "}", ""]
    return lines


def emit(model: Model, revisions: dict, units: dict, facts: dict, rom_names: dict) -> tuple[dict[str, str], dict]:
    banner = "// @generated by tools/generate.py. No ROM contents; do not hand edit."
    files: dict[str, list[str]] = {}
    lines = [banner,
             "use crate::{Address, FieldLayout, Function, Ptr, StructLayout, Target};", ""]
    layout_entries = []
    checks = []
    for record in model.records.values():
        rust = pascal(record.name)
        checks += [f"        let view = Address::<{rust}>::new(0);"]
        lines += [f"/// `{record.name}` from `include/{record.header}`; emulated layout, not host storage.",
                  f"pub struct {rust};", f"impl Target for {rust} {{ const SIZE: u32 = {record.size}; }}",
                  f"impl Address<{rust}> {{"]
        for field in record.fields:
            name = snake(field.name)
            name = f"r#{name}" if name in KEYWORDS else name
            lines += [f"    pub const fn {name}(self) -> Address<{field.ctype.rust()}> {{",
                      f"        Address::new(self.address().wrapping_add({field.offset}))", "    }"]
            observed = next(x for x in facts["layouts"][record.name]["fields"] if x["name"] == field.name)
            checks += [f"        assert_eq!(view.{name}().address(), {observed['offset']});"]
            checks += [f"        assert_eq!(view_size(view.{name}()), {observed['size']});"]
        lines += ["}", ""]
        observed = facts["layouts"][record.name]
        checks += [f"        assert_eq!({rust}::SIZE, {observed['size']});"]
        fields = ", ".join(f"FieldLayout {{ name: {quote(f.name)}, offset: {f.offset}, size: {model.shape(f.ctype)[0]} }}"
                           for f in record.fields)
        layout_entries += [f"    StructLayout {{ name: {quote(record.name)}, source: {quote('include/' + record.header)}, size: {record.size}, alignment: {record.alignment}, fields: &[{fields}] }},"]
    lines += ["#[rustfmt::skip]", "pub static STRUCT_LAYOUTS: &[StructLayout] = &[", *layout_entries, "];", ""]
    lines += ["#[cfg(test)]", "mod compiler_layout_tests {", "    use super::*;",
              "    fn view_size<T: Target>(_: Address<T>) -> u32 { T::SIZE }",
              "    #[test]", "    fn every_accessor_matches_the_mcc68k_probe() {", *checks, "    }", "}", ""]
    files["generated/structs.rs"] = lines
    symbol_types = {}
    coverage = {"declared": 0, "structural_alias": 0, "opaque_byte": []}
    canonical = revisions["rev02"]
    for symbol in canonical:
        name, address = symbol["name"], int(symbol["address"], 16)
        if name in model.declarations:
            ctype, source = model.declarations[name]
            coverage["declared"] += 1
        elif name in OPEN_ARRAY_ALIASES:
            base_name = OPEN_ARRAY_ALIASES[name]
            array, header = model.declarations[base_name]
            if not array.dimensions or array.dimensions[0] is not None:
                raise ValueError(f"expected an incomplete outer bound: {base_name}")
            ctype = replace(array, dimensions=array.dimensions[1:])
            stride = model.shape(ctype)[0]
            for revision, symbols in revisions.items():
                addresses = {s["name"]: int(s["address"], 16) for s in symbols}
                if addresses[name] != addresses[base_name] + stride:
                    raise ValueError(f"{revision} element-one alias moved: {name}")
            source = f"{header}: alias {base_name}[1] (outer bound unspecified)"
            coverage["structural_alias"] += 1
        else:
            candidates = []
            for base in canonical:
                if base["name"] in model.declarations:
                    typ, header = model.declarations[base["name"]]
                    found = model.alias_at(typ, address - int(base["address"], 16), base["name"])
                    if found:
                        candidates.append((model.shape(typ)[0], found[0], f"{header}: alias {found[1]}"))
            if candidates:
                _, ctype, source = min(candidates, key=lambda x: x[0])
                coverage["structural_alias"] += 1
            else:
                ctype, source = CType("unsigned char"), "config/names.json: opaque byte (no header declaration)"
                coverage["opaque_byte"].append(name)
        symbol_types[name] = ctype, source
    lines = [banner, "use super::structs::*;",
             "use crate::{Address, Function, OpenArray, Ptr, RamSymbol, Revision};", ""]
    for revision, symbols in revisions.items():
        lines += ["#[rustfmt::skip]", f"pub static RAM_{revision.upper()}: &[RamSymbol] = &["]
        for symbol in symbols:
            ctype, source = symbol_types[symbol["name"]]
            lines += [f"    RamSymbol {{ name: {quote(symbol['name'])}, address: {symbol['address']}, c_type: {quote(ctype.spelling())}, source: {quote(source)} }},"]
        lines += ["];", ""]
        rom_lines = [banner, "use crate::RomDataUnit;", "", "#[rustfmt::skip]",
                     f"pub static ROM_DATA_{revision.upper()}: &[RomDataUnit] = &["]
        for unit in units[revision]:
            rom_lines += [f"    RomDataUnit {{ name: {quote(unit['name'])}, kind: {quote(unit['kind'])}, address: {unit['address']}, size: {unit['size']} }},"]
        rom_lines += ["];", ""]
        files[f"generated/rom_{revision}.rs"] = rom_lines
    lines += ["pub fn ram_symbols(revision: Revision) -> &'static [RamSymbol] {",
              "    match revision { Revision::Rev02 => RAM_REV02, Revision::Rev00 => RAM_REV00 }", "}",
              "/// Named typed RAM views. Array indices are intentionally unchecked emulated arithmetic.",
              "pub mod ram {", "    use super::*;"]
    by_revision = {r: {s["name"]: s["address"] for s in symbols} for r, symbols in revisions.items()}
    rust_names = set()
    for symbol in canonical:
        name = symbol["name"]
        rust_name = snake(name)
        if rust_name in rust_names:
            raise ValueError(f"Rust RAM name collision: {name}")
        rust_names.add(rust_name)
        ctype, source = symbol_types[name]
        lines += [f"    /// `{name}` — {source}.",
                  f"    pub const fn {rust_name}(revision: Revision) -> Address<{ctype.rust()}> {{",
                  "        Address::new(match revision {",
                  f"            Revision::Rev02 => {by_revision['rev02'][name]},",
                  f"            Revision::Rev00 => {by_revision['rev00'][name]},", "        })", "    }"]
    ram_checks = [f"        assert_eq!(ram::{snake(s['name'])}(Revision::{revision.capitalize()}).address(), {s['address']});"
                  for revision, symbols in revisions.items() for s in symbols]
    lines += ["}", "", "#[cfg(test)]", "mod compiler_address_tests {", "    use super::*;",
              "    #[test]", "    fn every_ram_view_matches_the_linked_address_probe() {", *ram_checks, "    }", "}", ""]
    files["generated/ram_views.rs"] = lines
    files.update(rom_api_files(model, revisions, rom_names, banner))
    files["generated/global_layout_tests.rs"] = global_layout_test_file(model, revisions, facts, banner)
    files["generated.rs"] = [
        banner, "mod structs;", "mod ram_views;", "mod rom_rev00;", "mod rom_rev02;", "",
        "mod rom_names;", "mod rom_views;", "#[cfg(test)]", "mod global_layout_tests;",
        "pub use structs::*;",
        "pub use ram_views::{ram, ram_symbols, RAM_REV00, RAM_REV02};",
        "pub use rom_rev00::ROM_DATA_REV00;", "pub use rom_rev02::ROM_DATA_REV02;", "",
        "pub use rom_names::{rom_symbol, rom_symbols};", "pub use rom_views::rom;",
        "use crate::{Revision, RomDataUnit};", "",
        "pub fn rom_data_units(revision: Revision) -> &'static [RomDataUnit] {",
        "    match revision { Revision::Rev02 => ROM_DATA_REV02, Revision::Rev00 => ROM_DATA_REV00 }", "}", "",
    ]
    formatted = {}
    for name, contents in files.items():
        result = subprocess.run(
            ["rustfmt", "--edition", "2024", "--emit", "stdout", "--config", "skip_children=true"],
            input="\n".join(contents), capture_output=True, text=True, check=True)
        formatted[name] = result.stdout
    return formatted, coverage


def survey(model: Model) -> dict:
    sources = {p.relative_to(model.root).as_posix(): p.read_text()
               for p in sorted((model.root / "src").rglob("*.c"))}
    record_arrays = [name for name, (typ, _) in model.declarations.items()
                     if typ.base.startswith("struct ") and not typ.pointers and typ.dimensions]
    index_pattern = re.compile(r"\b(" + "|".join(map(re.escape, record_arrays)) + r")\s*\[")
    index_sites = []
    steps = []
    bugs = []
    rom_assignments = []
    rom_names = {n for n in model.declarations if not n.startswith("g")}
    pointer_fields = {f.name for record in model.records.values() for f in record.fields if f.ctype.pointers}
    for source, text in sources.items():
        clean = without_comments(text)
        # Counts are lexical sites, not runtime executions or a complete C alias analysis.
        pointers = set(re.findall(r"\bstruct\s+\w+\s+\*\s*(\w+)", clean))
        for number, line in enumerate(clean.splitlines(), 1):
            for match in index_pattern.finditer(line):
                index_sites.append({"source": source, "line": number, "symbol": match[1]})
            for name in sorted(pointers):
                if re.search(rf"(?:\+\+|--)\s*\b{name}\b|\b{name}\s*(?:\+\+|--|\+=|-=)", line):
                    steps.append({"source": source, "line": number, "variable": name})
        for match in re.finditer(r"/\*.*?\*/", text, re.S):
            if "ORIGINAL BUG:" in match[0]:
                comment = " ".join(match[0].removeprefix("/*").removesuffix("*/").replace("\n", " ").split())
                bugs.append({"source": source, "line": text.count("\n", 0, match.start()) + 1, "comment": comment})
        for match in re.finditer(r"->(\w+)\s*=\s*([A-Za-z_]\w*)", clean):
            if match[1] in pointer_fields and match[2] in rom_names:
                rom_assignments.append({"source": source, "field": match[1], "rom_symbol": match[2]})
    reused = [{"record": r.name, "field": f.name} for r in model.records.values()
              for f in r.fields if "_or_" in f.name]
    assembly_bugs = [{"source": p.relative_to(model.root).as_posix(), "line": number}
                     for p in sorted((model.root / "src").rglob("*.s"))
                     for number, line in enumerate(p.read_text().splitlines(), 1) if "ORIGINAL BUG:" in line]
    return {"c_files": len(sources), "record_array_index_sites": index_sites,
            "record_pointer_step_sites": steps, "original_bug_comments": bugs,
            "assembly_original_bug_markers": assembly_bugs,
            "explicit_multi_meaning_fields": reused, "rom_symbol_to_pointer_field_assignments": rom_assignments,
            "variable_frame_prefix": {"bytes": model.records["render_frame"].size,
                                      "piece_stride": model.records["render_piece"].size,
                                      "bounded_access_views": ["render_frame_piece_view", "render_frame_four_piece_view"]}}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--decomp", type=Path, required=True)
    parser.add_argument("--probe-out", type=Path, required=True, help="private scratch, never inside G")
    parser.add_argument("--map-rev02", type=Path)
    parser.add_argument("--map-rev00", type=Path)
    parser.add_argument("--check", action="store_true", help="check committed outputs without modifying them")
    args = parser.parse_args()
    root = args.decomp.resolve()
    out = args.probe_out.resolve()
    if out.is_relative_to(root):
        raise ValueError("probe outputs must be outside read-only G")
    model = Model(root)
    revisions = {r: [s for s in json.loads((root / "config" / r / "names.json").read_text())["symbols"]
                     if s["kind"] == "ram"] for r in ("rev02", "rev00")}
    if {s["name"] for s in revisions["rev02"]} != {s["name"] for s in revisions["rev00"]}:
        raise ValueError("revision RAM symbol names differ; make missing symbols explicit before generation")
    units = {r: json.loads((root / "config" / r / "data_units.json").read_text())["units"] for r in revisions}
    maps = {r: getattr(args, f"map_{r}") or root / "build" / r / "toejam.map" for r in revisions}
    facts = compiler_probes(model, out)
    facts["global_layouts"] = global_type_probes(model, out)
    facts["ram_maps"] = check_maps(root, revisions, maps, out)
    rom_names = resolved_rom_symbols(model)
    rust, coverage = emit(model, revisions, units, facts, rom_names)
    facts["rom_symbol_counts"] = {revision: len(entries) for revision, entries in rom_names.items()}
    facts["typed_rom_views"] = len(model.declarations) - sum(name in model.declarations for name in
                                                        {symbol["name"] for symbol in revisions["rev02"]})
    facts["ram_type_coverage"] = coverage
    facts["rom_data_unit_counts"] = {r: len(values) for r, values in units.items()}
    facts["header_sha256"] = {h: hashlib.sha256(text.encode()).hexdigest() for h, text in model.headers.items()}
    facts["source_config_sha256"] = {f"config/{r}/{name}": hashlib.sha256((root / "config" / r / name).read_bytes()).hexdigest()
                                     for r in revisions for name in ("names.json", "data_units.json")}
    facts["survey"] = survey(model)
    crate = Path(__file__).resolve().parents[1]
    outputs = {crate / "src" / name: contents for name, contents in rust.items()}
    outputs[crate / "layout_receipt.json"] = json.dumps(facts, indent=2, sort_keys=True) + "\n"
    for path, contents in outputs.items():
        if args.check:
            if path.read_text() != contents:
                raise ValueError(f"generated output differs: {path}")
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(contents)
    fields = sum(len(r.fields) for r in model.records.values())
    print(f"MCC68K: {len(model.records)} structs, {fields} fields; maps: {len(revisions['rev02'])} RAM names/revision; "
          f"globals: {len(model.declarations)}; ROM units: {facts['rom_data_unit_counts']}; "
          f"ROM spellings: {facts['rom_symbol_counts']}; types: {coverage['declared']} declared, "
          f"{coverage['structural_alias']} structural aliases, {len(coverage['opaque_byte'])} opaque views")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
