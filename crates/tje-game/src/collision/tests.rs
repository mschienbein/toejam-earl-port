use super::*;
use crate::{Dispatcher, Platform};
use tje_mem::{AddressSpace, Bus, Ptr, Revision};

struct Machine {
    memory: AddressSpace,
    data: [u32; 8],
    address: [u32; 8],
    sr: u16,
}
impl Machine {
    fn new(revision: Revision) -> Self {
        Self {
            memory: AddressSpace::new(vec![], revision),
            data: [0x1357_2468; 8],
            address: [0x2468_1357; 8],
            sr: 0x2710,
        }
    }
}
impl Bus for Machine {
    fn read_u8(&self, address: u32) -> u8 {
        self.memory.read_u8(address)
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        self.memory.write_u8(address, value);
    }
}
impl tje_mem::Platform for Machine {
    fn write_vdp_control(&mut self, _: u16) {
        panic!("collision query accessed VDP");
    }
    fn write_vdp_data(&mut self, _: u16) {
        panic!("collision query accessed VDP");
    }
}
impl Platform for Machine {
    fn read_device_u8(&mut self, _: u32) -> u8 {
        panic!("collision query accessed a device");
    }
    fn read_device_u16(&mut self, _: u32) -> u16 {
        panic!("collision query accessed a device");
    }
    fn write_device_u8(&mut self, _: u32, _: u8) {
        panic!("collision query accessed a device");
    }
    fn write_device_u16(&mut self, _: u32, _: u16) {
        panic!("collision query accessed a device");
    }
}
impl Context for Machine {
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
        self.address[index]
    }
    fn set_address_register(&mut self, index: usize, value: u32) {
        self.address[index] = value;
    }
    fn status_register(&self) -> u16 {
        self.sr
    }
    fn set_status_register(&mut self, value: u16) {
        self.sr = value;
    }
    fn dispatcher(&self) -> Dispatcher {
        Dispatcher::hybrid()
    }
    fn call_original(&mut self, _: crate::FunctionId, _: &[u32]) -> Result<u32> {
        panic!("collision leaf called another function");
    }
}

fn setup(revision: Revision) -> (Machine, Address<ObjectState>, Address<ObjectState>) {
    let mut machine = Machine::new(revision);
    let entity = ram::g_player_info(revision).at(6);
    let target = ram::g_player_info(revision).at(7);
    for (object, index) in [(entity, 6), (target, 7)] {
        object.x().write(&mut machine, 100);
        object.y().write(&mut machine, 100);
        object.height().write(&mut machine, 20);
        object.width().write(&mut machine, 32);
        object.height_pixels().write(&mut machine, 20);
        object.entity_index().write(&mut machine, index);
        object.draw_flags().write(&mut machine, 1);
        object.behavior_flags().write(&mut machine, 1);
        object.level().write(&mut machine, 1);
        object.next().write(&mut machine, Ptr::new(0));
    }
    ram::g_entity_draw_list_head(revision).write(&mut machine, Ptr::new(target.address()));
    (machine, entity, target)
}

#[test]
fn narrow_keeps_n_only_word_magnitudes_and_strict_boundaries() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let (mut machine, entity, _) = setup(revision);
        let check = |m: &mut Machine, x: i16, y: i16| {
            entity_find_narrow_collision_at_position(m, &[entity.address(), x as u32, y as u32, 0])
                .unwrap()
                .unwrap()
        };
        assert_eq!(check(&mut machine, 121, 107), 8);
        assert_eq!(check(&mut machine, 122, 100), 0);
        assert_eq!(check(&mut machine, 100, 108), 0);
        entity.height().write(&mut machine, 30);
        assert_eq!(check(&mut machine, 100, 100), 8);
        entity.height().write(&mut machine, 31);
        assert_eq!(check(&mut machine, 100, 100), 0);
        // SUB.W produces 0x8000. NEG.W leaves it negative, unlike a wide abs.
        entity
            .height()
            .write(&mut machine, 20_i16.wrapping_add(i16::MIN));
        assert_eq!(check(&mut machine, 100_i16.wrapping_add(i16::MIN), 100), 8);
        assert_eq!(machine.data[1], 0x1357_0064);
        assert_eq!(machine.sr & 0x1f, 0);
    }
}

