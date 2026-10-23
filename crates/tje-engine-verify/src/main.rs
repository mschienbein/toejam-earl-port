use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Instant,
};
use tje_engine_verify::{
    InstructionMachine, ReplayReport, RuntimeMachine, ir_backend::IrBackend,
    reference::ReferenceMachine, verify_instruction_session_with, verify_session_with,
};
use tje_rom::{Revision, Rom};
use tje_session::Session;

#[derive(Parser)]
#[command(about = "Exact frame/session engine verification; private ROMs required")]
struct Args {
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    /// One complete committed session; writes a receipt before returning failure.
    Replay {
        #[arg(long)]
        rom: PathBuf,
        #[arg(long)]
        session: PathBuf,
        #[arg(long, value_enum, default_value = "ir")]
        backend: Backend,
        #[arg(long)]
        cache_dir: Option<PathBuf>,
        #[arg(long)]
        report: PathBuf,
        /// Compare full CPU state and all 68000 bus transactions each instruction.
        #[arg(long)]
        instruction_proof: bool,
    },
    /// All committed sessions, both revisions, independent reference vs reference.
    ReferenceSanity(SuiteArgs),
    /// Release acceptance: every session, both revisions, IR, native and browser
    /// Wasm, plus reference sanity, all with mandatory per-instruction proof.
    Accept {
        #[command(flatten)]
        suite: SuiteArgs,
        /// Executable browser proof wrapper. See --wasm-arg for prefix arguments.
        #[arg(long)]
        wasm_command: PathBuf,
        #[arg(long)]
        wasm_arg: Vec<OsString>,
        #[arg(long)]
        cache_dir: Option<PathBuf>,
    },
}

