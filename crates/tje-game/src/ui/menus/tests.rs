use super::{player_update_health_and_lives, text_write_decimal_tiles};
use crate::{
    Context, Dispatcher, FunctionId, Result, call_frame::FrameState, execution::ExecutionClass, ids,
};
use tje_mem::{AddressSpace, Bus, Revision, ram};

struct Fixture {
    memory: AddressSpace,
    data: [u32; 8],
    addresses: [u32; 7],
    status: u16,
    sp: u32,
    frame: FrameState,
    class: ExecutionClass,
    kills: Vec<u32>,
    strings: Vec<(u32, Vec<u8>)>,
    clock: crate::execution::CallClock,
}
impl Fixture {
    fn new(revision: Revision) -> Self {
        Self {
            memory: AddressSpace::new(vec![0; 0x100000], revision),
            data: std::array::from_fn(|index| 0x76540000 | index as u32),
            addresses: std::array::from_fn(|index| 0xff1000 + index as u32 * 4),
            status: 0x2000,
            sp: 0xff8000,
            frame: FrameState::new(0xff8000),
            class: ExecutionClass::TimeAware,
            kills: Vec::new(),
            strings: Vec::new(),
            clock: crate::execution::CallClock::default(),
        }
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
    fn write_vdp_control(&mut self, _: u16) {
        panic!("unexpected VDP control");
    }
    fn write_vdp_data(&mut self, _: u16) {
        panic!("unexpected VDP data");
    }
}
impl crate::Platform for Fixture {
    fn discard_u16(&mut self, address: u32) -> Result<()> {
        let _ = self.memory.read_u16(address);
        Ok(())
    }
    fn read_device_u8(&mut self, _: u32) -> u8 {
        panic!("unexpected device read");
    }
    fn read_device_u16(&mut self, _: u32) -> u16 {
        panic!("unexpected device read");
    }
    fn write_device_u8(&mut self, _: u32, _: u8) {
        panic!("unexpected device write");
    }
    fn write_device_u16(&mut self, _: u32, _: u16) {
        panic!("unexpected device write");
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
    fn stack_pointer(&self) -> Result<u32> {
        Ok(self.sp)
    }
    fn set_stack_pointer(&mut self, value: u32) -> Result<()> {
        self.sp = value;
        Ok(())
    }
    fn frame_state(&self) -> Option<FrameState> {
        Some(self.frame)
    }
    fn set_frame_state(&mut self, value: FrameState) -> Result<()> {
        self.frame = value;
        Ok(())
    }
    fn execution_class(&self) -> ExecutionClass {
        self.class
    }
    fn set_execution_class(&mut self, value: ExecutionClass) -> Result<()> {
        self.class = value;
        Ok(())
    }
    fn clocked_execution(&self) -> bool {
        self.clock.active() || self.class != ExecutionClass::Atomic
    }
    fn enter_call_ceiling(
        &mut self,
        function: FunctionId,
    ) -> Result<crate::execution::CallCeilingScope> {
        self.clock
            .enter(
                function,
                crate::execution_contract(function)
                    .and_then(|contract| contract.operational_ceiling)
                    .unwrap_or(crate::execution::CallCeiling::DEFAULT),
            )
            .map_err(crate::Error::Clock)
    }
    fn leave_call_ceiling(&mut self, scope: crate::execution::CallCeilingScope) -> Result<()> {
        self.clock
            .leave(scope)
            .map(|_| ())
            .map_err(crate::Error::Clock)
    }
    fn dispatcher(&self) -> Dispatcher {
        Dispatcher::hybrid_with(&[])
    }
    fn call_original(&mut self, id: FunctionId, arguments: &[u32]) -> Result<u32> {
        self.call_original_with_execution(id, arguments, ExecutionClass::Atomic)
    }
    fn call_original_with_execution(
        &mut self,
        id: FunctionId,
        arguments: &[u32],
        _: ExecutionClass,
    ) -> Result<u32> {
        assert_eq!(
            id,
            ids::KILL_THE_PLAYER,
            "unexpected dependency in isolated arithmetic test"
        );
        self.kills.push(arguments[0]);
        Ok(self.data[0])
    }
    fn call_original_entry(&mut self, id: FunctionId, class: ExecutionClass) -> Result<u32> {
        assert_eq!(id, ids::TEXT_WRITE_STRING_TILES);
        assert_eq!(class, ExecutionClass::Functional);
        let return_pc = self.read_u32(self.sp);
        let expected = ids::TEXT_WRITE_DECIMAL_TILES
            .metadata()
            .address(self.revision())
            + 0x8a;
        assert_eq!(return_pc, expected);
        let pointer = self.frame.argument_slot(0).read(self);
        let mut text = Vec::new();
        let mut address = pointer;
        while self.read_u8(address) != 0 {
            text.push(self.read_u8(address));
            address += 1;
        }
        self.strings.push((pointer, text));
        self.sp += 4;
        Ok(self.data[0])
    }
}

#[test]
fn health_predecrement_matches_subq_byte_n_xor_v_for_every_life_byte() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for player in 0..2 {
            let mut context = Fixture::new(revision);
            let object = ram::g_player_info(revision).at(player);
            object.entity_index().write(&mut context, player as i8);
            for bits in 0..=255_u8 {
                let lives = bits as i8;
                for saved in [-1, 0, 100] {
                    context.kills.clear();
                    ram::g_player_health(revision)
                        .at(player)
                        .write(&mut context, 0);
                    ram::g_player_lives(revision)
                        .at(player)
                        .write(&mut context, lives);
                    ram::g_player_health_before_swimming(revision)
                        .at(player)
                        .write(&mut context, saved);
                    player_update_health_and_lives(&mut context, &[object.address(), 0]).unwrap();
                    let decremented = lives.wrapping_sub(1);
                    let overflow = lives == i8::MIN;
                    let negative_branch = (decremented < 0) != overflow;
                    let expected_lives = if negative_branch { lives } else { decremented };
                    let expected_health = if negative_branch { 0 } else { 100 };
                    let expected_saved = if negative_branch || saved == -1 {
                        -1
                    } else {
                        100
                    };
                    assert_eq!(
                        ram::g_player_lives(revision).at(player).read(&context),
                        expected_lives
                    );
                    assert_eq!(
                        ram::g_player_health(revision).at(player).read(&context),
                        expected_health
                    );
                    assert_eq!(
                        ram::g_player_health_before_swimming(revision)
                            .at(player)
                            .read(&context),
                        expected_saved
                    );
                    assert_eq!(context.kills, [player as u32]);
                }
            }
        }
    }
}

#[test]
fn decimal_uses_original_emulated_array_and_preserves_minimum_short_bug() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (value, expected) in [
            (i16::MIN, "--.)*("),
            (-10000, "-10000"),
            (-1, "-1"),
            (0, "0"),
            (9, "9"),
            (10, "10"),
            (100, "100"),
            (32767, "32767"),
        ] {
            let mut context = Fixture::new(revision);
            let data = context.data;
            let addresses = context.addresses;
            text_write_decimal_tiles(&mut context, &[i32::from(value) as u32]).unwrap();
            assert_eq!(context.strings, [(0xff7ff0, expected.as_bytes().to_vec())]);
            assert_eq!(context.sp, 0xff8000);
            assert_eq!(context.frame, FrameState::new(0xff8000));
            assert_eq!(context.data, data);
            assert_eq!(context.addresses, addresses);
        }
    }
}
