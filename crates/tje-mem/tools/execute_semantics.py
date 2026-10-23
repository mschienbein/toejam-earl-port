#!/usr/bin/env python3
"""Execute synthetic MCC68K code on Unicorn M68000 and compare real Rust APIs.

Requires G's Python dependencies/toolchain plus unicorn==2.1.4 (an optional
--unicorn-path may point to a user-local installation). All executable artifacts
and the temporary Rust comparison crate stay in --out, outside read-only G.
Only a compact ROM-free execution receipt is committed.
"""
from __future__ import annotations

import argparse
import hashlib
import importlib
import json
import os
from pathlib import Path
import subprocess
import sys


DATA = 0x40000
STACK = 0xF0000
RETURN = 0x80000
MASK = 0xFFFFFFFF


def command(arguments: list[str], **kwargs) -> subprocess.CompletedProcess:
    result = subprocess.run(arguments, capture_output=True, text=True, **kwargs)
    if result.returncode:
        raise RuntimeError(f"command failed: {arguments}\n{result.stdout}{result.stderr}")
    return result


class Machine:
    def __init__(self, unicorn, registers, elf: Path) -> None:
        from elftools.elf.elffile import ELFFile
        self.unicorn = unicorn
        self.r = registers
        self.cpu = unicorn.Uc(unicorn.UC_ARCH_M68K, unicorn.UC_MODE_BIG_ENDIAN)
        self.cpu.ctl_set_cpu_model(registers.UC_CPU_M68K_M68000)
        self.cpu.mem_map(0, 0x100000)
        with elf.open("rb") as stream:
            image = ELFFile(stream)
            self.symbols = {s.name: s["st_value"] for s in image.get_section_by_name(".symtab").iter_symbols()}
            for section in image.iter_sections():
                if section["sh_flags"] & 2 and section["sh_type"] != "SHT_NOBITS":
                    self.cpu.mem_write(section["sh_addr"], section.data())
        self.interrupts: list[int] = []
        self.events: list[list] = []
        self.helper_entries: list[str] = []
        self.aggregate_arguments: list[bytes] = []
        self.cpu.hook_add(unicorn.UC_HOOK_INTR, self.interrupt)
        self.cpu.hook_add(unicorn.UC_HOOK_CODE, self.code)
        self.cpu.hook_add(unicorn.UC_HOOK_MEM_READ | unicorn.UC_HOOK_MEM_WRITE, self.memory)

    def interrupt(self, cpu, vector: int, unused) -> None:
        self.interrupts.append(vector)
        cpu.emu_stop()

    def code(self, cpu, address: int, size: int, unused) -> None:
        for name in ("uldivt", "ulmodt"):
            if address == self.symbols[name]:
                self.helper_entries.append(name)
        if address == self.symbols["record_argument"]:
            sp = cpu.reg_read(self.r.UC_M68K_REG_A7)
            self.aggregate_arguments.append(bytes(cpu.mem_read(sp + 4, 8)))

    def memory(self, cpu, access: int, address: int, size: int, value: int, unused) -> None:
        if DATA <= address < DATA + 64:
            kind = "write" if access == self.unicorn.UC_MEM_WRITE else "read"
            self.events.append([kind, address, size])

    def run(self, name: str, arguments: tuple[int, ...] = (), *, registers: dict | None = None,
            writes: tuple[tuple[int, bytes], ...] = ()) -> str:
        self.interrupts.clear()
        self.events.clear()
        self.helper_entries.clear()
        self.aggregate_arguments.clear()
        for register in range(self.r.UC_M68K_REG_A0, self.r.UC_M68K_REG_D7 + 1):
            self.cpu.reg_write(register, 0x13579BDF)
        self.cpu.reg_write(self.r.UC_M68K_REG_SR, 0x2000)
        self.cpu.reg_write(self.r.UC_M68K_REG_A7, STACK)
        self.cpu.mem_write(STACK, b"".join((n & MASK).to_bytes(4, "big") for n in (RETURN, *arguments)))
        for address, data in writes:
            self.cpu.mem_write(address, data)
        for register, value in (registers or {}).items():
            self.cpu.reg_write(register, value & MASK)
        self.cpu.emu_start(self.symbols[name], RETURN, count=10000)
        if self.interrupts:
            if self.interrupts != [5]:
                raise AssertionError((name, arguments, self.interrupts))
            return "trap5"
        if self.cpu.reg_read(self.r.UC_M68K_REG_PC) != RETURN:
            raise AssertionError(f"{name} did not return within the instruction limit")
        return str(self.cpu.reg_read(self.r.UC_M68K_REG_D0) & MASK)


