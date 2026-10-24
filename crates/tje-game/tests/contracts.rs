use tje_game::execution::ExecutionClass;
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
    execution: ExecutionClass,
    original_class: Option<ExecutionClass>,
    /// Explicit frozen diagnostic clock provider for these ROM-free fixtures.
    clock: Option<tje_game::execution::CallClock>,
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
            execution: ExecutionClass::Atomic,
            original_class: None,
            clock: Some(tje_game::execution::CallClock::default()),
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
    fn execution_class(&self) -> ExecutionClass {
        self.execution
    }
    fn set_execution_class(&mut self, class: ExecutionClass) -> tje_game::Result<()> {
        self.execution = class;
        Ok(())
    }
    fn clocked_execution(&self) -> bool {
        self.clock
            .as_ref()
            .is_some_and(tje_game::execution::CallClock::active)
    }
    fn enter_call_ceiling(
        &mut self,
        function: FunctionId,
    ) -> tje_game::Result<tje_game::execution::CallCeilingScope> {
        if let Some(clock) = &mut self.clock {
            clock
                .enter(function, tje_game::execution::CallCeiling::DEFAULT)
                .map_err(Error::Clock)
        } else if tje_game::execution_class(function) == Some(ExecutionClass::Atomic) {
            Ok(tje_game::execution::CallCeilingScope::not_required())
        } else {
            Err(tje_game::execution::CallClockError::Unavailable.into())
        }
    }
    fn leave_call_ceiling(
        &mut self,
        scope: tje_game::execution::CallCeilingScope,
    ) -> tje_game::Result<()> {
        if let Some(clock) = &mut self.clock {
            clock.leave(scope).map(|_| ()).map_err(Error::Clock)
        } else if scope.is_required() {
            Err(tje_game::execution::CallClockError::Unavailable.into())
        } else {
            Ok(())
        }
    }
    fn call_original_with_execution(
        &mut self,
        id: FunctionId,
        arguments: &[u32],
        class: ExecutionClass,
    ) -> tje_game::Result<u32> {
        self.original_class = Some(class);
        self.call_original(id, arguments)
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
            .call(&mut context, ids::GET_RANDOM_NUMBER, &[])
            .is_err()
    );
    assert_eq!(context.original_calls, [ids::GET_RANDOM_NUMBER]);
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

#[test]
fn production_classes_scope_rust_and_original_calls_and_restore_on_errors() {
    fn functional(context: &mut dyn Context, _: &[u32]) -> tje_game::Result<Option<u32>> {
        assert_eq!(context.execution_class(), ExecutionClass::Functional);
        let result = context
            .dispatcher()
            .call(context, ids::ABS, &[(-7_i32) as u32])?;
        assert_eq!(context.execution_class(), ExecutionClass::Functional);
        Ok(Some(result))
    }
    fn failing(context: &mut dyn Context, _: &[u32]) -> tje_game::Result<Option<u32>> {
        assert_eq!(context.execution_class(), ExecutionClass::Functional);
        Err(Error::Arithmetic(ArithmeticTrap::DivideByZero))
    }
    static PENDING: &[tje_game::Translation] = &[
        tje_game::Translation {
            id: ids::MOVE_DATA_TO_VRAM,
            function: functional,
        },
        tje_game::Translation {
            id: ids::CLEAR_COLOUR_RAM,
            function: failing,
        },
    ];
    let dispatcher = Dispatcher::hybrid_with_overrides(PENDING);
    let mut context = Fixture::new(dispatcher);
    assert!(matches!(
        dispatcher.call(&mut context, ids::MOVE_DATA_TO_VRAM, &[]),
        Err(Error::ExecutionUnavailable { .. })
    ));
    context.execution = ExecutionClass::TimeAware;
    assert_eq!(
        dispatcher.call(&mut context, ids::MOVE_DATA_TO_VRAM, &[]),
        Ok(7)
    );
    assert_eq!(context.execution, ExecutionClass::TimeAware);
    assert_eq!(
        dispatcher.call(&mut context, ids::CLEAR_COLOUR_RAM, &[]),
        Err(Error::Arithmetic(ArithmeticTrap::DivideByZero))
    );
    assert_eq!(context.execution, ExecutionClass::TimeAware);
    assert!(context.original_calls.is_empty()); // proven ABS stayed Rust
    let residual = Dispatcher::hybrid_with(&[]);
    assert_eq!(
        residual.call(&mut context, ids::MOVE_DATA_TO_VRAM, &[]),
        Err(Error::Untranslated(ids::MOVE_DATA_TO_VRAM))
    );
    assert_eq!(context.original_class, Some(ExecutionClass::Functional));
    assert_eq!(context.execution, ExecutionClass::TimeAware); // caller envelope retained
    context.execution = ExecutionClass::Functional;
    let wait = ids::WAIT_FOR_NEXT_V_BLANK;
    assert!(matches!(
        residual.call(&mut context, wait, &[]),
        Err(Error::ExecutionUnavailable { .. })
    ));
}

