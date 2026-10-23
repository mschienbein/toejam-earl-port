use tje_mem::{Address, AddressSpace, Bus, ObjectState, OpenArray, Revision, Target, ram};

fn every_word_index<const STRIDE: usize>() {
    let pointer = Address::<[u8; STRIDE]>::new(0x00ffa25a);
    let array = Address::<[[u8; STRIDE]; 40]>::new(pointer.address());
    let open = Address::<OpenArray<[u8; STRIDE]>>::new(pointer.address());
    for index in i16::MIN..=i16::MAX {
        // Deliberately widen the independent expected product before narrowing.
        let bytes = (i64::from(index) * STRIDE as i64) as i16;
        let expected = pointer.address().wrapping_add(bytes as i32 as u32);
        assert_eq!(pointer.offset_word_index(index).address(), expected);
        assert_eq!(array.at_word_index(index).address(), expected);
        assert_eq!(open.at_word_index(index).address(), expected);
    }
}

#[test]
fn word_scaling_wraps_all_signed_word_values_for_each_stride_class() {
    every_word_index::<1>();
    every_word_index::<18>();
    every_word_index::<38>();
    every_word_index::<128>();
    every_word_index::<512>();
    every_word_index::<65537>();
}

#[test]
fn rain_cloud_index_512_wraps_to_player_zero_before_base_addition() {
    let players = ram::g_player_info(Revision::Rev02);
    assert_eq!(ObjectState::SIZE, 128);
    assert_eq!(players.at_word_index(512).address(), players.address());
    assert_eq!(players.at_word_index(-512).address(), players.address());
    assert_eq!(
        players.at_word_index(256).address(),
        players.address() - 32768
    );
    assert_eq!(
        players.at_word_index(-256).address(),
        players.address() - 32768
    );
    assert_eq!(
        players.at_word_index(511).address(),
        players.address() - 128
    );
    assert_eq!(
        players.at_long_index(512).address(),
        players.address() + 65536
    );
    assert_eq!(
        players.at_long_index(-512).address(),
        players.address() - 65536
    );

    let mut memory = AddressSpace::new(vec![0; 0x10000], Revision::Rev02);
    players.at_word_index(512).x().write(&mut memory, 123);
    assert_eq!(players.at_word_index(0).x().read(&memory), 123);
    // The wrong positive long product crosses the work-RAM window into ROM.
    assert_eq!(players.at_long_index(512).x().read(&memory), 0);
    players.at_long_index(512).x().write(&mut memory, -1);
    assert_eq!(players.at_word_index(0).x().read(&memory), 123);
    assert_eq!(memory.read_i16(players.address()), 123);
}

#[test]
fn char_extension_precedes_the_selected_scale_and_address_width() {
    let array = Address::<[[u8; 512]; 40]>::new(0x40000);
    assert_eq!(array.at_word_index(i16::from(64_i8)).address(), 0x38000);
    assert_eq!(array.at_long_index(i32::from(64_i8)).address(), 0x48000);
    assert_eq!(array.at_word_index(i16::from(-128_i8)).address(), 0x40000);
    assert_eq!(array.at_long_index(i32::from(-128_i8)).address(), 0x30000);
    assert_eq!(array.at_word_index(i16::from(255_u8)).address(), 0x3fe00);
    assert_eq!(array.at_long_index(i32::from(255_u8)).address(), 0x5fe00);
}

fn long_edges<const STRIDE: usize>() {
    let pointer = Address::<[u8; STRIDE]>::new(0xfffffff0);
    let bounded = Address::<[[u8; STRIDE]; 1]>::new(pointer.address());
    let open = Address::<OpenArray<[u8; STRIDE]>>::new(pointer.address());
    for index in [
        i32::MIN,
        -65537,
        -32769,
        -512,
        -1,
        0,
        1,
        512,
        32768,
        65536,
        i32::MAX,
    ] {
        let expected = (i64::from(pointer.address()) + i64::from(index) * STRIDE as i64) as u32;
        assert_eq!(pointer.offset_long_index(index).address(), expected);
        assert_eq!(bounded.at_long_index(index).address(), expected);
        assert_eq!(open.at_long_index(index).address(), expected);
        assert_eq!(pointer.offset(index).address(), expected);
        assert_eq!(bounded.at(index).address(), expected);
        assert_eq!(open.at(index).address(), expected);
    }
}

#[test]
fn long_indexing_keeps_full_word_product_and_u32_base_wrap() {
    long_edges::<1>();
    long_edges::<18>();
    long_edges::<38>();
    long_edges::<128>();
    long_edges::<512>();
}

#[test]
fn explicit_index_helpers_remain_const_and_unchecked() {
    const WRAPPED: u32 = Address::<[[u8; 128]; 1]>::new(0xffa25a)
        .at_word_index(512)
        .address();
    const LONG: u32 = Address::<OpenArray<[u8; 128]>>::new(0xffa25a)
        .at_long_index(512)
        .address();
    assert_eq!(WRAPPED, 0xffa25a);
    assert_eq!(LONG, 0x0100a25a);
}