def compile_probes(root: Path, out: Path, source: Path) -> Path:
    mcc = importlib.import_module("mcc")
    toolchain = importlib.import_module("toolchain")
    compiled = mcc.compile_to_gas(source, "execution")
    (out / "execution.SRC").write_text(compiled.listing)
    obj = out / "execution.o"
    mcc.assemble(compiled.gas, source, obj, "execution")
    runtime = out / "runtime.o"
    command([str(toolchain.tool("m68k-elf", "as")), "-m68000", "-o", str(runtime),
             str(root / "src/asm/runtime.s")])
    script = out / "execution.ld"
    script.write_text("SECTIONS { .text 0x10000 : { *(.text*) } .data 0x20000 : { *(.data*) } "
                      ".bss : { *(.bss*) } }\n")
    elf = out / "execution.elf"
    command([str(toolchain.tool("m68k-elf", "ld")), "-T", str(script), "-Map", str(out / "execution.map"),
             "-o", str(elf), str(obj), str(runtime)])
    return elf


def rust_results(crate: Path, out: Path, target: Path, pairs: list[tuple[int, int]]) -> list[list[str]]:
    project = out / "rust-api"
    (project / "src").mkdir(parents=True, exist_ok=True)
    (project / "Cargo.toml").write_text(
        '[package]\nname = "memory-semantic-execution"\nversion = "0.0.0"\nedition = "2024"\n'
        '[workspace]\n[dependencies]\ntje-mem = { path = ' + json.dumps(crate.as_posix()) + ' }\n')
    (project / "src/main.rs").write_text('''use std::io::{self, BufRead};
use tje_mem::arithmetic::{ArithmeticTrap, div_u32, rem_u32, expr_div_u32, expr_rem_u32};
struct Token(Result<u32, ArithmeticTrap>);
impl std::fmt::Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 { Ok(number) => write!(f, "{number}"), Err(trap) => write!(f, "trap{}", trap.vector()) }
    }
}
fn main() {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut line = String::new();
    while input.read_line(&mut line).unwrap() != 0 {
        let mut values = line.split_whitespace().map(|s| s.parse::<u32>().unwrap());
        let (a, b) = (values.next().unwrap(), values.next().unwrap());
        println!("{} {} {} {}", Token(expr_div_u32(a, b)), Token(expr_rem_u32(a, b)),
                 Token(div_u32(a, b)), Token(rem_u32(a, b)));
        line.clear();
    }
}
''')
    result = command(["cargo", "run", "--quiet", "--manifest-path", str(project / "Cargo.toml")],
                     input="".join(f"{a} {b}\n" for a, b in pairs),
                     env={**os.environ, "CARGO_TARGET_DIR": str(target), "CARGO_BUILD_JOBS": "4"})
    return [line.split() for line in result.stdout.splitlines()]