#[test]
fn menu_retains_signed_indices_word_increment_and_menu_pointer() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let (mut machine, entity, target) = setup(revision);
        target.entity_index().write(&mut machine, -1);
        let menu = ram::g_player_inventory_open(revision);
        menu.at(-1).write(&mut machine, 0);
        let args = [entity.address(), 100, 100];
        assert_eq!(
            entity_find_menu_aware_collision_at_position(&mut machine, &args).unwrap(),
            Some(0)
        );
        assert_eq!(machine.address[0], menu.address());
        menu.at(-1).write(&mut machine, 1);
        assert_eq!(
            entity_find_menu_aware_collision_at_position(&mut machine, &args).unwrap(),
            Some(0xffff_0000)
        );
        assert_eq!(machine.sr & 0x1f, 0x15);
        assert_eq!(machine.data[1], 0x1357_0064);
    }
}

#[test]
fn volume_preserves_zero_extended_y_and_asymmetric_height_endpoints() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let (mut machine, entity, target) = setup(revision);
        let check = |m: &mut Machine, y: i16, height: i16| {
            entity_find_volume_collision_at_position(
                m,
                &[entity.address(), 100, y as u32, height as u32, 0],
            )
            .unwrap()
            .unwrap()
        };
        assert_eq!(check(&mut machine, 100, 40), 0);
        assert_eq!(check(&mut machine, 100, 39), 8);
        assert_eq!(check(&mut machine, 100, 0), 8);
        assert_eq!(check(&mut machine, 100, -1), 0);
        assert_eq!(check(&mut machine, 100_i16.wrapping_add(i16::MIN), 20), 0);
        target.height_pixels().write(&mut machine, 255);
        assert_eq!(check(&mut machine, 100, 20), 0);
        entity.height_pixels().write(&mut machine, 255);
        assert_eq!(check(&mut machine, 100, 19), 0);
    }
}

#[test]
fn projectile_accepts_y_eight_but_has_strict_vertical_padding_and_no_self_gate() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let (mut machine, entity, target) = setup(revision);
        let check = |m: &mut Machine| {
            projectile_find_collision_excluding_owner(m, &[entity.address(), 99])
                .unwrap()
                .unwrap()
        };
        entity.y().write(&mut machine, 108);
        assert_eq!(check(&mut machine), 8);
        entity.height().write(&mut machine, 13);
        assert_eq!(check(&mut machine), 0);
        entity.height().write(&mut machine, 14);
        assert_eq!(check(&mut machine), 8);
        entity.height().write(&mut machine, 48);
        assert_eq!(check(&mut machine), 0);
        entity.height().write(&mut machine, 47);
        assert_eq!(check(&mut machine), 8);
        target.height_pixels().write(&mut machine, 255);
        assert_eq!(check(&mut machine), 8);
        ram::g_entity_draw_list_head(revision).write(&mut machine, Ptr::new(entity.address()));
        assert_eq!(check(&mut machine), 7);
    }
}

#[test]
fn broad_phase_uses_current_y_and_stops_without_scanning_later_links() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let (mut machine, entity, target) = setup(revision);
        let later = ram::g_player_info(revision).at(8);
        target.next().write(&mut machine, Ptr::new(later.address()));
        // An early stop must not dereference this deliberately invalid next link.
        later.next().write(&mut machine, Ptr::new(0x500000));
        target.y().write(&mut machine, 80);
        assert_eq!(
            entity_find_narrow_collision_at_position(&mut machine, &[entity.address(), 100, 80, 0])
                .unwrap(),
            Some(0)
        );
        target.y().write(&mut machine, 100);
        target.next().write(&mut machine, Ptr::new(0));
        assert_eq!(
            entity_find_horizontal_collision_at_x(&mut machine, &[entity.address(), 131, 30000, 0])
                .unwrap(),
            Some(8)
        );
        assert_eq!(
            entity_find_horizontal_collision_at_x(&mut machine, &[entity.address(), 132, 100, 0])
                .unwrap(),
            Some(0)
        );
        ram::g_entity_draw_list_head(revision).write(&mut machine, Ptr::new(0));
        machine.sr = 0x271f;
        assert_eq!(
            entity_find_horizontal_collision_at_x(&mut machine, &[entity.address(), 0, 0, 0])
                .unwrap(),
            Some(0)
        );
        assert_eq!(machine.sr, 0x2714);
    }
}
