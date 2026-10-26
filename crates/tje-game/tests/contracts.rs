use std::cell::{Cell, RefCell};
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
    d0_reads: Cell<usize>,
    reads: RefCell<Vec<u32>>,
    sp: u32,
    frame: tje_game::call_frame::FrameState,
    completion_consumed: bool,
    completion_error: Option<Error>,
    completion_requests: Vec<tje_game::timing::CompletionRequest>,
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
            d0_reads: Cell::new(0),
            reads: RefCell::new(Vec::new()),
            sp: 0xfff000,
            frame: tje_game::call_frame::FrameState::new(0xfff000),
            completion_consumed: false,
            completion_error: None,
            completion_requests: Vec::new(),
        }
    }
}
impl Bus for Fixture {
    fn read_u8(&self, address: u32) -> u8 {
        self.reads.borrow_mut().push(address);
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
        if index == 0 {
            self.d0_reads.set(self.d0_reads.get() + 1);
        }
        self.registers[index]
    }
    fn set_data_register(&mut self, index: usize, value: u32) {
        self.registers[index] = value;
    }
    fn address_register(&self, index: usize) -> u32 {
        if index == 7 {
            self.sp
        } else {
            self.addresses[index]
        }
    }
    fn set_address_register(&mut self, index: usize, value: u32) {
        if index == 7 {
            self.sp = value;
        } else {
            self.addresses[index] = value;
        }
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
    fn stack_pointer(&self) -> tje_game::Result<u32> {
        Ok(self.sp)
    }
    fn set_stack_pointer(&mut self, value: u32) -> tje_game::Result<()> {
        self.sp = value;
        Ok(())
    }
    fn stack_banks(&self) -> tje_game::Result<tje_game::exits::StackBanks> {
        if self.status & 0x2000 == 0 {
            return Err(Error::Platform(
                "this fixture models supervisor mode only".into(),
            ));
        }
        Ok(tje_game::exits::StackBanks {
            user: 0,
            supervisor: self.sp,
        })
    }
    fn frame_state(&self) -> Option<tje_game::call_frame::FrameState> {
        Some(self.frame)
    }
    fn set_frame_state(&mut self, frame: tje_game::call_frame::FrameState) -> tje_game::Result<()> {
        self.frame = frame;
        Ok(())
    }
    fn consume_timing_completion(
        &mut self,
        request: tje_game::timing::CompletionRequest,
    ) -> tje_game::Result<tje_game::timing::Completion> {
        self.completion_requests.push(request);
        if let Some(error) = self.completion_error.take() {
            return Err(error);
        }
        if self.completion_consumed {
            assert_eq!(request.kind, tje_game::timing::CompletionKind::Rts);
            self.sp = request.entry_sp.wrapping_add(4);
            Ok(tje_game::timing::Completion::Consumed)
        } else {
            Ok(tje_game::timing::Completion::Absent)
        }
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
                .enter(
                    function,
                    tje_game::execution_contract(function)
                        .and_then(|contract| contract.operational_ceiling)
                        .unwrap_or(tje_game::execution::CallCeiling::DEFAULT),
                )
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
    fn call_original_void_with_execution(
        &mut self,
        id: FunctionId,
        _: &[u32],
        class: ExecutionClass,
    ) -> tje_game::Result<()> {
        self.original_class = Some(class);
        self.original_calls.push(id);
        if id != ids::WAIT_FOR_NEXT_V_BLANK {
            return Err(Error::Untranslated(id));
        }
        self.registers[0] = 0xfeed_5eed;
        Ok(())
    }
    fn call_original_entry_void(
        &mut self,
        id: FunctionId,
        class: ExecutionClass,
    ) -> tje_game::Result<()> {
        self.original_class = Some(class);
        self.original_calls.push(id);
        let return_pc = self.memory.read_u32(self.sp);
        assert_eq!(return_pc, 0x1234);
        self.sp = self.sp.wrapping_add(4);
        self.registers[0] = 0xfeed_5eed;
        Ok(())
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
        for dividend in [0, 7, u32::MAX, i32::MIN as u32, 0x12348000] {
            for incoming_x in [0, 0x10] {
                let mut zero = Fixture::new(dispatcher);
                zero.registers = [dividend, 0, 0x89abcdef, 3, 4, 5, 6, 7];
                zero.status = 0x2700 | incoming_x;
                let entry_sp = zero.sp;
                let error = dispatcher.call(&mut zero, id, &[]).unwrap_err();
                assert!(error.preserves_call_frame());
                let Error::ControlTransfer(transfer) = error else {
                    panic!("zero divisor must remain a typed exception transfer");
                };
                let tje_game::exits::Exit::Exception { request } = transfer.exit else {
                    panic!("zero divisor must not manufacture an RTS result");
                };
                assert_eq!(transfer.entry, id);
                assert_eq!(
                    request.trap,
                    tje_game::control::SynchronousTrap::DivideByZero
                );
                assert_eq!(request.validate_shape(Revision::Rev02), Ok(()));
                assert_eq!(zero.sp, entry_sp.wrapping_sub(4));
                assert_eq!(zero.memory.read_u32(zero.sp), 0x89abcdef);
                assert_eq!(zero.registers[2], 0);
                let signed = id == ids::LMODT || id == ids::LDIVT;
                let negative = signed && (dividend as i32) < 0;
                let magnitude = if negative {
                    dividend.wrapping_neg()
                } else {
                    dividend
                };
                assert_eq!(zero.registers[1], magnitude >> 16);
                assert_eq!(zero.addresses[1], magnitude as i16 as i32 as u32);
                assert_eq!(
                    zero.registers[0],
                    if signed {
                        if negative { u32::MAX } else { 0 }
                    } else {
                        dividend
                    }
                );
                assert_eq!(
                    zero.addresses[0],
                    if signed {
                        zero.registers[0]
                    } else {
                        u32::from(id == ids::ULDIVT)
                    }
                );
                assert_eq!(
                    zero.status,
                    0x2700 | if negative { 0x10 } else { incoming_x }
                );
                assert_eq!(&zero.registers[3..], &[3, 4, 5, 6, 7]);
                assert!(zero.original_calls.is_empty());
                assert_eq!(
                    tje_game::exits::contract(id),
                    tje_game::exits::ExitContract::Rts
                );
            }
        }
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
        .call_void(&mut context, ids::PRE_GENERATE_RANDOMS, &[])
        .unwrap();
    assert_eq!(ram::g_rng_state(Revision::Rev02).read(&context), 12345);
    assert_eq!(
        ram::g_cached_random_values(Revision::Rev02)
            .at(0)
            .read(&context),
        tje_game::map::park_miller(12345) as u16
    );
    assert_eq!(
        dispatcher.call_void(&mut context, ids::FURTHER_INITIALISATION, &[]),
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
        dispatcher.call_void(&mut context, ids::MOVE_DATA_TO_VRAM, &[]),
        Err(Error::ExecutionUnavailable { .. })
    ));
    context.execution = ExecutionClass::TimeAware;
    assert_eq!(
        dispatcher.call_void(&mut context, ids::MOVE_DATA_TO_VRAM, &[]),
        Ok(())
    );
    assert_eq!(context.data_register(0), 7);
    assert_eq!(context.execution, ExecutionClass::TimeAware);
    assert_eq!(
        dispatcher.call_void(&mut context, ids::CLEAR_COLOUR_RAM, &[]),
        Err(Error::Arithmetic(ArithmeticTrap::DivideByZero))
    );
    assert_eq!(context.execution, ExecutionClass::TimeAware);
    assert!(context.original_calls.is_empty()); // proven ABS stayed Rust
    let residual = Dispatcher::hybrid_with(&[]);
    assert_eq!(
        residual.call_void(&mut context, ids::MOVE_DATA_TO_VRAM, &[]),
        Err(Error::Untranslated(ids::MOVE_DATA_TO_VRAM))
    );
    assert_eq!(context.original_class, Some(ExecutionClass::Functional));
    assert_eq!(context.execution, ExecutionClass::TimeAware); // caller envelope retained
    context.execution = ExecutionClass::Functional;
    let wait = ids::WAIT_FOR_NEXT_V_BLANK;
    assert!(matches!(
        residual.call_void(&mut context, wait, &[]),
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
        tje_game::call_void(&mut fixture, ids::WAIT_FOR_NEXT_V_BLANK, &[1]),
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
        let error =
            tje_game::call_void(&mut fixture, ids::WAIT_FOR_NEXT_V_BLANK, &[1]).unwrap_err();
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

#[test]
fn unsupported_discard_never_falls_back_to_a_semantic_read() {
    let mut context = Fixture::new(Dispatcher::hybrid());
    assert!(matches!(
        tje_game::Platform::discard_u8(&mut context, 0xff0100),
        Err(Error::Platform(_))
    ));
    assert!(matches!(
        tje_game::Platform::discard_u16(&mut context, 0xa11100),
        Err(Error::Platform(_))
    ));
    assert!(context.ports.is_empty());
}

// Authored API fixtures, not proofs of the catalogue functions used as ABI ids.
fn authored_void_register_effects(
    context: &mut dyn Context,
    _: &[u32],
) -> tje_game::Result<Option<u32>> {
    context.set_data_register(0, 0xc0de_0042);
    context.set_data_register(1, 0x1234_5678);
    context.set_status_register((context.status_register() & !0x1f) | 0x15);
    Ok(None)
}
fn authored_void_frame(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    let frame = context
        .frame_state()
        .ok_or(tje_game::call_frame::FrameError::Unavailable)?;
    let slot = frame.entry_sp.wrapping_add(4);
    let value = context.read_u32(slot);
    context.write_u32(slot, value ^ 0xffff_0000);
    authored_void_register_effects(context, &[])
}
fn authored_fatal_void_frame(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    let frame = context
        .frame_state()
        .ok_or(tje_game::call_frame::FrameError::Unavailable)?;
    context.set_data_register(0, 0xbad0_0042);
    context.set_stack_pointer(frame.entry_sp.wrapping_sub(2))?;
    Err(Error::TimingExecutionFailure {
        entry: ids::WAIT_FOR_NEXT_V_BLANK,
        cause: Box::new(Error::Platform("authored fatal callee frame".into())),
    })
}
static AUTHORED_VOID_VALUES: &[tje_game::Translation] = &[tje_game::Translation {
    id: ids::PRE_GENERATE_RANDOMS,
    function: authored_void_register_effects,
}];
static AUTHORED_VOID_FRAMED: &[tje_game::FramedTranslation] = &[tje_game::FramedTranslation {
    id: ids::WAIT_FOR_NEXT_V_BLANK,
    function: authored_void_frame,
}];
static AUTHORED_FATAL_VOID: &[tje_game::FramedTranslation] = &[tje_game::FramedTranslation {
    id: ids::WAIT_FOR_NEXT_V_BLANK,
    function: authored_fatal_void_frame,
}];

#[test]
fn typed_void_values_keep_actual_register_effects_without_observing_d0() {
    assert!(!ids::PRE_GENERATE_RANDOMS.metadata().abi.returns_value);
    for revision in [Revision::Rev02, Revision::Rev00] {
        for direct_dispatcher in [false, true] {
            let dispatcher = Dispatcher::hybrid_with(AUTHORED_VOID_VALUES);
            let mut context = Fixture::new(dispatcher);
            context.memory = AddressSpace::new(vec![0; 64], revision);
            context.registers[0] = 0xaaaa_5555;
            if direct_dispatcher {
                dispatcher
                    .call_void(&mut context, ids::PRE_GENERATE_RANDOMS, &[])
                    .unwrap();
            } else {
                tje_game::call_void(&mut context, ids::PRE_GENERATE_RANDOMS, &[]).unwrap();
            }
            assert_eq!(context.d0_reads.get(), 0);
            assert_eq!(context.registers[0], 0xc0de_0042);
            assert_eq!(context.registers[1], 0x1234_5678);
            assert_eq!(context.status, 0x2715);
            assert!(!context.clock.as_ref().unwrap().active());
            // An explicit caller register read is a separate observation.
            assert_eq!(context.data_register(0), 0xc0de_0042);
            assert_eq!(context.d0_reads.get(), 1);
        }
    }
}

#[test]
fn typed_void_frames_preserve_arguments_cleanup_and_optional_native_completion() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for consumed in [false, true] {
            for form in 0..3 {
                let dispatcher =
                    Dispatcher::hybrid_with_overrides_and_frames(&[], AUTHORED_VOID_FRAMED);
                let mut context = Fixture::new(dispatcher);
                context.memory = AddressSpace::new(vec![0; 64], revision);
                context.execution = ExecutionClass::TimeAware;
                context.completion_consumed = consumed;
                let sp = context.sp;
                let parent = context.frame;
                let site = tje_game::call_frame::CallSite::new(0x1234);
                match form {
                    0 => tje_game::call_at_void(
                        &mut context,
                        ids::WAIT_FOR_NEXT_V_BLANK,
                        &[0x1122_3344],
                        site,
                    )
                    .unwrap(),
                    1 => dispatcher
                        .call_at_void(
                            &mut context,
                            ids::WAIT_FOR_NEXT_V_BLANK,
                            &[0x1122_3344],
                            site,
                        )
                        .unwrap(),
                    _ => {
                        let outgoing = tje_game::call_frame::OutgoingArguments::push(
                            &mut context,
                            &[0x1122_3344],
                        )
                        .unwrap();
                        outgoing
                            .call_void(&mut context, dispatcher, ids::WAIT_FOR_NEXT_V_BLANK, site)
                            .unwrap();
                        assert_eq!(context.sp, sp - 4);
                        outgoing.release(&mut context).unwrap();
                    }
                }
                assert_eq!(context.sp, sp);
                assert_eq!(context.frame, parent);
                assert_eq!(context.execution, ExecutionClass::TimeAware);
                assert_eq!(context.memory.read_u32(sp - 4), 0xeedd_3344);
                assert_eq!(context.registers[0], 0xc0de_0042);
                assert_eq!(context.d0_reads.get(), 0);
                assert!(!context.clock.as_ref().unwrap().active());
                assert_eq!(context.completion_requests.len(), 1);
                assert_eq!(context.completion_requests[0].value, None);
                let return_reads = context
                    .reads
                    .borrow()
                    .iter()
                    .filter(|&&address| (sp - 8..sp - 4).contains(&address))
                    .count();
                assert_eq!(return_reads, if consumed { 0 } else { 4 });
                assert_eq!(
                    context
                        .reads
                        .borrow()
                        .iter()
                        .filter(|&&address| (sp - 4..sp).contains(&address))
                        .count(),
                    4,
                    "framed arguments stay lazy; no eager adapter snapshot"
                );
            }
        }
    }
}

#[test]
fn typed_void_rejects_valued_callees_before_arguments_frames_or_callbacks() {
    assert!(ids::GET_RANDOM_NUMBER.metadata().abi.returns_value);
    for framed in [false, true] {
        let dispatcher = Dispatcher::hybrid();
        let mut context = Fixture::new(dispatcher);
        let sp = context.sp;
        let frame = context.frame;
        let result = if framed {
            tje_game::call_at_void(
                &mut context,
                ids::GET_RANDOM_NUMBER,
                &[0x1122_3344],
                tje_game::call_frame::CallSite::new(0x1234),
            )
        } else {
            tje_game::call_void(&mut context, ids::GET_RANDOM_NUMBER, &[])
        };
        assert_eq!(
            result,
            Err(Error::CallReturnType {
                function: ids::GET_RANDOM_NUMBER,
                requested_value: false
            })
        );
        assert_eq!(context.sp, sp);
        assert_eq!(context.frame, frame);
        assert_eq!(context.memory.read_u32(sp - 4), 0);
        assert!(context.original_calls.is_empty());
        assert!(context.completion_requests.is_empty());
        assert!(context.reads.borrow().is_empty());
        assert_eq!(context.d0_reads.get(), 0);
        assert!(!context.clock.as_ref().unwrap().active());
    }
}

#[test]
fn typed_void_original_fallback_selects_only_the_no_result_capability() {
    for framed in [false, true] {
        let dispatcher = Dispatcher::hybrid();
        assert!(
            dispatcher
                .resolve_entry(ids::WAIT_FOR_NEXT_V_BLANK)
                .unwrap()
                .is_none()
        );
        let mut context = Fixture::new(dispatcher);
        context.execution = ExecutionClass::TimeAware;
        let sp = context.sp;
        let parent = context.frame;
        if framed {
            tje_game::call_at_void(
                &mut context,
                ids::WAIT_FOR_NEXT_V_BLANK,
                &[1],
                tje_game::call_frame::CallSite::new(0x1234),
            )
            .unwrap();
        } else {
            tje_game::call_void(&mut context, ids::WAIT_FOR_NEXT_V_BLANK, &[1]).unwrap();
        }
        assert_eq!(context.original_calls, [ids::WAIT_FOR_NEXT_V_BLANK]);
        assert_eq!(context.original_class, Some(ExecutionClass::TimeAware));
        assert_eq!(context.registers[0], 0xfeed_5eed);
        assert_eq!(context.d0_reads.get(), 0);
        assert_eq!(context.sp, sp);
        assert_eq!(context.frame, parent);
        assert!(!context.clock.as_ref().unwrap().active());
    }
}

#[test]
fn typed_void_fatal_callee_keeps_its_actual_frame_without_d0_observation() {
    let dispatcher = Dispatcher::hybrid_with_overrides_and_frames(&[], AUTHORED_FATAL_VOID);
    let mut context = Fixture::new(dispatcher);
    context.execution = ExecutionClass::TimeAware;
    let sp = context.sp;
    let result = tje_game::call_at_void(
        &mut context,
        ids::WAIT_FOR_NEXT_V_BLANK,
        &[1],
        tje_game::call_frame::CallSite::new(0x1234),
    );
    assert!(matches!(result, Err(Error::TimingExecutionFailure { .. })));
    assert_eq!(context.sp, sp - 10);
    assert_eq!(context.frame.entry_sp, sp - 8);
    assert_eq!(context.registers[0], 0xbad0_0042);
    assert_eq!(context.d0_reads.get(), 0);
    assert!(context.completion_requests.is_empty());
    assert!(!context.clock.as_ref().unwrap().active());
}

#[test]
fn typed_void_completion_failure_keeps_the_frame_and_no_scalar_expectation() {
    let dispatcher = Dispatcher::hybrid_with_overrides_and_frames(&[], AUTHORED_VOID_FRAMED);
    let mut context = Fixture::new(dispatcher);
    context.execution = ExecutionClass::TimeAware;
    context.completion_error = Some(Error::Platform("authored completion failure".into()));
    let sp = context.sp;
    let result = tje_game::call_at_void(
        &mut context,
        ids::WAIT_FOR_NEXT_V_BLANK,
        &[1],
        tje_game::call_frame::CallSite::new(0x1234),
    );
    assert!(matches!(result, Err(Error::TimingExecutionFailure { .. })));
    assert_eq!(context.sp, sp - 8);
    assert_eq!(context.frame.entry_sp, sp - 8);
    assert_eq!(context.completion_requests.len(), 1);
    assert_eq!(context.completion_requests[0].value, None);
    assert_eq!(context.d0_reads.get(), 0);
    assert!(!context.clock.as_ref().unwrap().active());
}

fn authored_void_with_explicit_d0(_: &mut dyn Context, _: &[u32]) -> tje_game::Result<Option<u32>> {
    Ok(Some(0x0dd0_0042))
}
static AUTHORED_VOID_D0: &[tje_game::Translation] = &[tje_game::Translation {
    id: ids::PRE_GENERATE_RANDOMS,
    function: authored_void_with_explicit_d0,
}];

#[test]
fn typed_void_keeps_explicit_body_d0_output_without_extracting_it() {
    let dispatcher = Dispatcher::hybrid_with(AUTHORED_VOID_D0);
    let mut context = Fixture::new(dispatcher);
    tje_game::call_void(&mut context, ids::PRE_GENERATE_RANDOMS, &[]).unwrap();
    assert_eq!(context.registers[0], 0x0dd0_0042);
    assert_eq!(context.d0_reads.get(), 0);
}

#[test]
fn scalar_calls_reject_void_ids_before_dispatch_frames_or_clock_effects() {
    use tje_game::call_frame::{CallSite, OutgoingArguments};
    for revision in [Revision::Rev02, Revision::Rev00] {
        for dispatcher in [
            Dispatcher::hybrid_with(AUTHORED_VOID_VALUES),
            Dispatcher::hybrid_with(&[]),
            Dispatcher::native_with(&[], AUTHORED_VOID_VALUES).unwrap(),
            Dispatcher::native(&[]).unwrap(),
        ] {
            for form in 0..6 {
                let mut context = Fixture::new(dispatcher);
                context.memory = AddressSpace::new(vec![0; 64], revision);
                context.registers[0] = 0x1234_5678;
                let sp = context.sp;
                let frame = context.frame;
                let site = CallSite::new(0x1234);
                let id = ids::PRE_GENERATE_RANDOMS;
                let result = match form {
                    0 => tje_game::call(&mut context, id, &[]),
                    1 => dispatcher.call(&mut context, id, &[]),
                    2 => tje_game::call_at(&mut context, id, &[0x1122_3344], site),
                    3 => dispatcher.call_at(&mut context, id, &[0x1122_3344], site),
                    4 => OutgoingArguments::existing(&context, 0).unwrap().call(
                        &mut context,
                        dispatcher,
                        id,
                        site,
                    ),
                    _ => tje_game::call_frame::call_at(
                        &mut context,
                        dispatcher,
                        id,
                        &[0x1122_3344],
                        site,
                    ),
                };
                let error = result.unwrap_err();
                assert_eq!(
                    error,
                    Error::CallReturnType {
                        function: id,
                        requested_value: true
                    }
                );
                let message = error.to_string();
                assert!(message.contains("call_void") && message.contains("call_at_void"));
                assert!(message.contains(id.metadata().name));
                assert_eq!(context.sp, sp);
                assert_eq!(context.frame, frame);
                assert_eq!(context.registers[0], 0x1234_5678);
                assert_eq!(context.memory.read_u32(sp - 4), 0);
                assert!(context.reads.borrow().is_empty());
                assert_eq!(context.d0_reads.get(), 0);
                assert!(context.original_calls.is_empty());
                assert!(context.completion_requests.is_empty());
                assert!(!context.clock.as_ref().unwrap().active());
            }
        }
    }
}

#[test]
fn native_void_dispatch_keeps_the_authored_body_register_effects() {
    let dispatcher =
        Dispatcher::native_with(&[ids::PRE_GENERATE_RANDOMS], AUTHORED_VOID_VALUES).unwrap();
    let mut context = Fixture::new(dispatcher);
    tje_game::call_void(&mut context, ids::PRE_GENERATE_RANDOMS, &[]).unwrap();
    assert_eq!(context.registers[0], 0xc0de_0042);
    assert_eq!(context.registers[1], 0x1234_5678);
    assert_eq!(context.d0_reads.get(), 0);
    assert!(context.original_calls.is_empty());
}

#[test]
fn missing_typed_control_capability_is_structured_and_has_no_effect() {
    use tje_game::Platform;
    use tje_game::control::{ControlRequest, RetryDelayKind};
    let requests = [
        ControlRequest::WriteStatusRegister { value: 0x2700 },
        ControlRequest::WriteUserStackPointer { value: 0 },
        ControlRequest::Z80BusRequest { value: 0x0100 },
        ControlRequest::Z80Reset { value: 0 },
        ControlRequest::RetryDelay(RetryDelayKind::Pcm24),
        ControlRequest::RetryDelay(RetryDelayKind::Psg124),
        ControlRequest::JoinVdpFill,
    ];
    for class in [
        ExecutionClass::Atomic,
        ExecutionClass::Functional,
        ExecutionClass::TimeAware,
    ] {
        let mut fixture = Fixture::new(Dispatcher::hybrid());
        fixture.execution = class;
        let registers = fixture.registers;
        let addresses = fixture.addresses;
        let status = fixture.status;
        let sp = fixture.sp;
        let frame = fixture.frame;
        for request in requests {
            assert_eq!(
                fixture.control_effect(request),
                Err(Error::UnsupportedControlEffect(request))
            );
            assert_eq!(fixture.registers, registers);
            assert_eq!(fixture.addresses, addresses);
            assert_eq!(fixture.status, status);
            assert_eq!(fixture.sp, sp);
            assert_eq!(fixture.frame, frame);
            assert!(fixture.ports.is_empty());
            assert!(fixture.reads.borrow().is_empty());
            assert!(fixture.original_calls.is_empty());
        }
    }
}
