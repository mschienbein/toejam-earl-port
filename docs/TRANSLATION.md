# Translating the matching decompilation

This is the contract for translating the [matching decompilation][decomp] (called
**G** below) into Rust using `tje-mem`. Preserve the executable behavior, including
its defects; do not substitute modern C semantics or a redesigned game model.
The compiler identification/profile comes from [G/docs/COMPILER.md][compiler],
the executable source for arithmetic helpers from [G/src/asm/runtime.s][runtime],
and the measured ABI facts from the committed
[`layout_receipt.json`](../crates/tje-mem/layout_receipt.json).

## Reproducing the evidence

Normal Cargo builds use committed Rust and do **not** require G, Python, MCC68K,
ROMs, or an asset extraction. Regeneration requires G's provisioned private
MCC68K/emu2 and pinned binutils installation, as described in
[G/docs/TOOLCHAIN.md][toolchain] and G's README. Run from this repository:

```sh
G=/absolute/path/to/toejam-earl-decomp
"$G/.venv/bin/python" -B crates/tje-mem/tools/generate.py \
  --decomp "$G" --probe-out "$PWD/scratch/memory-probes"
# Recompile probes, check links, and compare without changing committed output:
"$G/.venv/bin/python" -B crates/tje-mem/tools/generate.py \
  --decomp "$G" --probe-out "$PWD/scratch/memory-probes" --check
cargo test -p tje-mem
cargo build -p tje-mem --target wasm32-unknown-unknown
```

`--map-rev02` and `--map-rev00` optionally select existing G link maps; by default
these are `G/build/<revision>/toejam.map`. G is read-only: generated C, compiler
listings, objects, ELF files, and forced-reference maps go into `--probe-out`,
which must be outside G. Do not commit that scratch directory. The generator
reads headers/configuration and link maps, never a ROM or extracted asset.

Generated source is split into a small `src/generated.rs` entry point and
`src/generated/{structs,ram_views,rom_rev00,rom_rev02}.rs`. Record accessors, RAM
views/address tests, and each revision's ROM metadata have separate modules, all
below the public audit's unchanged 1-MB per-blob limit. Their public Rust API is
unchanged; `--check` deterministically compares every module and the receipt.

