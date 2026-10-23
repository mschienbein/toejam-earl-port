use tje_game::{Context, Dispatcher, Error, FunctionId, ids};
use tje_mem::{AddressSpace, Bus, Revision, arithmetic::ArithmeticTrap, ram};

struct Fixture {
    memory: AddressSpace,
    registers: [u32; 8],
    addresses: [u32; 7],
    status: u16,
    dispatcher: Dispatcher,
    original_calls: Vec<FunctionId>,
    ports: Vec<(u32, u16, bool)>,
}
impl Fixture {
    fn new(dispatcher: Dispatcher) -> Self {
        Self {
            memory: AddressSpace::new(vec![0; 64], Revision::Rev02),
            registers: [0; 8],
            addresses: [0; 7],
            status: 0x2700,
            dispatcher,
            original_calls: Vec::new(),
            ports: Vec::new(),
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
    fn write_vdp_control(&mut self, value: u16) {
        tje_game::Platform::write_device_u16(self, 0xc00004, value);
    }
    fn write_vdp_data(&mut self, value: u16) {
        tje_game::Platform::write_device_u16(self, 0xc00000, value);
    }
}
impl tje_game::Platform for Fixture {
    fn read_device_u8(&mut self, address: u32) -> u8 {
        panic!("unexpected device read {address:06X}")
    }
    fn read_device_u16(&mut self, address: u32) -> u16 {
        panic!("unexpected device read {address:06X}")
    }
    fn write_device_u8(&mut self, address: u32, value: u8) {
        self.ports.push((address, u16::from(value), false));
    }
    fn write_device_u16(&mut self, address: u32, value: u16) {
        self.ports.push((address, value, true));
    }
}
impl Context for Fixture {
    fn revision(&self) -> Revision {
        self.memory.revision()
    }
    fn data_register(&self, index: usize) -> u32 {
        self.registers[index]
    }
    fn set_data_register(&mut self, index: usize, value: u32) {
        self.registers[index] = value;
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
        self.dispatcher
    }
    fn call_original(&mut self, id: FunctionId, _: &[u32]) -> tje_game::Result<u32> {
        self.original_calls.push(id);
        Err(Error::Untranslated(id))
    }
}

#[test]
fn register_runtime_and_exception_contract() {
    let dispatcher = Dispatcher::native(&[
        ids::LMODT,
        ids::LDIVT,
        ids::LMULT,
        ids::ULMODT,
        ids::ULDIVT,
        ids::ABS,
    ])
    .unwrap_or_else(|_| panic!("runtime closure incomplete"));
    let mut context = Fixture::new(dispatcher);
    for (id, a, b, expected) in [
        (ids::LMODT, -7_i32 as u32, 3, -1_i32 as u32),
        (ids::LDIVT, -7_i32 as u32, 3, -2_i32 as u32),
        (ids::LDIVT, i32::MIN as u32, u32::MAX, i32::MIN as u32),
        (ids::ULMODT, u32::MAX, 65536, 65535),
        (ids::ULDIVT, u32::MAX, 0xffff8000, 1),
        (ids::LMULT, u32::MAX, 2, 0xfffffffe),
    ] {
        context.registers = [a, b, 2, 3, 4, 5, 6, 7];
        assert_eq!(dispatcher.call(&mut context, id, &[]).unwrap(), expected);
        assert_eq!(context.registers[0], expected);
        assert_eq!(&context.registers[2..], &[2, 3, 4, 5, 6, 7]);
    }
    assert_eq!(
        dispatcher
            .call(&mut context, ids::ABS, &[i32::MIN as u32])
            .unwrap(),
        i32::MIN as u32
    );
    for id in [ids::LMODT, ids::LDIVT, ids::ULMODT, ids::ULDIVT] {
        context.registers[1] = 0;
        assert_eq!(
            dispatcher.call(&mut context, id, &[]),
            Err(Error::Arithmetic(ArithmeticTrap::DivideByZero))
        );
    }
    assert!(context.original_calls.is_empty());
}

#[test]
fn native_never_falls_back_and_translated_nested_calls_stay_rust() {
    let dispatcher = Dispatcher::native(&[ids::PRE_GENERATE_RANDOMS])
        .unwrap_or_else(|_| panic!("random closure incomplete"));
    let mut context = Fixture::new(dispatcher);
    ram::g_rng_state(Revision::Rev02).write(&mut context, 12345);
    dispatcher
        .call(&mut context, ids::PRE_GENERATE_RANDOMS, &[])
        .unwrap();
    assert_eq!(ram::g_rng_state(Revision::Rev02).read(&context), 12345);
    assert_eq!(
        ram::g_cached_random_values(Revision::Rev02)
            .at(0)
            .read(&context),
        tje_game::map::park_miller(12345) as u16
    );
    assert_eq!(
        dispatcher.call(&mut context, ids::FURTHER_INITIALISATION, &[]),
        Err(Error::Untranslated(ids::FURTHER_INITIALISATION))
    );
    assert!(context.original_calls.is_empty());
    let hybrid = Dispatcher::hybrid_with(&[]);
    assert!(
        hybrid
            .call(&mut context, ids::FURTHER_INITIALISATION, &[])
            .is_err()
    );
    assert_eq!(context.original_calls, [ids::FURTHER_INITIALISATION]);
}

#[test]
fn leaf_truth_promotion_and_alias_contracts() {
    let dispatcher = Dispatcher::hybrid();
    let mut context = Fixture::new(dispatcher);
    for rank in [-128_i8, -1, 0, 1, 8, 127] {
        ram::g_player_rank(Revision::Rev02)
            .at(0)
            .write(&mut context, rank);
        assert_eq!(
            dispatcher
                .call(&mut context, ids::PLAYER_GET_MAXIMUM_HEALTH, &[0])
                .unwrap(),
            (23_i32 + 4 * i32::from(rank)) as u32
        );
    }
    for (a, b, expected) in [(0, 0, 1), (0, -1, 1), (-1, 0, 1), (-1, -1, 0)] {
        ram::g_timed_effect_entities(Revision::Rev02)
            .at(0)
            .write(&mut context, a);
        ram::g_timed_effect_entities(Revision::Rev02)
            .at(1)
            .write(&mut context, b);
        assert_eq!(
            dispatcher
                .call(&mut context, ids::ENTITY_HAS_FREE_TIMED_EFFECT_SLOT, &[])
                .unwrap(),
            expected
        );
    }
    for player in [0, 1] {
        for swapped in [0, -1] {
            ram::g_controller_ports_swapped(Revision::Rev02).write(&mut context, swapped);
            ram::g_controller_buttons_ac_swapped(Revision::Rev02).write(&mut context, -1);
            ram::g_controller1_raw_input(Revision::Rev02).write(&mut context, 0xff40);
            ram::g_controller2_raw_input(Revision::Rev02).write(&mut context, 0x20);
            let expected = if (player == 0) != (swapped != 0) {
                0x20
            } else {
                0x40
            };
            assert_eq!(
                dispatcher
                    .call(&mut context, ids::READ_MAPPED_PLAYER_INPUT, &[player])
                    .unwrap(),
                expected
            );
        }
    }
}
