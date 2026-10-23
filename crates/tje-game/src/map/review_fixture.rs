//! Opt-in RH04/RH05 differential regressions on a real gameplay state.
//! Args: ROM START_GAME_INPUT METRICS. No cartridge/state bytes are recorded.
use std::{error::Error, fs};
use tje_game::{FunctionId, Translation, ids, map};
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::{Address, Revision, ram};
type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;
static ENTRIES: &[Translation] = &[
    Translation {
        id: ids::MAP_TRACE_TRANSITION_CHAIN,
        function: map::map_trace_transition_chain,
    },
    Translation {
        id: ids::MAP_EXPAND_REGIONS_BESIDE_PATHS,
        function: map::map_expand_regions_beside_paths,
    },
    Translation {
        id: ids::MAP_GET_BANK_PATH_DIRECTIONS,
        function: map::map_get_bank_path_directions,
    },
];
fn cell(r: Revision, row: i16, column: i16) -> Address<u16> {
    ram::g_player1_level_map(r)
        .at_word_index(row)
        .at_word_index(column)
}
fn canonical(address: u32) -> Option<u32> {
    let address = address & 0xffffff;
    if address >= 0xe00000 {
        Some(0xff0000 | (address & 0xffff))
    } else {
        None
    }
}
fn external_ram(address: Address<u16>) -> bool {
    canonical(address.address()).is_some_and(|value| value >= 0xff0500)
}
fn check(host: &mut Host, id: FunctionId, args: &[u32], label: &str) -> Result<u32> {
    host.check_fixture(id.metadata().address(host.revision()), args)
        .map_err(|error| format!("{label}: {error}").into())
}
fn trace(host: &mut Host, row: i16, column: i16, label: &str) -> Result<bool> {
    let r = host.revision();
    let origin = cell(r, row, column);
    let neighbors = [
        cell(r, row.wrapping_sub(1), column),
        cell(r, row.wrapping_add(1), column),
        cell(r, row, column.wrapping_sub(1)),
        cell(r, row, column.wrapping_add(1)),
    ];
    if !external_ram(origin) || neighbors.iter().any(|&p| !external_ram(p)) {
        return Ok(false);
    }
    let mut c = host.context();
    for i in 0..3 {
        ram::g_level_anchor_positions(r)
            .at(i)
            .x()
            .write(&mut c, -120);
        ram::g_level_anchor_positions(r)
            .at(i)
            .y()
            .write(&mut c, -120);
    }
    for neighbor in neighbors {
        neighbor.write(&mut c, 0x1f00);
    }
    origin.write(&mut c, 28);
    ram::g_transition_path_visit_count(r).write(&mut c, 0);
    ram::g_transition_path_minimum_visits(r).write(&mut c, 0);
    let value = check(
        host,
        ids::MAP_TRACE_TRANSITION_CHAIN,
        &[i32::from(row) as u32, i32::from(column) as u32, 8, 8],
        label,
    )?;
    assert_eq!(value, 1, "{label}");
    Ok(true)
}
fn expand(host: &mut Host, row: i16, column: i16, label: &str) -> Result<bool> {
    let r = host.revision();
    let map = ram::g_player1_level_map(r);
    let origin = cell(r, row, column);
    // Source left neighbor has a word row displacement but a promoted long column.
    let left = map.at_word_index(row).at(i32::from(column) - 1);
    let start_row = row.wrapping_sub(1);
    let start_column = column.wrapping_sub(2);
    if i32::from(start_row) + 3 > i32::from(i16::MAX)
        || i32::from(start_column) + 2 > i32::from(i16::MAX)
    {
        return Ok(false);
    }
    let targets = [
        cell(r, start_row, start_column),
        cell(r, start_row, start_column.wrapping_add(1)),
        cell(r, start_row.wrapping_add(1), start_column),
        cell(r, start_row.wrapping_add(1), start_column.wrapping_add(1)),
        cell(r, start_row.wrapping_add(2), start_column),
        cell(r, start_row.wrapping_add(2), start_column.wrapping_add(1)),
    ];
    if !external_ram(origin) || !external_ram(left) || targets.iter().any(|&p| !external_ram(p)) {
        return Ok(false);
    }
    let checks = [
        (i32::from(row) - 1, i32::from(column) - 1),
        (i32::from(row) + 1, i32::from(column) - 1),
        (i32::from(row) - 1, i32::from(column) - 2),
        (i32::from(row) + 1, i32::from(column) - 2),
    ];
    // These long-expression reads must be mapped too; never invent open-bus data.
    if checks.iter().any(|&(y, x)| {
        let p = map.at(y).at(x).address() & 0xffffff;
        p >= 0x100000 && p < 0xe00000
    }) {
        return Ok(false);
    }
    let mut c = host.context();
    for target in targets {
        target.write(&mut c, 0);
    }
    left.write(&mut c, 0x1000);
    origin.write(&mut c, 8);
    let value = check(
        host,
        ids::MAP_EXPAND_REGIONS_BESIDE_PATHS,
        &[i32::from(row) as u32, i32::from(column) as u32],
        label,
    )?;
    if label.starts_with("RH05-") {
        assert_eq!(value, 1, "{label}");
    }
    Ok(true)
}
fn main() -> Result {
    let argv: Vec<String> = std::env::args().collect();
    if argv.len() != 4 {
        return Err("expected ROM INPUT METRICS".into());
    }
    let bytes = fs::read(&argv[1])?;
    let r = tje_rom::Rom::import(&bytes)?.revision();
    let script = InputScript::parse(&fs::read_to_string(&argv[2])?)?;
    let mut host = Host::new(&bytes, Mode::Check, HookRegistry::proof(r, ENTRIES)?)?;
    for frame in 0..1300 {
        host.run_frame(script.at(frame))?;
    }
    assert!(host.measurements.game_frame.is_some());
    let result = (|| -> Result<(usize, usize)> {
        // ReviewHazards' actual USP/SSP recipe keeps out-of-array input RAM above
        // the active stack frontier. It does not waive an uninitialized read.
        let mut c = host.context();
        let regs = c.cpu().address_registers();
        c.cpu().set_address_registers(regs, 0xff0400, 0xff0400);
        assert!(trace(&mut host, 1000, 0, "RH04-transition-row-word-scale")?);
        assert!(expand(&mut host, 901, 0, "RH05-expansion-row-word-scale")?);
        let mut traces = 1;
        let mut expansions = 1;
        for row in [901_i16, 1000, 2000, -2000, -1000, -901, -1, 0, 1, 5] {
            for column in [-32768_i16, -20000, -1, 0, 1, 16384, 20000, 32767] {
                if trace(&mut host, row, column, "word-index trace sweep")? {
                    traces += 1;
                }
                if expand(&mut host, row, column, "mixed-index expansion sweep")? {
                    expansions += 1;
                }
            }
        }
        // NC01: independent word terms are ADDED as long addresses, not rewrapped.
        host.context().write_u8(0xff2596, 0xa7);
        assert_eq!(
            check(
                &mut host,
                ids::MAP_GET_BANK_PATH_DIRECTIONS,
                &[0, 1000, 20000],
                "NC01-independent-word-adds"
            )?,
            0
        );
        Ok((traces, expansions))
    })();
    fs::write(
        &argv[3],
        serde_json::to_string_pretty(&serde_json::json!({
            "revision":r.name(),"frames":host.frame_count(),"game_frame":host.measurements.game_frame,
            "source":"ReviewHazards RH04/RH05 exact counterexamples plus labelled large/negative sweeps",
            "sweeps":result.as_ref().ok(),"error":result.as_ref().err().map(ToString::to_string),"checks":host.checks,
        }))?,
    )?;
    let counts = result?;
    assert_eq!(host.checks.mismatches, 0);
    assert!(host.checks.uninitialized_stack_reads.is_empty());
    assert!(host.checks.unexercised.is_empty());
    println!(
        "{}: RH04/RH05 corrected, {} trace and {} expansion fixtures, zero mismatches",
        r.name(),
        counts.0,
        counts.1
    );
    Ok(())
}