For each of **27 structs and all 186 fields**, the generator compiles independent
C constants recording `sizeof(struct)`, alignment via an enclosing `char`-prefix
record, `offsetof` (the compiler's constant null-base member-address expression),
and `sizeof(field)`. It compares these with a separately calculated layout before
emitting any Rust. Generated ROM-free tests compare every accessor's offset and
actual emulated field type size with those compiler facts. For example,
`object_state` is 128 bytes, `process` is at `0x16`, `current_frame` at `0x22`, and
`next` at `0x5a`; pointers/longs require **two**, not four, byte alignment.
Even byte-only structs have two-byte alignment.

Both revision catalogues contain **438 RAM names with identical addresses**.
Each existing G game map names 378 of them; the remaining 60 are unused aliases,
not missing allocations. The generator checks all 378 directly, then compiles
address relocations for **all 438**, obtains definitions from G's actual
`tools/build.py:c_unit_definitions` and `tools/names.py:load`, links with G's pinned
68000 GNU tools, and compares both the resulting map and the linked big-endian
pointer words. This checks the otherwise unreferenced aliases too. The receipt
separately reports the existing-map and forced-reference counts rather than
pretending the existing maps contain every alias.

There are **4,163 ROM data units per revision**. Only their names, kinds, addresses,
and sizes are generated, never their contents. Use the selected revision's table:
ROM addresses can move even though RAM addresses do not.

### What a typed view means

```rust
use tje_mem::{ram, AddressSpace, Ptr, Revision};
let mut memory = AddressSpace::new(vec![0; 16], Revision::Rev02);
let player = ram::g_player_info(Revision::Rev02).at(0);
player.x().write(&mut memory, -10);
player.sequence().write(&mut memory, Ptr::new(8));
assert_eq!(player.x().read(&memory), -10);
assert_eq!(player.sequence().read(&memory).address(), 8);
```

`Address<T>` is a copyable 32-bit emulated address. `Ptr<T>` is a 32-bit pointer
**value** stored in memory, not a host reference. Generated struct names are type
markers: use `ObjectState::SIZE` with `Target` in scope, never
`std::mem::size_of::<ObjectState>()`. Accessors read/write big-endian bytes through
`Bus`; an emulator can implement `Bus` over its live RAM without a copy.

Of the RAM views, 368 use explicit header declarations and 53 are aliases resolved
through those layouts, including eight element-one aliases of incomplete arrays.
Seventeen undeclared legacy/assembly symbols retain an explicitly labelled `u8`
storage view. G provides no C declaration for them: no fabricated signedness,
pointee, or array capacity is asserted. Their names and addresses are complete;
use `cast::<u16>()`, `cast::<u32>()`, or a known record view where the **particular
assembly access** establishes that width. The receipt lists every such symbol.
A notable example is `gVdpDmaCommand`: G's `src/asm/dma.s` writes a long, then reads
its high and low words. `gMusicFmChannel4OwnerByte1` is deliberately an interior
byte of a pointer. Legacy names are not reliable type declarations:
`gEarlAccelX`, for example, addresses a byte field of the modern object view.

Complete array bounds retain G's declared dimensions, including bounds that G
itself labels inferred. `OpenArray<T>` represents an unspecified outer bound and
has no `Target::SIZE`, invented length, or whole-array pointer stride. `.at(i32)`
on either kind is intentionally unchecked emulated address arithmetic. This is
not Rust slice indexing and never creates an out-of-bounds host pointer.

## Integer and expression rules

### Sizes, signedness, and promotion

The synthetic [`tools/semantics.c`](../crates/tje-mem/tools/semantics.c) is compiled
with MCC68K **3.3K, defaults plus `/STRINTEXT`**, the profile identified in G's
compiler document. Its constant records and emitted operations establish:

| MCC68K type | Bytes | Rust storage | Expression promotion |
|---|---:|---|---|
| `char` | 1 | `i8` | sign-extend to `i32` |
| `unsigned char` | 1 | `u8` | zero-extend to **`u32`** |
| `short` | 2 | `i16` | sign-extend to `i32` |
| `unsigned short` | 2 | `u16` | zero-extend to **`u32`** |
| `int`, `long` | 4 | `i32` | already full width |
| `unsigned int`, `unsigned long` | 4 | `u32` | already full width |
| data/function pointer | 4 | `Ptr<T>` | address arithmetic, not host pointers |

The probe also records `float=4`, `double=8`; the current header structs contain
neither. Do not introduce host `isize`/`usize` for C arithmetic or game pointers.
Use `usize` only when actually indexing a host-owned byte buffer.

**MCC68K has pre-ANSI unsigned-preserving promotions.** Do not apply the modern
rule that a 32-bit signed `int` can represent every unsigned byte/word. The constant
probe gives `(unsigned short)65535 < -1` as **true**. Both dynamic unsigned-byte
and unsigned-word comparison probes zero-extend and emit an unsigned condition
(`SCS`); signed/unsigned-long comparisons likewise use an unsigned condition.
Promote an unsigned small operand to `u32` before arithmetic/comparison, converting
the other operand to that type when required. Explicit signed casts in G can
change this. `arithmetic::promote_*` encodes the four small-type promotions.

Plain `char` is signed: the dynamic probe reads the low argument byte then emits
`EXT.W` and `EXT.L`; its constant `(long)(char)-1` is `0xffffffff`. Preserve
explicit unsigned-byte reinterpretations, particularly the attachment offsets in
worked example 2. Rust's `as i8`/`as u8` gives the required low-bit reinterpretation.

### K&R calls and return values

G's compiler document records that prototypes, `signed`, and `volatile` are
rejected, while K&R definitions work. Do not translate a call as if a modern
prototype performed implicit narrowing first. Default argument promotions apply;
**each argument occupies a four-byte stack slot**, pointers included. With the
return address at `SP`, a first `char` parameter is loaded from `7(SP)`, a `short`
from `6(SP)`, and a full word/pointer from `4(SP)` (observed in the probes).
Sign/zero extend at the call site's original declared expression type; a K&R
callee then reads the portion selected by its own parameter declaration.

Integer results use D0. Do not assume that an original narrow result always
cleared or sign-extended its unused upper bits. G explicitly marks several
consumer-local `extern int` declarations as original mismatched-result bugs.
At those call sites, preserve the observed register/result view rather than
silently substituting a corrected shared signature. Missing arguments,
uninitialized locals, and residual register bits require the corresponding
original stack/register state from the hybrid host; inventing a zero is not a
faithful translation.

### Wraparound, narrowing, and shifts

Translate the operation width, not just the destination's width. Signed byte/short
expressions generally calculate in `i32`, then narrow on assignment. For example,
`(selection - 1) % 6` must not subtract as `i16`. Unsigned small types promote as
above. Explicit storage-width updates may compile directly to byte/word
operations: `store_short` emits `ADDQ.W`. Narrowing via `as` retains the low bits;
use `wrapping_add/sub/mul/neg` at the observed width whenever the machine can
wrap. Ordinary Rust overflow panics in debug builds are not original behavior.
The runtime multiply retains the low 32 bits for both signed and unsigned uses;
`wrapping_mul` implements that value. `abs(i32::MIN)` remains `i32::MIN` because
G's stock ABS simply executes a wrapping `NEG.L`.

Signed right shift is arithmetic (`ASR.L` in the probe); unsigned right shift is
logical (`LSR.L`). Signed-left shifts retain the low result bits, not Rust/C
undefined behavior. A **68000 register-count shift uses count modulo 64**, and a
32-bit shift by 32..63 yields zero (left/logical right) or all sign bits
(arithmetic right). Rust `wrapping_shl` masks modulo 32 and is therefore wrong
for those counts. Use `lsl_u32`, `lsr_u32`, and `asr_i32` when preserving that
instruction behavior. A word/byte instruction has its own width: do not substitute
the longword helper if G's generated code genuinely uses a narrower operation.
The helpers model values, not condition-code flags; code that consumes CCR must
preserve those flags at the CPU/translation boundary too.

### Division and modulo

The probes demonstrate native `DIVS`/`DIVU` fast paths with fallback calls to
`ldivt`, `lmodt`, `uldivt`, and `ulmodt`. G's runtime source establishes their full
32-bit magnitude/restoring-division behavior:

- Signed division truncates toward **zero**, not toward negative infinity.
- Signed remainder has the **dividend's** sign: `-7 / 3 = -2`, `-7 % 3 = -1`;
  `7 % -3 = 1`. Do not use Rust's Euclidean division/remainder.
- `i32::MIN / -1` returns the wrapped `i32::MIN`; its remainder is zero.
- Unsigned division/remainder operate on all 32 bits, including divisors above
  `0xffff`; do not truncate the inputs to the native 68000 word-divisor format.
- **Division by zero is a real 68000 vector-5 exception.** In G's helpers, zero
  takes the word-divisor path and executes `DIVU` with zero; there is no zero
  result or saturation path. `tje_mem::arithmetic` returns
  `Err(ArithmeticTrap::DivideByZero)` (vector 5), which the hybrid/platform caller
  must propagate as that exception, not swallow. A fixed nonzero literal divisor
  may use the corresponding Rust operator directly, as in example 1.

Unit tests cover signs, minimum values, both divisor paths, and all four zero
cases. These are synthetic semantic tests, not a claim that those exceptional
paths have already been differentially executed in the reference emulator.

### Comparisons and evaluation order

Choose signed or unsigned comparison **after** MCC68K promotion/conversion.
`i16`/`i8` signed sentinel values (`-1`, `-2`) must remain negative; testing their
stored bytes as `u8` changes game branches. Conversely, comparing a promoted
unsigned short with a negative signed int normally compares `u32` bit patterns.
Treat C truth as `!= 0`, including negative values, and produce integer 0/1 when
C uses the comparison value rather than just a branch.

Never assume Rust's left-to-right evaluation reproduces MCC68K for side-effecting
operands. The probes evaluate `consume(first(), second())` **second first**, and
also evaluate `first() + second()` **second first**. This is observed compiler
behavior for those expressions, not a guarantee for every operator/function.
Inspect G's matching compiler output when order matters, then extract explicit
statements in that order. C's comma operator and short-circuit `&&`/`||` retain
sequencing; the comma/`&&` probes call `first` before `second`. Preserve short
circuiting, conditional branches, and each separate assignment's read/write order.
Do not precompute an entire record's new values when earlier writes may alias
later reads. A `Bus` implementation mapping devices must also preserve access
width and ordered side effects.

## Pointers, control flow, storage, and names

- Keep addresses as 32-bit values; `Address::offset` scales by the emulated type's
  `Target::SIZE` with wrapping arithmetic, and `byte_offset` does not scale.
  Casts change the view, not the bytes or address. Pointer subtraction is address
  subtraction followed by the original element-size division, with its original
  signed width. Do not use host pointer provenance or Rust reference subtraction.
- Pointer zero is **ROM address zero**, not an invented trap. Only a C truth test
  treats zero as false. Do not make pointer dereference an `Option` that hides an
  original read. ROM pointers stored in RAM remain big-endian numeric words.
  Function pointers are addresses dispatched through the hybrid/native function
  map; never transmute ROM addresses into Rust function pointers.
- `AddressSpace` masks physical accesses to 24 bits, maps immutable ROM at zero,
  and maps the 64-KiB RAM through the Genesis `E00000..FFFFFF` mirror window;
  canonical RAM names use `FF0000`. Ordinary cartridge-ROM writes do not mutate
  it. Byte-based storage allows unaligned views; the CPU host, not a storage
  accessor, owns instruction address-error exceptions. Unmapped reads/writes
  outside these storage regions fail explicitly, never return fake zero. VDP,
  Z80, audio, controller, and other devices require their actual platform bus.
- Preserve `switch` **fallthrough** explicitly. Rust `match` does not fall through;
  share the executed tail or use a small explicit control-flow state where needed.
  The probe's case 1 adds one and then executes case 2's add-two, yielding three.
  Do not duplicate predicates or side-effecting selector evaluation. Preserve
  observed out-of-range switch behavior and signedness of dispatch bounds.
- A C `static` has persistent storage, not a fresh local each call. The synthetic
  static probe emits a data initializer and updates the same location. Mutable
  game storage belongs at its original address in the snapshot-able memory model,
  or in explicit per-machine state if it has no original emulated address; never
  a process-global Rust `static mut` shared by independent machines. No `static`
  declarations occur in the currently surveyed 109 C files; this is still the
  rule for translating that compiler construct.
- Use Rust **snake_case** for functions and variables, PascalCase for record
  markers, and the existing snake_case field names. Preserve G's canonical name
  in a source comment/doc comment with the decomp path and REV 02 identity/address.
  A Rust rename is not a newly recovered historical name. Per [G's compiler
  document][compiler], address-spelled G names mean REV 02 identities in both
  revisions; use revision metadata instead of hardcoding a REV 02 ROM pointer.
- Keep every `ORIGINAL BUG:` behavior, including cross-array reads/writes, field
  reinterpretations, omitted arguments, and uninitialized state. A bounds check,
  clamp, fallback pointer, extra allocation retry, initialized zero, or corrected
  call signature changes the program. G's `[INFERENCE]` reachability and matching
  qualifications remain qualifications, not permission to repair behavior.

## Survey: does the flat big-endian model hold?

**Yes, for the game's memory representation; it is not by itself a complete
console bus or scheduler.** The following counts are reproducible in the
receipt's `survey` section. They cover all **109 C source files**, with comments
removed for code-site counts, plus original-bug markers in assembly. The lexical
site metrics are explicitly conservative inventories, not full C alias analysis
or claims about runtime frequency.

| Required pattern | Count / definition | Concrete evidence and consequence |
|---|---|---|
| Arithmetic across records | **64** increment/decrement/compound-step lines for variable names declared as `struct *` in that file; **919** indexed accesses to declared global struct arrays | `SetRAMIndexes`, `src/sprites/entity_indexes.c`, advances `entity` through forty 128-byte objects. `OpenEarthlingPresent`, `src/earthlings/spawning_and_banks.c`, advances through 18-byte earthlings. Native Rust object vectors are not interchangeable with either layout. |
| Original out-of-bounds access | **3 explicitly labelled sites in 2 C routines**: two in `OpenEarthlingPresent`, one in `GetMapTerrainAtWorldPosition` | The first exhausted bank scan reads record 20; the second exhausted scan can write that record. For bank zero, this is bank one's first record; for bank one, it is following RAM. The terrain query does not reject negative map coordinates before reading through `map[map_y * 19 + map_x]`; reachability is explicitly `[INFERENCE]` in G. |
| Reused fields | **3 fields explicitly named `_or_`**, not an exhaustive count of semantic reuse | `earthling_state.saved_x_or_status` is spawn X, terrain class, or a latch. `object_state.attachment_x_or_latch` / `_y_or_latch` are signed parent offsets, unsigned camera offsets, or contact latches. G also documents kind-dependent `state`, `timer`, and `health_visibility`; splitting these into disjoint Rust variants would lose aliases. |
| ROM pointers stored in RAM | **76** direct assignments of declared ROM asset symbols to pointer-typed member names | Cutscene constructors store frame-table addresses in object `frames` and sometimes `current_frame`. `src/cutscenes/impact_and_planet_entities.c` explicitly labels the frame-table-address-in-`current_frame` case an original bug. `sequence`, callbacks, linked-list pointers, and nested ROM pointer tables are further address-bearing uses not all captured by this lexical metric. |
| Variable-length frames | **1,287 `metasprite` units per revision**, with **11 distinct sizes, 14..124 bytes** in the data-unit metadata; **2 bounded C access views** | `render_frame` is a **4-byte prefix** plus `piece_count` **10-byte** pieces. The 12-piece/4-piece views in `sprite_frame_views.h` are access views, never universal ROM allocations or strides. `Address<RenderFrame>::piece` and `byte_len` use the actual count. |

There are **43 `ORIGINAL BUG:` C comment blocks** overall and **1** additional Z80
assembly marker; most concern other issues, not out-of-bounds storage. Do not
confuse that total with the three specifically identified out-of-bounds sites.
The terrain query's negative-index defect recurs as an unchecked condition in
other code, but the table counts only explicitly labelled sites. A source label
is not proof that every bad state is reachable during normal play.

These patterns confirm retaining one flat big-endian store with overlapping
views. They also refute replacing pointers with host references, representing
frames as fixed twelve-element records, copying field aliases into independent
variables, or "safely" truncating an original out-of-bounds scan. The model must
still delegate hardware I/O, traps, interrupt timing, and uninitialized
register/stack state to the proper host; a flat store alone does not prove whole
game equivalence.

## Three worked, tested translations

The complete implementations and all synthetic-memory tests live together in
[`crates/tje-mem/src/examples.rs`](../crates/tje-mem/src/examples.rs). The snippets
below explain those implementations, not alternate untested versions. Tests use
only constructed RAM/ROM bytes, never cartridge assets.

### 1. Leaf arithmetic: `GetLevelMusicIndex` → `get_level_music_index`

Source: `G/src/core/startup_and_main_loop.c`, REV 02 `0x001262`.
G's original expression is `(selection - 1) % 6`, with `selection` a `short`:

```rust
pub fn get_level_music_index(selection: i16) -> i16 {
    ((i32::from(selection) - 1) % 6) as i16
}
```

`leaf_arithmetic_reads_and_writes_synthetic_memory` reads the argument from
synthetic big-endian RAM, calls this function, and stores/reads its result.
It covers `i16::MIN → -3`, `-1 → -2`, `0 → -1`, ordinary music indices, and
`i16::MAX → 0`. Narrowing the subtraction before the remainder would fail the
minimum-input case. Rust's signed `%` is appropriate here because six is nonzero
and the promoted numerator has no overflow case.

### 2. Struct-heavy: `UpdateAttachedEntityPosition` → `update_attached_entity_position`

Source: `G/src/sprites/entity_sprite_rendering.c`, REV 02 `0x00C170`.
The function takes an `Address<ObjectState>` and reads the signed `owner` byte.
For nonnegative owners it selects `g_player_info(revision).at(owner)` and performs
three separate promoted additions/narrowing stores:

```rust
let parent = ram::g_player_info(revision).at(i32::from(owner));
let x = (i32::from(entity.attachment_x_or_latch().read(bus))
    + i32::from(parent.x().read(bus))) as i16;
entity.x().write(bus, x);
```

The Y operation is identical in shape. Height explicitly reinterprets the stored
`u8` attachment height as `i8`. Tile priority is copied from the parent without
changing the child's other tile bits. Owner `-1` instead uses camera-one pending
coordinates; owner `-2` uses camera two. Those camera branches reinterpret X/Y
attachment bytes as **unsigned**, leave height/tile unchanged, and narrow the
sum only on assignment. Other negative owners perform no writes, as in G.

`parent_attachment_preserves_signed_views_and_narrowing` covers both revisions,
signed offsets, 16-bit wraparound, and both parent priority states.
`camera_attachment_reinterprets_offsets_as_unsigned` proves that stored `-1`
means **255**, not -1, for a camera offset; it covers both camera owners,
unchanged height/tile, and the unchanged-memory path for another negative owner.

### 3. VDP writes: `SelectSpriteChainStart` → `select_sprite_chain_start`

Source: `G/src/sprites/select_first_link.c`, REV 02 `0x00CFBA`.
The real function selects link 1 for any nonzero `short` argument and 11 for zero:

```rust
let link = if primary_only != 0 { 1 } else { 11 };
platform.write_vdp_command(0x78020003);
platform.write_vdp_data(u16::from(link));
ram::g_hardware_sprite_root_link(revision).write(bus, link);
```

The `tje_mem::Platform` trait requires `write_vdp_control(u16)` and
`write_vdp_data(u16)`. Its default `write_vdp_command(u32)` emits the high command
word then the low command word, matching the 68000's two ordered bus writes to
the aliased VDP control port. It does not reinterpret a port as ordinary RAM.
The function's RAM mirror update occurs **after** the hardware writes.

`vdp_example_preserves_port_width_order_and_ram_alias` tests both revisions,
zero, positive, negative, and minimum-short arguments. It captures the exact
control/control/data trace (`0x7802`, `0x0003`, selected link), verifies the RAM
mirror aliases hardware sprite zero's `link`, and checks that every other RAM
byte remains unchanged. The test trace is a test-only observer; production
platform implementations must perform the real device writes.

[decomp]: https://github.com/mschienbein/toejam-earl-decomp
[compiler]: https://github.com/mschienbein/toejam-earl-decomp/blob/main/docs/COMPILER.md
[toolchain]: https://github.com/mschienbein/toejam-earl-decomp/blob/main/docs/TOOLCHAIN.md
[runtime]: https://github.com/mschienbein/toejam-earl-decomp/blob/main/src/asm/runtime.s