#[derive(clap::Args)]
struct SuiteArgs {
    #[arg(long)]
    rom_rev00: PathBuf,
    #[arg(long)]
    rom_rev02: PathBuf,
    #[arg(long, default_value = "sessions")]
    sessions: PathBuf,
    #[arg(long)]
    report: PathBuf,
    /// Optional for reference-sanity; accept always enables instruction proof.
    #[arg(long)]
    instruction_proof: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Backend {
    Reference,
    Ir,
    Native,
}

#[derive(Debug, Serialize, Deserialize)]
struct NativeStats {
    instruction_count: usize,
    code_bytes: usize,
    cache_bytes: u64,
    compilation_seconds: f64,
    startup_seconds: f64,
    cache_status: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct CaseReport {
    session: String,
    backend: String,
    wall_seconds: f64,
    native: Option<NativeStats>,
    replay: Option<ReplayReport>,
    error: Option<String>,
}
impl CaseReport {
    fn passed(&self) -> bool {
        self.error.is_none() && self.replay.as_ref().is_some_and(ReplayReport::passed)
    }
}

#[derive(Serialize)]
struct SuiteReport {
    format: u32,
    /// Only the full native+Wasm matrix can be a release acceptance receipt.
    release_acceptance: bool,
    passed: bool,
    cases: Vec<CaseReport>,
}

fn main() -> Result<()> {
    match Args::parse().command {
        Action::Replay {
            rom,
            session,
            backend,
            cache_dir,
            report,
            instruction_proof,
        } => {
            let rom = import(&rom)?;
            let recording = load_session(&session)?;
            ensure_revision(&rom, &recording)?;
            let case = run_case(
                &session,
                &recording,
                &rom,
                backend,
                cache_dir.as_deref(),
                instruction_proof,
            );
            let passed = case.passed();
            save(&report, &case)?;
            ensure!(passed, "replay failed; see {}", report.display());
        }
        Action::ReferenceSanity(suite) => {
            run_suite(suite, None, None)?;
        }
        Action::Accept {
            suite,
            wasm_command,
            wasm_arg,
            cache_dir,
        } => {
            run_suite(suite, Some((wasm_command, wasm_arg)), cache_dir.as_deref())?;
        }
    }
    Ok(())
}

fn import(path: &Path) -> Result<Rom> {
    Rom::import(&fs::read(path).with_context(|| format!("reading ROM {}", path.display()))?)
        .with_context(|| format!("importing ROM {}", path.display()))
}
fn load_session(path: &Path) -> Result<Session> {
    Session::from_json(&fs::read_to_string(path)?)
        .with_context(|| format!("reading session {}", path.display()))
}
fn ensure_revision(rom: &Rom, session: &Session) -> Result<()> {
    ensure!(
        rom.revision() == Revision::from(session.revision),
        "session revision does not match imported {} ROM",
        rom.revision()
    );
    Ok(())
}
fn save(path: &Path, value: &impl Serialize) -> Result<()> {
    if let Some(parent) = path.parent().filter(|path| !path.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

fn verify_selected_session<R: InstructionMachine, C: InstructionMachine>(
    session: &Session,
    factory: impl FnMut() -> Result<(R, C)>,
    instruction_proof: bool,
) -> Result<ReplayReport> {
    if instruction_proof {
        verify_instruction_session_with(session, factory)
    } else {
        verify_session_with(session, factory)
    }
}

fn run_case(
    path: &Path,
    session: &Session,
    rom: &Rom,
    backend: Backend,
    cache_dir: Option<&Path>,
    instruction_proof: bool,
) -> CaseReport {
    let start = Instant::now();
    let mut native = None;
    let replay = match backend {
        Backend::Reference => verify_selected_session(
            session,
            || Ok((ReferenceMachine::new(rom), ReferenceMachine::new(rom))),
            instruction_proof,
        ),
        Backend::Ir => verify_selected_session(
            session,
            || {
                Ok((
                    ReferenceMachine::new(rom),
                    RuntimeMachine::new(rom, IrBackend::new(rom), "ir"),
                ))
            },
            instruction_proof,
        ),
        Backend::Native => native_replay(rom, session, cache_dir, &mut native, instruction_proof),
    };
    let (replay, error) = match replay {
        Ok(report) => (Some(report), None),
        Err(error) => (None, Some(format!("{error:#}"))),
    };
    CaseReport {
        session: path.display().to_string(),
        backend: match backend {
            Backend::Reference => "reference",
            Backend::Ir => "ir",
            Backend::Native => "native",
        }
        .into(),
        wall_seconds: start.elapsed().as_secs_f64(),
        native,
        replay,
        error,
    }
}

#[cfg(all(feature = "native", not(target_arch = "wasm32")))]
fn native_replay(
    rom: &Rom,
    session: &Session,
    cache_dir: Option<&Path>,
    stats: &mut Option<NativeStats>,
    instruction_proof: bool,
) -> Result<ReplayReport> {
    use tje_engine_cranelift::{CacheConfig, NativeProgram};
    let metadata = tje_engine_core::metadata::game_metadata(rom.revision());
    let program = tje_engine_core::metadata::compile_program(rom.bytes(), &metadata)?;
    let mut sha1 = [0; 20];
    for (index, byte) in sha1.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&rom.revision().sha1()[2 * index..2 * index + 2], 16)?;
    }
    verify_selected_session(
        session,
        || {
            let backend = if let Some(data_dir) = cache_dir {
                NativeProgram::load_or_compile(
                    sha1,
                    &program,
                    &CacheConfig {
                        data_dir: data_dir.into(),
                    },
                )?
            } else {
                NativeProgram::compile(sha1, &program)?
            };
            if stats.is_none() {
                let observed = backend.stats();
                *stats = Some(NativeStats {
                    instruction_count: observed.instruction_count,
                    code_bytes: observed.code_bytes,
                    cache_bytes: observed.cache_bytes,
                    compilation_seconds: observed.compilation_time.as_secs_f64(),
                    startup_seconds: observed.startup_time.as_secs_f64(),
                    cache_status: format!("{:?}", observed.cache_status),
                });
            }
            Ok((
                ReferenceMachine::new(rom),
                RuntimeMachine::new(rom, backend, "native"),
            ))
        },
        instruction_proof,
    )
}

#[cfg(not(all(feature = "native", not(target_arch = "wasm32"))))]
fn native_replay(
    _: &Rom,
    _: &Session,
    _: Option<&Path>,
    _: &mut Option<NativeStats>,
    _: bool,
) -> Result<ReplayReport> {
    anyhow::bail!("native verification requires a native target and cargo --features native");
}

fn session_paths(directory: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_type()?.is_file()
            && entry.file_name().to_string_lossy().ends_with(".tje.json")
        {
            paths.push(entry.path());
        }
    }
    paths.sort();
    ensure!(
        !paths.is_empty(),
        "no committed-format sessions in {}",
        directory.display()
    );
    Ok(paths)
}

fn run_suite(
    args: SuiteArgs,
    wasm: Option<(PathBuf, Vec<OsString>)>,
    cache: Option<&Path>,
) -> Result<()> {
    let roms = [import(&args.rom_rev00)?, import(&args.rom_rev02)?];
    ensure!(
        roms[0].revision() == Revision::Rev00,
        "--rom-rev00 is not REV 00"
    );
    ensure!(
        roms[1].revision() == Revision::Rev02,
        "--rom-rev02 is not REV 02"
    );
    let sessions = session_paths(&args.sessions)?
        .into_iter()
        .map(|path| {
            let session = load_session(&path)?;
            ensure!(
                session.frames > 0,
                "empty acceptance session {}",
                path.display()
            );
            Ok((path, session))
        })
        .collect::<Result<Vec<_>>>()?;
    for revision in Revision::ALL {
        ensure!(
            sessions
                .iter()
                .any(|(_, session)| Revision::from(session.revision) == revision),
            "acceptance requires sessions for {revision}"
        );
    }
    let mut report = SuiteReport {
        format: 1,
        release_acceptance: wasm.is_some(),
        passed: false,
        cases: Vec::new(),
    };
    for (path, session) in sessions {
        let index = usize::from(Revision::from(session.revision) == Revision::Rev02);
        let rom = &roms[index];
        let instruction_proof = args.instruction_proof || wasm.is_some();
        report.cases.push(run_case(
            &path,
            &session,
            rom,
            Backend::Reference,
            cache,
            instruction_proof,
        ));
        if let Some((command, arguments)) = &wasm {
            report
                .cases
                .push(run_case(&path, &session, rom, Backend::Ir, cache, true));
            report
                .cases
                .push(run_case(&path, &session, rom, Backend::Native, cache, true));
            let rom_path = if index == 0 {
                &args.rom_rev00
            } else {
                &args.rom_rev02
            };
            report
                .cases
                .push(browser_case(command, arguments, rom_path, &path, &session));
        }
        // Retain receipts even when a later session or backend fails.
        save(&args.report, &report)?;
    }
    report.passed = report.cases.iter().all(CaseReport::passed);
    save(&args.report, &report)?;
    ensure!(
        report.passed,
        "acceptance failed; see {}",
        args.report.display()
    );
    Ok(())
}

fn browser_case(
    command: &Path,
    arguments: &[OsString],
    rom: &Path,
    path: &Path,
    session: &Session,
) -> CaseReport {
    let start = Instant::now();
    let result = (|| -> Result<ReplayReport> {
        // A fresh create_new file prevents an old successful browser receipt
        // from being accepted if the command fails to produce a new report.
        let mut nonce = 0;
        let (report_path, file) = loop {
            let path = std::env::temp_dir().join(format!(
                "tje-wasm-proof-{}-{nonce}.json",
                std::process::id()
            ));
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(file) => break (path, file),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => nonce += 1,
                Err(error) => return Err(error.into()),
            }
        };
        drop(file);
        let result = (|| {
            let status = Command::new(command)
                .args(arguments)
                .arg("--rom")
                .arg(rom)
                .arg("--session")
                .arg(path)
                .arg("--report")
                .arg(&report_path)
                .status()
                .with_context(|| format!("launching Wasm proof {}", command.display()))?;
            let replay: ReplayReport = serde_json::from_slice(&fs::read(&report_path)?)
                .context("browser wrapper did not write a valid fresh ReplayReport")?;
            ensure!(
                replay.format == tje_engine_verify::REPORT_FORMAT,
                "browser report format mismatch"
            );
            ensure!(
                replay.candidate_backend == "wasm" && replay.reference_backend == "reference",
                "browser report did not compare the Wasm backend against the reference"
            );
            ensure!(
                replay.requested_frames == session.frames
                    && replay.revision == Revision::from(session.revision).name(),
                "browser session/revision mismatch"
            );
            ensure!(
                replay.instruction_proof.is_some(),
                "browser acceptance requires per-instruction CPU, bus and cycle proof"
            );
            ensure!(
                status.success() || !replay.passed(),
                "browser exited with failure despite a passing report"
            );
            Ok(replay)
        })();
        let cleanup = fs::remove_file(report_path);
        let replay = result?;
        cleanup?;
        Ok(replay)
    })();
    let (replay, error) = match result {
        Ok(replay) => (Some(replay), None),
        Err(error) => (None, Some(format!("{error:#}"))),
    };
    CaseReport {
        session: path.display().to_string(),
        backend: "wasm".into(),
        wall_seconds: start.elapsed().as_secs_f64(),
        native: None,
        replay,
        error,
    }
}
