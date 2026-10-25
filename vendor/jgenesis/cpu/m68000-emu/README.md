# m68000-emu

An instruction-based [68000](https://en.wikipedia.org/wiki/Motorola_68000) CPU emulation core.

The 68000 is used in the Genesis as the main CPU, and in the Sega CD as a secondary CPU meant for controlling the CD-ROM drive and interacting with the new graphics functions and audio chips.

## TJE source-reviewed synchronous trap entry

`hybrid_enter_trap` is a narrow readable-control adapter to the existing
`handle_trap` implementation. It executes no opcode and advances no device clock.
The source owner must bind the actual function/revision/fault opcode/site and
produce its pre-trap registers, flags and exact next-word prefetch first. The
adapter retains the decoded fault instruction only for secondary address-error
attribution and updates the source mul/div and address-error latches; it does not
mint a retired-opcode count or proof receipt.
The explicit fault site is distinct from the saved PC. The live pipeline must
be at `fault_site + 2` after that one prefetch; a reviewed DIVU saves the next PC,
whereas first-opcode ORI/ANDI-to-SR privilege failures save `fault_site`. The
source owns these disjoint contracts; true CCR forms are not privileged.

The real supervisor switch, trace clearing, banked stack pushes, vector fetch,
handler prefetch and successful-entry marker are unchanged. An actual entry
fault runs the original address-error/double-fault handling, retains its state
and returns failure, never a fabricated successful six-byte frame.

The error's `last_vector` is the retained CPU latch and may describe a PREVIOUS
entry. It is not evidence that the failed attempt entered a new vector; actual
successful-entry markers remain separate.

Authored DIVU-zero and user ORI/ANDI-to-SR tests compare all 25 CPU words and
exact physical reads/purposes/writes against an actual original instruction,
including odd inactive SSP and handler controls. Privilege tests retain the
original SR (including T), save the opcode PC, and do not consume the immediate
or invent an instruction retirement. These are not physical-console
cycle-accuracy claims.