#[test]
fn a_time_aware_root_without_clock_capability_fails_before_effects() {
    fn forbidden_effect(context: &mut dyn Context, _: &[u32]) -> tje_game::Result<Option<u32>> {
        context.write_u8(0xff9000, 0x55);
        Ok(None)
    }
    static BODY: &[tje_game::Translation] = &[tje_game::Translation {
        id: ids::WAIT_FOR_NEXT_V_BLANK,
        function: forbidden_effect,
    }];
    let mut fixture = Fixture::new(Dispatcher::hybrid_with(BODY));
    fixture.execution = ExecutionClass::TimeAware;
    fixture.clock = None;
    assert_eq!(
        tje_game::call(&mut fixture, ids::WAIT_FOR_NEXT_V_BLANK, &[1]),
        Err(Error::Clock(
            tje_game::execution::CallClockError::Unavailable
        )),
    );
    assert_eq!(fixture.read_u8(0xff9000), 0);
    assert!(fixture.original_calls.is_empty());
    assert!(fixture.ports.is_empty());
}

#[test]
fn zero_time_rng_rejection_expires_work_without_repairing_rng_outputs() {
    use tje_game::execution::{CallCeiling, CallClockError, ClockResource};
    fn reject_forever(context: &mut dyn Context, _: &[u32]) -> tje_game::Result<Option<u32>> {
        loop {
            let value = tje_game::call(context, ids::GET_RANDOM_NUMBER, &[])?;
            context.write_u16(0xff9000, value as u16);
        }
    }
    // An authored unbounded callback using the actual pure-Rust RNG, not a
    // cartridge-equivalence fixture for the overridden wait identity.
    static BODY: &[tje_game::Translation] = &[
        tje_game::Translation {
            id: ids::WAIT_FOR_NEXT_V_BLANK,
            function: reject_forever,
        },
        tje_game::Translation {
            id: ids::GET_RANDOM_NUMBER,
            function: tje_game::map::get_random_number,
        },
    ];
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut fixture = Fixture::new(Dispatcher::hybrid_with(BODY));
        fixture.memory = AddressSpace::new(vec![0; 64], revision);
        fixture.execution = ExecutionClass::TimeAware;
        ram::g_rng_state(revision).write(&mut fixture, 1);
        let root = fixture
            .clock
            .as_mut()
            .unwrap()
            .enter(
                ids::WAIT_FOR_NEXT_V_BLANK,
                CallCeiling::new(4, 100).unwrap(),
            )
            .unwrap();
        let error = tje_game::call(&mut fixture, ids::WAIT_FOR_NEXT_V_BLANK, &[1]).unwrap_err();
        assert!(matches!(
            error,
            Error::Clock(CallClockError::CeilingExceeded {
                resource: ClockResource::DispatchSteps,
                ..
            })
        ));
        let expected = tje_game::map::park_miller(tje_game::map::park_miller(1));
        assert_eq!(ram::g_rng_state(revision).read(&fixture), expected);
        assert_eq!(fixture.read_u16(0xff9000), expected as u16);
        let clock = fixture.clock.as_mut().unwrap();
        assert!(clock.leave(root).is_err());
        let usage = clock.usage();
        assert_eq!(usage.dispatch_steps, 4);
        assert_eq!(usage.ticks, 0);
        assert_eq!(usage.instructions, 0);
        assert_eq!(usage.master_clocks, 0);
        assert_eq!(usage.frames, 0);
    }
}
