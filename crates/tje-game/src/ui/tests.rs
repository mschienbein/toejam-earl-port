use super::*;
use crate::{Context, Dispatcher, FunctionId, ids};
use std::collections::VecDeque;
use tje_mem::{AddressSpace, Bus, Revision, rom};

struct Fixture {
    memory: AddressSpace,
    data: [u32; 8],
    addresses: [u32; 8],
    status: u16,
    reads: VecDeque<(u32, u8)>,
    word_reads: VecDeque<(u32, u16)>,
    writes: Vec<(u32, u16, bool)>,
}

impl Fixture {
    fn new(revision: Revision) -> Self {
        Self {
            memory: AddressSpace::new(vec![0; 0x100000], revision),
            data: [0; 8],
            addresses: [0; 8],
            status: 0x2700,
            reads: VecDeque::new(),
            word_reads: VecDeque::new(),
            writes: Vec::new(),
        }
    }

    fn call(&mut self, id: FunctionId, arguments: &[u32]) -> u32 {
        Dispatcher::hybrid().call(self, id, arguments).unwrap()
    }
}

impl Bus for Fixture {
    fn read_u8(&self, address: u32) -> u8 {
        self.memory.read_u8(address)
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        self.memory.write_u8(address, value);
    }
}
impl tje_mem::Platform for Fixture {
    fn write_vdp_control(&mut self, value: u16) {
        crate::Platform::write_device_u16(self, 0xc00004, value);
    }
    fn write_vdp_data(&mut self, value: u16) {
        crate::Platform::write_device_u16(self, 0xc00000, value);
    }
}
impl crate::Platform for Fixture {
    fn read_device_u8(&mut self, address: u32) -> u8 {
        let (expected, value) = self.reads.pop_front().expect("unexpected device read");
        assert_eq!(address, expected);
        value
    }
    fn read_device_u16(&mut self, address: u32) -> u16 {
        let (expected, value) = self
            .word_reads
            .pop_front()
            .expect("unexpected word device read");
        assert_eq!(address, expected);
        value
    }
    fn write_device_u8(&mut self, address: u32, value: u8) {
        self.writes.push((address, u16::from(value), false));
    }
    fn write_device_u16(&mut self, address: u32, value: u16) {
        self.writes.push((address, value, true));
    }
}
impl Context for Fixture {
    fn revision(&self) -> Revision {
        self.memory.revision()
    }
    fn data_register(&self, index: usize) -> u32 {
        self.data[index]
    }
    fn set_data_register(&mut self, index: usize, value: u32) {
        self.data[index] = value;
    }
    fn address_register(&self, index: usize) -> u32 {
        self.addresses[index]
    }
    fn set_address_register(&mut self, index: usize, value: u32) {
        self.addresses[index] = value;
    }
    fn status_register(&self) -> u16 {
        self.status
    }
    fn set_status_register(&mut self, value: u16) {
        self.status = value;
    }
    fn dispatcher(&self) -> Dispatcher {
        Dispatcher::hybrid()
    }
    fn call_original(&mut self, id: FunctionId, _: &[u32]) -> Result<u32> {
        Err(crate::Error::Untranslated(id))
    }
}

