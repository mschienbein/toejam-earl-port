//! Authored source identities and bus events, not cartridge observations.
use super::*;
use m68000_emu::ReadPurpose;

fn read(address: u32, value: u16, word: bool) -> Access {
    Access {
        address,
        value,
        word,
        direction: Direction::Read(ReadPurpose::Semantic),
    }
}
fn write(address: u32, value: u16, word: bool) -> Access {
    Access {
        address,
        value,
        word,
        direction: Direction::Write,
    }
}
fn register_sources(origins: &mut Origins, field: StateField) -> [Origin; 16] {
    std::array::from_fn(|bit| origins.registers(StateMask::single(field, 1 << bit)))
}

#[test]
fn physical_occurrences_keep_old_then_committed_then_external_origins() {
    let mut origins = Origins::new();
    let first = origins
        .new_read(read(0xe00010, 0x1234, true), 0xffff)
        .unwrap();
    origins
        .bind_read(first, read(0xe00010, 0x1234, true))
        .unwrap();
    let old = origins.read_source(first, 0xffff).unwrap();
    let sources = register_sources(&mut origins, StateField::Data(2));
    origins.store(write(0xff0010, 0x5678, true), &sources);
    let second = origins
        .new_read(read(0xff0010, 0x5678, true), 0xffff)
        .unwrap();
    origins
        .bind_read(second, read(0xff0010, 0x5678, true))
        .unwrap();
    let committed = origins.read_source(second, 0xff00).unwrap();
    origins.store(write(0xe00010, 0x9a, false), &[Origin::EMPTY; 16]);
    let third = origins.new_read(read(0xff0010, 0x9a, false), 0xff).unwrap();
    origins
        .bind_read(third, read(0xff0010, 0x9a, false))
        .unwrap();
    let external = origins.read_source(third, 0xff).unwrap();
    let before = origins.demand(old).unwrap();
    assert_eq!(
        before.memory[&0xff0010],
        MemoryInput {
            value: 0x12,
            bits: 0xff
        }
    );
    assert_eq!(
        before.memory[&0xff0011],
        MemoryInput {
            value: 0x34,
            bits: 0xff
        }
    );
    assert!(before.registers.is_empty());
    let after = origins.demand(committed).unwrap();
    assert_eq!(
        after.registers,
        StateMask::single(StateField::Data(2), 0xff00)
    );
    assert!(after.memory.is_empty());
    let after_external = origins.demand(external).unwrap();
    assert!(after_external.registers.is_empty() && after_external.memory.is_empty());
    // A later write changes neither earlier occurrence nor its initial label.
    assert_eq!(origins.demand(old).unwrap().memory[&0xff0010].value, 0x12);
}

#[test]
fn changed_raw_ram_without_a_preceding_commit_fails_even_if_unrequested() {
    let mut origins = Origins::new();
    let first = origins.new_read(read(0xff0010, 0, true), 0xffff).unwrap();
    origins.bind_read(first, read(0xff0010, 0, true)).unwrap();
    let changed = origins.new_read(read(0xe00010, 1, true), 0xffff).unwrap();
    assert!(origins.bind_read(changed, read(0xe00010, 1, true)).is_err());
}

#[test]
fn missing_duplicate_changed_and_out_of_range_read_markers_fail_closed() {
    let mut origins = Origins::new();
    let access = read(0xff0010, 0x1234, true);
    let occurrence = origins.new_read(access, 0xffff).unwrap();
    let source = origins.read_source(occurrence, 1).unwrap();
    assert!(origins.demand(source).is_err());
    assert!(origins.check_read(occurrence).is_err());
    assert!(
        origins
            .bind_read(occurrence, read(0xff0010, 0x1235, true))
            .is_err()
    );
    origins.bind_read(occurrence, access).unwrap();
    assert!(origins.bind_read(occurrence, access).is_err());
    assert!(origins.bind_read(usize::MAX, access).is_err());
    assert!(origins.read_source(usize::MAX, 1).is_err());
    assert!(origins.read_source(occurrence, 0xffff).is_ok());
    assert!(origins.demand(Origin(usize::MAX)).is_err());
    assert!(
        origins.demand(source).is_ok(),
        "a rejected query must not poison or refresh the source"
    );
}

#[test]
fn driven_bits_are_checked_at_demand_not_replaced_by_an_observed_sample() {
    let mut origins = Origins::new();
    let access = read(0xa11100, 0x0100, true);
    let occurrence = origins.new_read(access, 0x0100).unwrap();
    origins.bind_read(occurrence, access).unwrap();
    let valid = origins.read_source(occurrence, 0x0100).unwrap();
    let undefined = origins.read_source(occurrence, 0x0101).unwrap();
    assert!(origins.demand(valid).is_ok());
    assert!(origins.demand(undefined).is_err());
    assert!(origins.demand(valid).is_ok());
}

