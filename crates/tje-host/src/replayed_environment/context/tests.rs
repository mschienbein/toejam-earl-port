//! Private provider mechanisms, not admission/proof-completion tests. Each
//! provider deliberately has no body/case capability; the input graph comes
//! from an actual JSR-to-RTS run of an entirely authored synthetic cartridge.
use super::*;
use crate::replayed_environment::{frame::FrameMemory, tests::causal_inputs};

fn memory(original: &Invocation) -> FrameMemory {
    let mut ram = Box::new([0_u8; 65536]);
    // Authored main: MOVE #$2700,SR; JSR abs.l root. Its return is $10A.
    let sp = crate::catalogue::STACK_TOP - 4;
    ram[(sp & 65535) as usize..(sp & 65535) as usize + 4].copy_from_slice(&0x10a_u32.to_be_bytes());
    FrameMemory::new(original, ram, 0).unwrap()
}
fn write(memory: &mut FrameMemory, address: u32, value: u8) {
    memory.write_private(Access {
        address,
        word: false,
        direction: Direction::Write,
        value: u16::from(value),
    });
}
fn provider<'a>(
    original: &'a Invocation,
    inputs: inputs::CalleeInputs,
    memory: &FrameMemory,
) -> CallOutput<'a> {
    let mut values = std::array::from_fn(|_| RegisterValue::Rust(0));
    values[StateField::Sr.index() as usize] = RegisterValue::Rust(0x2700);
    values[StateField::Ssp.index() as usize] = RegisterValue::Rust(crate::catalogue::STACK_TOP - 4);
    let mut snapshots = BTreeMap::new();
    for address in inputs.output_inputs.memory_addresses() {
        snapshots
            .entry(address)
            .or_insert_with(|| memory.call_input_byte(address));
    }
    CallOutput {
        original,
        entry_values: values,
        output_inputs: inputs.output_inputs,
        memory_inputs: snapshots,
        required: RefCell::new(OutputCoverage::default()),
        case: None,
        reason: Some("unit input-provider mechanism: no body proof or admission".into()),
        body: None,
        contract: None,
        obligation_id: 0,
    }
}
fn private_word(revision: Revision) -> (crate::Host, std::sync::Arc<Invocation>) {
    // Only the word read's low half becomes D1. Its N/Z/C/V are also the final
    // CCR; X remains the actual entry X, independent of the private RAM word.
    causal_inputs::capture(revision, vec![0x598f, 0x3217, 0x588f, 0x4e75], vec![])
}

#[test]
fn lazy_word_output_requires_only_the_requested_causal_byte() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let (host, original) = private_word(revision);
        let inputs = causal_inputs::analyze(&host, &original).unwrap();
        let local = crate::catalogue::STACK_TOP - 8;
        let mut memory = memory(&original);
        write(&mut memory, local + 1, 0);
        let call = provider(&original, inputs, &memory);
        assert_eq!(call.resolve(StateField::Data(1), 0xff).unwrap(), 0);
        assert!(call.resolve(StateField::Data(1), 0xffff).is_err());
        assert_eq!(call.required.borrow().state.get(StateField::Data(1)), 0xff);
        assert!(
            call.case.is_none(),
            "an input-provider result cannot mint a proof case"
        );
    }
}

#[test]
fn wrong_call_time_byte_cannot_be_repaired_by_a_later_write() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let (host, original) = private_word(revision);
        let inputs = causal_inputs::analyze(&host, &original).unwrap();
        let local = crate::catalogue::STACK_TOP - 7;
        let mut memory = memory(&original);
        write(&mut memory, local, 1);
        let call = provider(&original, inputs, &memory);
        write(&mut memory, local, 0);
        assert!(call.resolve(StateField::Data(1), 0xff).is_err());
        assert_eq!(call.required.borrow().state.get(StateField::Data(1)), 0);
    }
}

#[test]
fn correct_call_time_provider_survives_a_later_legitimate_write() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let (host, original) = private_word(revision);
        let inputs = causal_inputs::analyze(&host, &original).unwrap();
        let local = crate::catalogue::STACK_TOP - 7;
        let mut memory = memory(&original);
        write(&mut memory, local, 0);
        let call = provider(&original, inputs, &memory);
        write(&mut memory, local, 1);
        assert_eq!(call.resolve(StateField::Data(1), 0xff).unwrap(), 0);
    }
}

#[test]
fn unknown_below_sp_and_opaque_future_bytes_never_gain_call_time_credit() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let (host, original) = private_word(revision);
        let local = crate::catalogue::STACK_TOP - 7;
        let mut memory = memory(&original);
        // Original RAM is also zero. Coincidence outside the current live SP
        // is not an initialization or ownership proof.
        assert!(memory.call_input_byte(local).is_err());
        let inputs = causal_inputs::analyze(&host, &original).unwrap();
        let call = provider(&original, inputs, &memory);
        write(&mut memory, local, 0);
        assert!(call.resolve(StateField::Data(1), 0xff).is_err());
        memory.mark_private_residue(local);
        assert!(memory.call_input_byte(local).is_err());
        let inputs = causal_inputs::analyze(&host, &original).unwrap();
        let opaque = provider(&original, inputs, &memory);
        write(&mut memory, local, 0);
        assert!(opaque.resolve(StateField::Data(1), 0xff).is_err());
    }
}

#[test]
fn lazy_ccr_subsets_keep_exact_memory_bit_demands() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let (host, original) = private_word(revision);
        let inputs = causal_inputs::analyze(&host, &original).unwrap();
        let local = crate::catalogue::STACK_TOP - 8;
        let mut memory = memory(&original);
        // N consumes only bit7 of the high byte; neither the remaining high
        // bits nor the unavailable low byte are an N input. X consumes no RAM.
        write(&mut memory, local, 0x7f);
        let call = provider(&original, inputs, &memory);
        assert_eq!(call.resolve(StateField::Sr, 0x10).unwrap(), 0);
        assert_eq!(call.resolve(StateField::Sr, 8).unwrap(), 0);
        assert!(call.resolve(StateField::Sr, 0x1f).is_err());
        assert_eq!(call.required.borrow().state.get(StateField::Sr), 0x18);
    }
}

#[test]
fn generic_non_own_writer_ingress_never_mints_explicit_entry_input_permission() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let (_, original) = private_word(revision);
        let mut memory = memory(&original);
        let address = original.entry.boundary.cpu.sp() - 4;
        memory.original_stack(address);
        memory.rust_stack(address).unwrap();
        let access = Access {
            address,
            word: true,
            direction: Direction::Write,
            value: 0x1234,
        };
        // Mechanism-level ingress model for supported Z80/host mutation records,
        // not an actual bus-master execution or a DMA-to-main-RAM writer proof.
        // Genesis VDP DMA does not write main RAM.
        memory.environment_write(access);
        assert_eq!(
            memory.explicit_unwritten_mask(address, 2).unwrap_err(),
            tje_game::stack_read::UnsupportedReason::InterveningNonOwnWrite
        );
        let Failure::StackRead(reason) = memory
            .uninitialized_read(
                Access {
                    direction: Direction::Read(m68000_emu::ReadPurpose::Semantic),
                    ..access
                },
                3,
            )
            .unwrap_err()
        else {
            panic!("non-own writer must remain typed unsupported");
        };
        assert_eq!(
            reason.reason,
            tje_game::stack_read::UnsupportedReason::InterveningNonOwnWrite
        );
    }
}