def divisions(machine: Machine, crate: Path, out: Path, target: Path) -> dict:
    dividends = [0, 1, 2, 32767, 32768, 65534, 65535, 65536, 0x7FFFFFFF, 0x80000000,
                 0xFFFEFFFE, 0xFFFEFFFF, 0xFFFF0000, 0xFFFFFFFE, 0xFFFFFFFF]
    divisors = [0, 1, 2, 0x7FFF, 0x8000, 0xFFFF, 0x10000, 0x7FFFFFFF, 0x80000000,
                0xFFFF7FFF, 0xFFFF8000, 0xFFFF8001, 0xFFFFFFFE, 0xFFFFFFFF]
    pairs = [(a, b) for a in dividends for b in divisors]
    observed = []
    native, runtime, differences = 0, 0, 0
    representatives = []
    for a, b in pairs:
        quotient = machine.run("expression_quotient", (a, b))
        used_runtime = bool(machine.helper_entries)
        remainder = machine.run("expression_remainder", (a, b))
        assert bool(machine.helper_entries) == used_runtime
        # Direct calls exercise the stock runtime separately from compiler lowering.
        registers = {machine.r.UC_M68K_REG_D0: a, machine.r.UC_M68K_REG_D1: b}
        direct_q = machine.run("uldivt", registers=registers)
        direct_r = machine.run("ulmodt", registers=registers)
        if b == 0:
            assert [quotient, remainder, direct_q, direct_r] == ["trap5"] * 4
        else:
            assert [direct_q, direct_r] == [str(a // b), str(a % b)]
            runtime += int(used_runtime)
            native += int(not used_runtime)
            differences += int([quotient, remainder] != [direct_q, direct_r])
        row = [quotient, remainder, direct_q, direct_r]
        observed.append(row)
        if (a, b) in ((65535, MASK), (0x7FFFFFFF, 0xFFFF8000), (0x80000000, 0xFFFF8000),
                      (0xFFFF0000, MASK), (MASK, MASK)):
            representatives.append({"dividend": a, "divisor": b, "expression": row[:2],
                                    "direct_runtime": row[2:], "expression_called_runtime": used_runtime})
    actual = rust_results(crate, out, target, pairs)
    assert len(actual) == len(observed)
    for pair, expected, result in zip(pairs, observed, actual, strict=True):
        assert result == expected, (pair, "executed C/runtime", expected, "Rust", result)
    assert differences > 0 and native > 0 and runtime > 0
    (out / "division_matrix.json").write_text(json.dumps(
        [{"dividend": a, "divisor": b, "results": result} for (a, b), result in zip(pairs, observed, strict=True)],
        indent=2) + "\n")
    return {"operand_pairs": len(pairs), "rust_vs_executed_results": len(pairs) * 4,
            "native_nonzero_pairs": native, "runtime_nonzero_pairs": runtime,
            "pairs_differing_from_direct_runtime": differences,
            "zero_vectors_per_entry": len(dividends), "representative_cases": representatives}


def aggregate_and_pointer_checks(machine: Machine) -> dict:
    layout = bytes(machine.cpu.mem_read(machine.symbols["aggregate_layout"], 16))
    layout = [int.from_bytes(layout[n:n + 4], "big") for n in range(0, 16, 4)]
    assert layout == [8, 0, 2, 6]
    original = bytes(range(16))
    machine.run("copy_record", (DATA + 8, DATA), writes=((DATA, original),))
    assert bytes(machine.cpu.mem_read(DATA + 8, 8)) == original[:8]
    assert machine.events == [["read", DATA, 4], ["write", DATA + 8, 4],
                              ["read", DATA + 4, 4], ["write", DATA + 12, 4]]
    copy_events = machine.events.copy()
    machine.run("copy_record", (DATA + 2, DATA), writes=((DATA, original),))
    overlapped = bytes(machine.cpu.mem_read(DATA, 16))
    expected = bytearray(original)
    expected[2:6] = expected[:4]
    expected[6:10] = expected[4:8]
    assert overlapped == expected and overlapped[2:10] != original[:8]
    first = bytes([1, 170, 1, 2, 3, 4, 255, 204])
    second = bytes([2, 187, 5, 6, 7, 8, 254, 221])
    assert machine.run("call_record_argument", (DATA,), writes=((DATA, first),)) == str(0x01020304)
    assert machine.aggregate_arguments == [first]
    scratch = int(machine.run("record_return", (DATA,), writes=((DATA, first),)))
    assert bytes(machine.cpu.mem_read(scratch, 8)) == first
    next_scratch = int(machine.run("record_return", (DATA,), writes=((DATA, second),)))
    assert scratch == next_scratch and bytes(machine.cpu.mem_read(scratch, 8)) == second
    comparisons = [(0x01000000, 0, 0), (0xFF000001, 1, 0), (0x80000000, 0x7FFFFFFF, 0),
                   (0x7FFFFFFF, 0x80000000, 1)]
    for left, right, result in comparisons:
        assert machine.run("compare_pointers", (left, right)) == str(result)
    differences = [(DATA + 2, DATA, 1), (DATA, DATA + 2, MASK), (DATA, DATA + 1, MASK),
                   (0, MASK, 0), (0x80000000, 0, 0xC0000000)]
    for left, right, result in differences:
        assert machine.run("subtract_short_pointers", (left, right)) == str(result)
    pointer_events = {}
    for name, offset in (("post_char_pointer", 3), ("post_short_pointer", 2)):
        old = DATA + offset
        assert machine.run(name, (DATA,), writes=((DATA, old.to_bytes(4, "big")),)) == "4"
        write = machine.events.index(["write", DATA, 4])
        read = machine.events.index(["read", old, 4 - offset])
        assert write < read
        pointer_events[name] = machine.events.copy()
    updates = 0
    for name, width in (("compound_char", 1), ("compound_short", 2)):
        mask = (1 << (width * 8)) - 1
        for initial in (0, 127, mask):
            for delta in (1, 65537, MASK):
                machine.run(name, (DATA, delta), writes=((DATA, initial.to_bytes(width, "big")),))
                assert int.from_bytes(machine.cpu.mem_read(DATA, width), "big") == (initial + delta) & mask
                assert [event for event in machine.events if event[0] == "write"] == [["write", DATA, width]]
                updates += 1
    for name, width, initials in (("post_char", 1, (127, 255)), ("post_short", 2, (32767, 65535))):
        mask = (1 << (width * 8)) - 1
        for initial in initials:
            result = machine.run(name, (DATA,), writes=((DATA, initial.to_bytes(width, "big")),))
            signed = initial if initial < (1 << (width * 8 - 1)) else initial - (mask + 1)
            assert result == str(signed & MASK)
            assert int.from_bytes(machine.cpu.mem_read(DATA, width), "big") == (initial + 1) & mask
            assert [event for event in machine.events if event[0] == "write"] == [["write", DATA, width]]
            updates += 1
    return {"aggregate_layout": layout, "copy_events": copy_events,
            "overlap_preserves_forward_bus_behavior": True, "inline_argument_bytes": len(first),
            "argument_padding_preserved": True, "return_scratch_is_persistent": True,
            "pointer_comparison_cases": len(comparisons), "pointer_difference_cases": len(differences),
            "aliasing_stored_pointer_events": pointer_events, "narrow_update_cases": updates}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--decomp", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--cargo-target", type=Path)
    parser.add_argument("--unicorn-path", type=Path)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    root, out = args.decomp.resolve(), args.out.resolve()
    if out.is_relative_to(root):
        raise ValueError("execution outputs must be outside read-only G")
    out.mkdir(parents=True, exist_ok=True)
    sys.dont_write_bytecode = True
    sys.path.insert(0, str(root / "tools"))
    if args.unicorn_path:
        sys.path.insert(0, str(args.unicorn_path.resolve()))
    import unicorn
    from unicorn import m68k_const
    if unicorn.__version__ != "2.1.4":
        raise ValueError("execution receipt requires pinned Unicorn 2.1.4")
    crate = Path(__file__).resolve().parents[1]
    source = Path(__file__).with_name("execution.c")
    machine = Machine(unicorn, m68k_const, compile_probes(root, out, source))
    result = {"compiler": "MCC68K 3.3K, defaults + /STRINTEXT", "engine": "Unicorn 2.1.4, M68000",
              "source_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
              "runtime_source_sha256": hashlib.sha256((root / "src/asm/runtime.s").read_bytes()).hexdigest(),
              "rust_arithmetic_sha256": hashlib.sha256((crate / "src/arithmetic.rs").read_bytes()).hexdigest(),
              "unsigned_division": divisions(machine, crate, out, args.cargo_target or out / "target"),
              "aggregate_and_pointer": aggregate_and_pointer_checks(machine)}
    contents = json.dumps(result, sort_keys=True, indent=2) + "\n"
    receipt = crate / "execution_receipt.json"
    if args.check:
        if receipt.read_text() != contents:
            raise ValueError("compiled execution receipt differs")
    else:
        receipt.write_text(contents)
    print(f"Compiled execution PASS: {result['unsigned_division']['rust_vs_executed_results']} Rust/C/runtime "
          f"division comparisons, {result['unsigned_division']['pairs_differing_from_direct_runtime']} "
          "compiler-expression defect cases; aggregate/pointer/narrow-update probes passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
