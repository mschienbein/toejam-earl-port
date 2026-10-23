//! Authored RAM/port programs; no cartridge image or captured game state.
use genesis_config::GenesisEmulatorConfig;
use genesis_core::GenesisBus;
use jgenesis_common::frontend::{PartialClone, TimingMode};
use m68000_emu::BusInterface;

#[derive(Clone, Copy)]
enum Write {
    Control(u16),
    Data(u16),
    DataByte(u8),
}

fn issue(bus: &mut GenesisBus, write: Write) {
    match write {
        Write::Control(value) => bus.write_word(0xc00004, value),
        Write::Data(value) => bus.write_word(0xc00000, value),
        Write::DataByte(value) => bus.write_byte(0xc00000, value),
    }
    bus.apply_pending_writes();
}

fn dma(source: u32, destination: u16, words: u16) -> [Write; 9] {
    [
        Write::Control(0x8114),
        Write::Control(0x8f02),
        Write::Control(0x9300 | (words & 255)),
        Write::Control(0x9400 | (words >> 8)),
        Write::Control(0x9500 | ((source >> 1) as u16 & 255)),
        Write::Control(0x9600 | ((source >> 9) as u16 & 255)),
        Write::Control(0x9700 | ((source >> 17) as u16 & 127)),
        Write::Control(0x4000 | (destination & 0x3fff)),
        Write::Control(0x0080 | (destination >> 14)),
    ]
}

fn target(destination: u16) -> [Write; 2] {
    [
        Write::Control(0x4000 | (destination & 0x3fff)),
        Write::Control(destination >> 14),
    ]
}

fn pattern(index: u16) -> u16 {
    index.wrapping_mul(37) ^ 0x5a5a
}

fn data(index: u16) -> Write {
    if index.is_multiple_of(29) {
        Write::DataByte(index as u8)
    } else {
        Write::Data(pattern(index))
    }
}

fn data_word(index: u16) -> u16 {
    match data(index) {
        Write::Data(value) => value,
        Write::DataByte(value) => u16::from(value) * 0x0101,
        Write::Control(_) => unreachable!(),
    }
}

fn pending_dma() -> GenesisBus {
    let mut bus = GenesisBus::new(TimingMode::Ntsc, None, &GenesisEmulatorConfig::default());
    bus.hybrid_accesses = Some(Vec::new());
    bus.hybrid_writes = Some(Vec::new());
    for index in 0..512_u16 {
        bus.memory
            .write_main_ram_word(0xff0000 + 2 * u32::from(index), pattern(index));
    }
    for write in dma(0xff0000, 0x1000, 128) {
        issue(&mut bus, write);
    }
    assert!(
        bus.vdp.should_halt_cpu(),
        "memory DMA must really own the CPU bus"
    );
    bus
}

fn compare_burst_to_serial(writes: &[Write]) -> GenesisBus {
    let mut burst = pending_dma();
    let mut serial = burst.partial_clone();
    let phase = (burst.vdp.scanline(), burst.vdp.scanline_mclk());
    let clocks = burst.cycles.m68k_mclk_cycles;

    // Real CPU issue cannot overtake this memory DMA. The serial comparison
    // completes each transaction with the existing check-only commit paths.
    serial.hybrid_set_functional_mode(true);
    for &write in writes {
        issue(&mut serial, write);
        issue(&mut burst, write);
    }
    assert!(burst.vdp.should_halt_cpu());
    burst.hybrid_set_functional_mode(true);

    assert!(!burst.vdp.should_halt_cpu());
    assert!(
        burst.vdp.hybrid_state_eq(&serial.vdp),
        "full VDP state, including FIFO/cache/latches"
    );
    assert_eq!(burst.hybrid_accesses, serial.hybrid_accesses);
    assert_eq!(burst.hybrid_writes, serial.hybrid_writes);
    assert_eq!(burst.cycles.m68k_mclk_cycles, clocks);
    assert_eq!((burst.vdp.scanline(), burst.vdp.scanline_mclk()), phase);
    burst
}

fn assert_words(bus: &GenesisBus, address: usize, words: impl IntoIterator<Item = u16>) {
    let (vram, _, _) = bus.vdp.hybrid_memories();
    for (index, value) in words.into_iter().enumerate() {
        let offset = address + 2 * index;
        assert_eq!(
            u16::from_be_bytes([vram[offset], vram[offset + 1]]),
            value,
            "VRAM {offset:04X}"
        );
    }
}

#[test]
fn small_and_large_deferred_port_batches_preserve_every_transfer_and_width() {
    // The large cases cover the two reported logical-batch sizes; they are not
    // hardware FIFO capacities or new storage limits.
    for count in [0_u16, 1, 10, 328, 5581] {
        let writes: Vec<_> = target(0x4000)
            .into_iter()
            .chain((0..count).map(data))
            .collect();
        let bus = compare_burst_to_serial(&writes);
        assert_words(&bus, 0x1000, (0..128).map(pattern));
        assert_words(&bus, 0x4000, (0..count).map(data_word));
        assert_eq!(bus.vdp.hybrid_registers().dma_length, 0);
        assert_eq!(bus.vdp.hybrid_registers().dma_source_address, 0xff0100);
    }
}

#[test]
fn chained_dma_retains_redeferred_writes_once_in_original_issue_order() {
    let mut writes = Vec::new();
    writes.extend(target(0x2000));
    writes.extend((0..17).map(data));
    writes.extend(dma(0xff0200, 0x3000, 3));
    writes.extend(target(0x4000));
    writes.extend((0..328).map(data));
    // These writes are re-deferred while the second DMA is active. Iterating
    // the growing queue, clearing it wholesale, or reversing it loses order.
    writes.extend(target(0x4800));
    writes.push(Write::Data(0x1357));
    writes.push(Write::DataByte(0x9a));
    let bus = compare_burst_to_serial(&writes);
    assert_words(&bus, 0x1000, (0..128).map(pattern));
    assert_words(&bus, 0x2000, (0..17).map(data_word));
    assert_words(&bus, 0x3000, (256..259).map(pattern));
    assert_words(&bus, 0x4000, (0..328).map(data_word));
    assert_words(&bus, 0x4800, [0x1357, 0x9a9a]);
    assert_eq!(bus.vdp.hybrid_registers().dma_length, 0);
    assert_eq!(bus.vdp.hybrid_registers().dma_source_address, 0xff0206);
}

#[test]
fn game_over_sized_deferred_batch_preserves_wrapped_vram_and_bounded_fifo() {
    // Two command words plus 32877 data words reproduce the third reported
    // deferred count (32879), including a complete wrap through 64-KiB VRAM.
    let count = 32877_u16;
    let writes: Vec<_> = target(0x4000)
        .into_iter()
        .chain((0..count).map(data))
        .collect();
    let bus = compare_burst_to_serial(&writes);
    let mut expected = [0_u8; 65536];
    for index in 0..count {
        let address = (0x4000 + 2 * usize::from(index)) & 0xffff;
        let [high, low] = data_word(index).to_be_bytes();
        expected[address] = high;
        expected[address + 1] = low;
    }
    let (vram, _, _) = bus.vdp.hybrid_memories();
    assert_eq!(vram, expected.as_slice());
}
