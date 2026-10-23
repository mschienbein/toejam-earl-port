# Working on this repository

- Nothing from the game goes into this repository: no ROM bytes, extracted assets or
  disassembly. The port reads game data from the user's ROM at runtime.
- The [decompilation](https://github.com/mschienbein/toejam-earl-decomp) (checkpoint c038-final)
  is the source of truth for behaviour. Every translated function names its decomp source file
  and function and keeps the decomp's name in snake_case. It preserves the original behaviour
  exactly, including integer widths and wraparound, evaluation order and original bugs; keep the
  decomp's `ORIGINAL BUG:` notes.
- Faithfulness is shown by tests against the original ROM, not by reading the code.
- Keep `cargo fmt`, `cargo clippy` and the tests clean for native targets and wasm32.