#[test]
fn cyclic_and_unbound_output_providers_never_become_empty_sources() {
    let mut origins = Origins::new();
    let access = read(0xff0010, 0, true);
    let occurrence = origins.new_read(access, 0xffff).unwrap();
    let source = origins.read_source(occurrence, 1).unwrap();
    origins.fields[0][0] = source;
    let outputs = origins.finish();
    assert!(outputs.demand(StateField::Data(0), 1).is_err());
    let mut origins = Origins::new();
    let occurrence = origins.new_read(access, 0xffff).unwrap();
    let source = origins.read_source(occurrence, 1).unwrap();
    let mut sources = [Origin::EMPTY; 16];
    sources[0] = source;
    origins.store(write(0xff0010, 0, true), &sources);
    origins.bind_read(occurrence, access).unwrap();
    assert!(origins.demand(source).is_err());
}

#[test]
fn endpoint_masks_and_graph_lifetimes_are_call_specific() {
    let mut first = Origins::new();
    let access = read(0xff0010, 0xabcd, true);
    let occurrence = first.new_read(access, 0xffff).unwrap();
    first.bind_read(occurrence, access).unwrap();
    first.fields[0][0] = first.read_source(occurrence, 0xff00).unwrap();
    first.fields[0][1] = first.read_source(occurrence, 0x00ff).unwrap();
    let outputs = first.finish();
    let mut unrelated = Origins::new();
    unrelated.store(write(0xff0010, 0, true), &[Origin::EMPTY; 16]);
    let high = outputs.demand(StateField::Data(0), 1).unwrap();
    assert_eq!(high.memory.len(), 1);
    assert_eq!(
        high.memory[&0xff0010],
        MemoryInput {
            value: 0xab,
            bits: 0xff
        }
    );
    let low = outputs.demand(StateField::Data(0), 2).unwrap();
    assert_eq!(low.memory.len(), 1);
    assert_eq!(
        low.memory[&0xff0011],
        MemoryInput {
            value: 0xcd,
            bits: 0xff
        }
    );
    assert_eq!(
        outputs.demand(StateField::Data(0), 3).unwrap().memory.len(),
        2
    );
    assert!(outputs.demand(StateField::Data(8), 0).is_err());
    assert!(outputs.demand(StateField::Address(7), 0).is_err());
}

#[test]
fn stack_transport_is_data_only_and_does_not_drop_an_index_or_memory_label() {
    let mut origins = Origins::new();
    let base = origins.registers(StateMask::single(StateField::Ssp, u32::MAX));
    let index = origins.registers(StateMask::single(StateField::Data(3), 0xff));
    let data = origins.union(base, index);
    let transported = origins.stack_transport(data, StateField::Ssp).unwrap();
    let controls = origins.registers(StateMask::single(StateField::Ssp, 1));
    let access = read(0xff0010, 0, true);
    let occurrence = origins.new_read(access, 0xffff).unwrap();
    origins.bind_read(occurrence, access).unwrap();
    let memory = origins.read_source(occurrence, 0x80).unwrap();
    let source = origins.union(transported, controls);
    let source = origins.union(source, memory);
    let demand = origins.demand(source).unwrap();
    assert_eq!(demand.registers.get(StateField::Ssp), 1);
    assert_eq!(demand.registers.get(StateField::Data(3)), 0xff);
    assert_eq!(demand.memory[&0xff0011].bits, 0x80);
}

#[test]
fn identical_address_width_purpose_and_sample_never_merge_distinct_occurrences() {
    let mut origins = Origins::new();
    let access = read(0xff0010, 0, true);
    let first = origins.new_read(access, 0xffff).unwrap();
    origins.bind_read(first, access).unwrap();
    let old = origins.read_source(first, 0xffff).unwrap();
    let sources = register_sources(&mut origins, StateField::Data(3));
    origins.store(write(0xff0010, 0, true), &sources);
    let second = origins.new_read(access, 0xffff).unwrap();
    origins.bind_read(second, access).unwrap();
    let new = origins.read_source(second, 0xffff).unwrap();
    assert_ne!(old, new);
    assert_eq!(new, origins.read_source(second, 0xffff).unwrap());
    assert_eq!(origins.demand(old).unwrap().memory.len(), 2);
    assert_eq!(
        origins.demand(new).unwrap().registers,
        StateMask::single(StateField::Data(3), 0xffff)
    );
}