#[test]
fn action_icon_position_publishes_volatile_registers_and_every_byte_origin_ccr() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut bytes = vec![0; 0x100000];
        let table = rom::spr_action_icons(revision).at(0).address() as usize;
        // Authored test descriptor in RAM, not copied cartridge sprite data.
        let descriptor = 0xff6000_u32;
        for index in 0..3 {
            bytes[table + index * 4..table + index * 4 + 4]
                .copy_from_slice(&descriptor.to_be_bytes());
        }
        let mut c = Fixture::new(revision);
        c.memory = AddressSpace::new(bytes, revision);
        c.write_u8(descriptor + 2, (-7_i8) as u8);
        for player in [0, 1] {
            for lives in [-1, 0] {
                ram::g_player_lives(revision)
                    .at(player)
                    .write(&mut c, lives);
                for own in [0, 1] {
                    ram::g_player_inventory_open(revision)
                        .at(player)
                        .write(&mut c, own);
                    for split in [0, -1] {
                        ram::g_split_screen_enabled(revision).write(&mut c, split);
                        for other in [0, 3] {
                            ram::g_player_inventory_open(revision)
                                .at(player ^ 1)
                                .write(&mut c, other);
                            for powerup in [-1, 0] {
                                ram::g_player_powerups(revision)
                                    .at(player)
                                    .write(&mut c, powerup);
                                let y_base = if lives < 0 {
                                    1
                                } else if own != 0 {
                                    if player == 0 { 218 } else { 321 }
                                } else if split != 0 || other != 0 {
                                    if powerup == -1 {
                                        1
                                    } else if player == 0 {
                                        136
                                    } else {
                                        249
                                    }
                                } else {
                                    321
                                };
                                for origin in i8::MIN..=i8::MAX {
                                    c.write_u8(descriptor + 3, origin as u8);
                                    for entry_d1 in [0x1234_abcd, 0xffff_5555] {
                                        c.data = [0x8765_4321; 8];
                                        c.data[1] = entry_d1;
                                        c.addresses = [0x7654_3210; 8];
                                        c.status = 0xa71f;
                                        c.call(
                                            ids::HUD_POSITION_ACTION_ICON_SPRITE,
                                            &[0x7777_0000 | player as u32],
                                        );
                                        let y = i32::from(origin) + y_base;
                                        let high = if lives >= 0 && own == 0 && split == 0 {
                                            0
                                        } else {
                                            entry_d1 & 0xffff0000
                                        };
                                        assert_eq!(c.data[0], y as u32);
                                        assert_eq!(
                                            c.data[1],
                                            high | if player == 0 { 168 } else { 176 }
                                        );
                                        assert_eq!(
                                            c.addresses[0],
                                            ram::g_hardware_sprite_table(revision).at(0).address()
                                        );
                                        assert_eq!(
                                            c.addresses[1],
                                            ram::g_player_powerups(revision).address()
                                        );
                                        assert_eq!(&c.data[2..], &[0x8765_4321; 6]);
                                        assert_eq!(&c.addresses[2..], &[0x7654_3210; 6]);
                                        let x = u16::from(origin < 0 && y >= 0) << 4;
                                        let nz = if y == 0 {
                                            4
                                        } else if y < 0 {
                                            8
                                        } else {
                                            0
                                        };
                                        assert_eq!(c.status, 0xa700 | x | nz);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn glyph_controls_signed_inputs_and_cache_wrap() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Fixture::new(revision);
        for (character, expected) in [
            (b'a', 1),
            (b'z', 26),
            (b'A', 1),
            (b'Z', 26),
            (b'0', 36),
            (b'9', 45),
            (b'.', 31),
            (b',', 32),
            (b':', 33),
            (b'\'', 33),
            (b'-', 34),
            (b'!', 35),
            (0x80, 0),
            (0xff, 0),
            (0, 0),
            (b' ', 0),
        ] {
            assert_eq!(
                c.call(ids::TEXT_MAP_CHARACTER_TO_GLYPH, &[u32::from(character)]),
                expected
            );
        }
        for (character, expected, words) in [
            (1, 46, &[][..]),
            (2, 49, &[0x85e3, 0x85e4][..]),
            (3, 51, &[0x85e6][..]),
            (4, 53, &[0x85e8][..]),
            (5, 56, &[0x85ea, 0x85eb][..]),
            (6, 58, &[0x85ed][..]),
        ] {
            c.writes.clear();
            assert_eq!(
                c.call(ids::TEXT_MAP_CHARACTER_TO_GLYPH, &[character]),
                expected
            );
            assert_eq!(
                c.writes,
                words
                    .iter()
                    .map(|&word| (0xc00000, word, true))
                    .collect::<Vec<_>>()
            );
        }
        for index in [0, 57, 58, u16::MAX] {
            ram::g_random_cache_index(revision).write(&mut c, index);
            let next = if index == 58 {
                0
            } else {
                index.wrapping_add(1)
            };
            ram::g_cached_random_values(revision)
                .at(i32::from(next))
                .write(&mut c, 0x1236);
            assert_eq!(
                c.call(ids::TEXT_MAP_CHARACTER_TO_GLYPH, &[u32::from(b'?')]),
                29
            );
            assert_eq!(ram::g_random_cache_index(revision).read(&c), next);
        }
    }
}

#[test]
fn font_banks_are_read_from_each_revisions_rom() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut bytes = vec![0; 0x100000];
        for (bank, source) in [
            rom::image_text_chars1(revision),
            rom::image_text_chars2(revision),
        ]
        .into_iter()
        .enumerate()
        {
            for word in 0..472_u32 {
                let value = ((bank as u32 + 1) << 24) | word;
                let address = source.address() as usize + word as usize * 4;
                bytes[address..address + 4].copy_from_slice(&value.to_be_bytes());
            }
        }
        let mut c = Fixture::new(revision);
        c.memory = AddressSpace::new(bytes, revision);
        c.call(ids::TEXT_COPY_FONT_BANKS_TO_RAM, &[]);
        for bank in 0..2 {
            for word in 0..472 {
                assert_eq!(
                    ram::g_text_font_buffers(revision)
                        .at(bank)
                        .cast::<u32>()
                        .offset(word)
                        .read(&c),
                    ((bank as u32 + 1) << 24) | word as u32
                );
            }
        }
    }
}

#[test]
fn cameras_use_full_int_truth_and_restore_both_open_views() {
    let revision = Revision::Rev02;
    let mut c = Fixture::new(revision);
    ram::g_camera1_pending_x(revision).write(&mut c, -123);
    ram::g_camera1_pending_y(revision).write(&mut c, 456);
    ram::g_camera2_pending_x(revision).write(&mut c, 789);
    ram::g_camera2_pending_y(revision).write(&mut c, -321);
    for player in [0, 1] {
        ram::g_player_inventory_open(revision)
            .at(player)
            .write(&mut c, -1);
    }
    c.call(ids::INVENTORY_SET_CAMERA_SUSPENSION, &[0x10000]);
    assert_eq!(ram::g_camera1_pending_x(revision).read(&c), -123);
    ram::g_split_screen_enabled(revision).write(&mut c, -1);
    c.call(ids::INVENTORY_SET_CAMERA_SUSPENSION, &[0x10000]);
    assert_eq!(ram::g_camera1_pending_x(revision).read(&c), 0);
    assert_eq!(ram::g_camera2_pending_y(revision).read(&c), 112);
    c.call(ids::INVENTORY_SET_CAMERA_SUSPENSION, &[0]);
    assert_eq!(ram::g_camera1_pending_x(revision).read(&c), -123);
    assert_eq!(ram::g_camera1_pending_y(revision).read(&c), 456);
    assert_eq!(ram::g_camera2_pending_x(revision).read(&c), 789);
    assert_eq!(ram::g_camera2_pending_y(revision).read(&c), -321);
}

#[test]
fn selected_slot_narrows_but_selector_retreats_in_place() {
    let revision = Revision::Rev02;
    let mut c = Fixture::new(revision);
    ram::g_player_inventory_selection_row(revision)
        .at(0)
        .write(&mut c, 64);
    ram::g_player_inventory(revision)
        .at(0)
        .at(-128)
        .write(&mut c, -3);
    assert_eq!(
        c.call(ids::INVENTORY_GET_SELECTED_PRESENT, &[0]),
        (-3_i32) as u32
    );
    for player in [0, 1] {
        for slot in 0..16 {
            ram::g_player_inventory(revision)
                .at(player)
                .at(slot)
                .write(&mut c, -1);
        }
        ram::g_player_inventory(revision)
            .at(player)
            .at(1)
            .write(&mut c, 7);
        ram::g_player_inventory_selection_row(revision)
            .at(player)
            .write(&mut c, 2);
        ram::g_player_inventory_selection_column(revision)
            .at(player)
            .write(&mut c, 1);
        let selector = if player == 0 {
            ram::g_player1_inventory_selector_entity(revision)
        } else {
            ram::g_player2_inventory_selector_entity(revision)
        };
        selector.behavior_flags().write(&mut c, 0xff);
        c.call(ids::INVENTORY_UPDATE_SELECTOR_ENTITY, &[player as u32]);
        assert_eq!(
            ram::g_player_inventory_selection_row(revision)
                .at(player)
                .read(&c),
            0
        );
        assert_eq!(
            ram::g_player_inventory_selection_column(revision)
                .at(player)
                .read(&c),
            1
        );
        assert_eq!(selector.x().read(&c), 167);
        assert_eq!(selector.y().read(&c), if player == 0 { 63 } else { 167 });
        assert_eq!(selector.behavior_flags().read(&c), 0xbf);
        ram::g_player_inventory(revision)
            .at(player)
            .at(1)
            .write(&mut c, -1);
        c.call(ids::INVENTORY_UPDATE_SELECTOR_ENTITY, &[player as u32]);
        assert_eq!(selector.behavior_flags().read(&c), 0xff);
    }
}

#[test]
fn controller_presence_preserves_widths_order_and_early_exit() {
    for (first, second, expected) in [(15, 15, 0), (0, 15, 0), (0, 0, 1)] {
        let mut c = Fixture::new(Revision::Rev02);
        for (data, control, value) in [(0xa10003, 0xa10009, first)]
            .into_iter()
            .chain((first != 15).then_some((0xa10005, 0xa1000b, second)))
        {
            c.reads.extend([
                (data, 0x77),
                (data, value),
                (control, 0x40),
                (data, 0x77),
                (data, value),
            ]);
        }
        c.word_reads.push_back((0xa11100, 0x100));
        let id = ids::ARE_BOTH_CONTROLLERS_CONNECTED;
        assert_eq!(c.call(id, &[]), expected);
        assert!(c.reads.is_empty());
        assert!(c.word_reads.is_empty());
        let mut expected_writes = vec![
            (0xa11100, 0x100, true),
            (0xa10009, 0x40, false),
            (0xa10003, 0, false),
            (0xa10009, 0, false),
            (0xa10003, 0, false),
        ];
        if first != 15 {
            expected_writes.extend([
                (0xa1000b, 0x40, false),
                (0xa10005, 0, false),
                (0xa1000b, 0, false),
                (0xa10005, 0, false),
            ]);
        }
        expected_writes.push((0xa11100, 0, true));
        assert_eq!(c.writes, expected_writes);
    }
}

#[test]
fn cursor_handle_word_wrap_and_coordinate_narrowing() {
    let revision = Revision::Rev02;
    let mut c = Fixture::new(revision);
    ram::g_menu_cursor_entity(revision).write(&mut c, 512);
    c.call(
        ids::POSITION_MENU_CURSOR_ENTITY,
        &[i16::MAX as u32, i16::MIN as i32 as u32],
    );
    let entity = ram::g_player_info(revision).at(0);
    assert_eq!(entity.x().read(&c), -8);
    assert_eq!(entity.y().read(&c), 6);
}

#[test]
fn text_background_color_preserves_command_word_order_and_short_truth() {
    let revision = Revision::Rev02;
    let mut c = Fixture::new(revision);
    c.call(ids::FORCE_TEXT_BACKGROUND_COLOR_TO_BLACK, &[0x10000]);
    assert_eq!(
        c.writes,
        [
            (0xc00004, 0xc01e, true),
            (0xc00004, 0, true),
            (0xc00000, 0x200, true)
        ]
    );
}

#[test]
fn ui_leaf_native_closure_and_original_classification_are_complete() {
    let roots: Vec<_> = TRANSLATIONS
        .iter()
        .filter(|translation| translation.id.metadata().callees.is_empty())
        .map(|translation| translation.id)
        .collect();
    assert!(Dispatcher::native(&roots).is_ok());
    for function in crate::functions().filter(|function| function.subsystem == "ui") {
        let atomic = ATOMIC_CAPABLE
            .iter()
            .filter(|&&id| id == function.id)
            .count();
        let non_atomic = EXECUTION_CONTRACTS
            .iter()
            .filter(|contract| contract.function == function.id)
            .count();
        assert_eq!(atomic + non_atomic, 1, "{}", function.name);
    }
    let all_ui_roots: Vec<_> = TRANSLATIONS.iter().map(|entry| entry.id).collect();
    match Dispatcher::native(&all_ui_roots) {
        Ok(_) => println!("UI_NATIVE_CLOSURE complete"),
        Err(missing) => {
            assert!(
                missing
                    .iter()
                    .all(|id| Dispatcher::hybrid().translation(*id).is_none())
            );
            let names: Vec<_> = missing.iter().map(|id| id.metadata().name).collect();
            println!("UI_NATIVE_CLOSURE missing {names:?}");
        }
    }
}

#[test]
fn word_scaled_entity_address_matches_every_original_short_index() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let context = Fixture::new(revision);
        let base = ram::g_player_info(revision).at(0).address();
        for bits in 0..=u16::MAX {
            let index = bits as i16;
            let displacement = i32::from((u32::from(bits).wrapping_mul(128) as u16) as i16);
            assert_eq!(
                super::entities::entity(&context, index).address(),
                base.wrapping_add(displacement as u32)
            );
        }
    }
}

#[test]
fn pending_inventory_candidates_are_not_production_registered() {
    for entry in PENDING_TRANSLATIONS {
        assert!(!TRANSLATIONS.iter().any(|proven| proven.id == entry.id));
        assert!(PENDING.contains(&entry.id));
    }
}
