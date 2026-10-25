# Extras: designs and opt-in implementations

These documents describe extras developed separately from the faithful port. Faithful mode stays
the default and every implemented extra is opt-in; design-only capabilities fail admission rather
than selecting a partial gameplay implementation.

| Design | Summary |
| --- | --- |
| [Jam Out in normal play](jam-out.md) | Jam Out moves and music in the overworld as an optional toggle: an interruptible dance state built from the existing move set and audio routines. |
| [Four-player co-op](four-player.md) | Four seats with typed per-seat state, quadrant-first views, four pads, and palette-swap characters computed at runtime from the user's ROM. The original's player tables are two-seat, so this is a generalisation, not a count change. |
| [Overlord (3 vs 1)](overlord.md) | Three heroes against a body-less director who spends an integer budget on validated commands: spawning Earthlings and laying harmless, telegraphed traps. |
| [Larger, denser levels](larger-denser-levels.md) | Independent opt-in size and density settings; a proposed expanded grid with a hard cap, within the original's sprite, VRAM and palette limits. |
| [Maker mode](maker.md), [format and native loader](maker-format.md) | Implemented first phase: bounded asset-free JSON courses, strict validation, local checksummed share codes, three authored samples and a native 19×26 one/two-seat terrain-and-population loader. Graphical editing, generalized capacities and hosted sharing remain later phases. |

All of them build on an **enhanced mode** in the shipped engine. Selected original functions are
swapped for modified Rust versions taken from the proven readable layer, while everything else runs
as the compiled original. Nothing derived from the ROM is stored in this repository: palette swaps and
any extracted graphics are computed at runtime from the user's own ROM, and new art is newly authored.

Citations use `G/...` for paths in the
[matching decompilation](https://github.com/mschienbein/toejam-earl-decomp) and `P/...` for paths in
this repository.
