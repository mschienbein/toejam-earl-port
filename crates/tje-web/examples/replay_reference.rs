//! Private reference stream for web/verify-replay.mjs. Never part of the wasm
//! application: native-only dev dependencies own the original CPU oracle.

#[cfg(target_arch = "wasm32")]
compile_error!("replay-reference is a native-only development exporter, not a browser oracle");

#[cfg(not(target_arch = "wasm32"))]
fn main() -> anyhow::Result<()> {
    use anyhow::{Context, ensure};
    use std::io::{BufWriter, Write};
    use tje_engine_verify::state::{ChipWrite as ReferenceWrite, Origin};
    use tje_engine_verify::{FrameState, Machine, reference::ReferenceMachine};
    use tje_rom::Rom;
    use tje_session::{CONNECTED, Session, Start};
    use tje_web::observation::{ChipWrite, Cpu, Frame, encode};

    fn publish(writer: &mut impl Write, number: u64, state: FrameState) -> anyhow::Result<()> {
        let frame = Frame {
            frame: number,
            position: (
                state.position.pc,
                state.position.master_clock,
                state.position.scanline,
                state.position.line_master_clock,
            ),
            cpu: Cpu {
                data: state.cpu.data,
                address: state.cpu.address,
                usp: state.cpu.usp,
                ssp: state.cpu.ssp,
                sr: state.cpu.sr,
                prefetch: state.cpu.prefetch,
                last_muldiv: state.cpu.last_muldiv,
            },
            work_ram: state.work_ram,
            vram: state.vram,
            cram: state.cram,
            vsram: state.vsram,
            vdp_registers: state.vdp_registers,
            vdp_state: state.vdp_state,
            z80_ram: state.z80_ram,
            width: state.width,
            height: state.height,
            pixels: state.pixels,
            audio_bits: state.audio_bits,
            chip_writes: state
                .chip_writes
                .into_iter()
                .map(|event| {
                    let (chip, port, value) = match event.write {
                        ReferenceWrite::Ym2612 { port, value } => (0, port, value),
                        ReferenceWrite::Psg { value } => (1, 0, value),
                    };
                    ChipWrite {
                        master_clock: event.master_clock,
                        origin: match event.origin {
                            Origin::M68k => 0,
                            Origin::Z80 => 1,
                        },
                        pc: event.pc,
                        chip,
                        port,
                        value,
                    }
                })
                .collect(),
        };
        let bytes = encode(&frame)?;
        writer.write_all(&u32::try_from(bytes.len())?.to_le_bytes())?;
        writer.write_all(&bytes)?;
        writer.flush()?;
        Ok(())
    }

    let args: Vec<_> = std::env::args_os().skip(1).collect();
    ensure!(
        args.len() == 2,
        "usage: replay-reference PRIVATE_ROM CONTROLLER_SESSION"
    );
    let rom = Rom::import(&std::fs::read(&args[0]).context("reading private reference ROM")?)?;
    let session = Session::from_json(
        &std::fs::read_to_string(&args[1]).context("reading controller recording")?,
    )?;
    ensure!(
        session.revision == rom.revision().into(),
        "session/ROM revision differs"
    );
    ensure!(
        matches!(session.start, Start::PowerOn),
        "web replay accepts power-on controller recordings, not RAM recipes"
    );
    ensure!(
        session.inputs().all(|pads| pads[0] & CONNECTED != 0),
        "the web frontend requires player one connected"
    );
    let mut reference = ReferenceMachine::new(&rom);
    let mut writer = BufWriter::new(std::io::stdout().lock());
    publish(&mut writer, 0, reference.capture()?)?;
    let mut steps = 0_u64;
    let mut instructions = 0_u64;
    for (frame, pads) in session.inputs().enumerate() {
        reference.begin_frame();
        loop {
            let step = reference.step(pads)?;
            steps += 1;
            instructions += u64::from(step.instruction_pc.is_some());
            if step.frame_complete {
                break;
            }
        }
        publish(&mut writer, frame as u64 + 1, reference.capture()?)?;
    }
    eprintln!(
        "{}",
        serde_json::json!({
            "reference": "vendored-jgenesis-portable-scalar", "revision": rom.revision().name(),
            "frames": session.frames, "schedulerSteps": steps, "instructions": instructions,
        })
    );
    Ok(())
}
