use super::get_level_music_index;
use tje_mem::{AddressSpace, Bus, RAM_START, Revision};

fn memory(revision: Revision) -> AddressSpace {
    AddressSpace::new(vec![0; 64], revision)
}
#[test]
fn leaf_arithmetic_reads_and_writes_synthetic_memory() {
    let mut bus = memory(Revision::Rev02);
    for (argument, expected) in [
        (i16::MIN, -3),
        (-1, -2),
        (0, -1),
        (1, 0),
        (2, 1),
        (6, 5),
        (7, 0),
        (i16::MAX, 0),
    ] {
        bus.write_i16(RAM_START, argument);
        let result = get_level_music_index(bus.read_i16(RAM_START));
        bus.write_i16(RAM_START + 2, result);
        assert_eq!(bus.read_i16(RAM_START + 2), expected);
    }
    assert_eq!(bus.ram()[0..2], [0x7f, 0xff]);
}
