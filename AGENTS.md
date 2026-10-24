# Working on this repository

- Nothing from the game goes into this repository: no ROM bytes, extracted assets or
  disassembly. The port reads game data from the user's ROM at runtime.
- Game data is never transcribed into source, including arrays, string literals or
  test fixtures. Translated code reads game data at runtime through `tje-mem`'s ROM
  views; only ROM-free synthetic data belongs in source-based fixtures.
- The [decompilation](https://github.com/mschienbein/toejam-earl-decomp) (checkpoint c038-final)
  is the source of truth for behaviour. Every translated function names its decomp source file
  and function and keeps the decomp's name in snake_case. It preserves the original behaviour
  exactly, including integer widths and wraparound, evaluation order and original bugs; keep the
  decomp's `ORIGINAL BUG:` notes.
- Faithfulness is shown by tests against the original ROM, not by reading the code.
- Keep `cargo fmt`, `cargo clippy` and the tests clean for native targets and wasm32.

## Checks

Run the same workspace-wide checks as CI (the toolchain and wasm target are pinned).
The audit uses the pinned source grammars in `tools/audit_requirements.txt`; `uv run`
provides an isolated Python 3.12 environment without changing the system Python:

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo build --locked --workspace --target wasm32-unknown-unknown --exclude tje-native
uv run --python 3.12 --with-requirements tools/audit_requirements.txt python -m unittest discover -s tools -p 'test_*.py' -v
uv run --python 3.12 --with-requirements tools/audit_requirements.txt python tools/audit_public.py --no-rom
```

Without `--refs`, the audit examines tracked worktree files, every staged blob and
HEAD. Adding `--history` checks every local Git ref as a hygiene report; it requires
a non-shallow repository and can legitimately expose unsafe unpublished prototypes.
It rejects private/generated paths, binary/assets/disassembly and long encoded data.
Hex, numeric/byte atoms, escaped Rust/Python strings, Base64 and escaped JSON are
decoded: non-text payloads of 64 bytes or more are rejected by size/shape without an
entropy exemption. Shorter ROM matches and additional opaque-format heuristics
remain checked. ROM-free CI skips only comparisons with private cartridge bytes.
Encoded-payload heuristics operate only on literal/comment contents in source files,
never on identifiers, paths or punctuation. Numeric/byte runs and exact ROM-window
checks remain active; ordinary dense Rust/JSON macro code is not an encoded blob.
Tests generate both synthetic revision images and cover worktree, staged-only and
deleted historical payloads; optional real-ROM probes stay entirely in memory.
Source grammars distinguish comments from literals before byte-run grouping,
including nested/multiline block comments and language-specific comment forms.
Adjacent literals, bounded literal-only `concat!` trees and homogeneous ordered
literal containers are decoded as one payload. Array/list/tuple/sequence elements
and sibling literal runs retain source order; identifiers and record/map fields
break aggregation rather than joining unrelated data. Rust, Python, YAML and C#
containers use their actual grammars, including nested containers. No macros run.
Lists of plausible text identifiers are fact tables, including short names such as
`d0`/`a0` that happen to resemble hexadecimal bytes. Their exact and decoded private
ROM comparisons remain active, without speculative Base64/opaque size findings
caused solely by joining names. Entire hex streams, substantial hex fragments,
byte-literal containers and nonidentifier Base64 fragments do not get this treatment.
Cargo-provided package metadata is a public fact,
not an evaluated environment value; literal bytes around it are still checked.
Vendored modifications use the same full-source tokens before changed-line
projection, so comments or unchanged macro delimiters cannot hide new data.
Parsing traversals allow eight nodes per input byte (a 4,096-node minimum and an
8,000,000-node absolute cap), so dense controller-only recordings are not rejected
by an arbitrary fixed node count. The 1 MB public-blob, nesting and decoded-byte
limits still apply. Exhausting a resource limit fails the audit; no traversal is
silently truncated and no remaining content checks are skipped on success.

Compile-time `include_bytes!`, `include_str!` and `include!` may name only a direct
literal path to a regular, tracked public input in the same audited Git state.
Their identifier tokens are reserved: forwarding them as macro arguments,
aliasing/importing them, or any unresolved use is rejected, wherever it occurs.
Private, missing, untracked and symlink inputs and indirect `concat!`/`env!` paths
are forbidden. Private ROM tests read files at runtime, never embed them.

Vendored jgenesis files are governed by `vendor/jgenesis/UPSTREAM.json`, generated
from the pinned public upstream Git tree:

```sh
uv run --python 3.12 --with-requirements tools/audit_requirements.txt python tools/generate_vendor_manifest.py
```

The audit fetches the pinned upstream revision into ignored `.cache/public-audit/`
and verifies its Git objects and every manifest path/blob id. An unavailable,
uncached upstream, an unlisted vendor file or unverifiable metadata fails closed.
Exact upstream blobs are exempt from content checks; modified files are audited
only on added/replaced lines, with changed include expressions checked in their
complete source context. The local `NOTICE.md` has no upstream counterpart and
is audited in full. Path/type/size restrictions still apply to all vendored files.
Generation uses only the current HEAD/index/worktree vendor closure, never unrelated
local refs. Historical audits also verify past manifests to recover provenance for
removed files; trees predating the first manifest use the verified published map.
Do not hand-add content exemptions.
No exceptions are inherited from the decompilation.

Before publishing, put your own unmodified images at `roms/rev02.bin` and
`roms/rev00.bin` (ignored by Git), then run the stronger private checks:

```sh
cargo test --locked -p tje-rom --test roms -- --nocapture
uv run --python 3.12 --with-requirements tools/audit_requirements.txt python tools/audit_public.py
```

The ROM tests print a skip message if a private image is absent; other read errors
and incorrect images fail. The private audit requires both pinned 1 MiB ROMs and
checks their whole-image SHA-1 before comparing exact raw and decoded ROM windows.

## Publication gate versus local hygiene

First run the plain audit above to check the index/worktree, commit the intended
changes, and then audit exactly the branch or commit being published:

```sh
uv run --python 3.12 --with-requirements tools/audit_requirements.txt python tools/audit_public.py --history --refs HEAD --no-rom
uv run --python 3.12 --with-requirements tools/audit_requirements.txt python tools/audit_public.py --history --refs HEAD
```

`--refs REF...` implies history checking and inspects only the selected committed
refs and their complete ancestry. It does not include unrelated refs, current HEAD,
the index or working files unless those objects belong to the selected ancestry.
It is equivalent to checking a clean single-branch clone of the exact branch being
pushed. CI runs both the plain ROM-free check and this committed publication check.

Keep the all-local-refs report as a separate hygiene operation:

```sh
uv run --python 3.12 --with-requirements tools/audit_requirements.txt python tools/audit_public.py --history --no-rom
```

Failures confined to unpublished prototypes or stashes must not be hidden by content
exceptions. Keep those refs out of publication; audit every ref actually being pushed.

