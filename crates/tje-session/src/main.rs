mod adventure;
mod bot;
mod navigation;
mod runner;

use anyhow::{Result, bail, ensure};
use clap::{Parser, Subcommand, ValueEnum};
use std::{fs, path::PathBuf};
use tje_host::Mode;
use tje_session::{CONNECTED, Run, Session, Start};

#[derive(Parser)]
#[command(
    version,
    about = "Input-only ToeJam & Earl sessions: replay, compare, coverage and a RAM-driven bot"
)]
struct Args {
    #[command(subcommand)]
    command: Command,
}
#[derive(Clone, Copy, ValueEnum)]
enum Boundary {
    Frame,
    Tick,
}
#[derive(Subcommand)]
enum Command {
    /// Replay sessions with the original, checked translations, or replacements.
    Replay {
        #[arg(long)]
        rom: PathBuf,
        #[arg(long, required = true, num_args = 1..)]
        session: Vec<PathBuf>,
        #[arg(long, value_enum, default_value = "reference")]
        mode: Mode,
        #[arg(long)]
        report: PathBuf,
    },
    /// Compare full video frames or completed logic ticks; report first divergence.
    Compare {
        #[arg(long)]
        rom: PathBuf,
        #[arg(long)]
        session: PathBuf,
        #[arg(long, value_enum, default_value = "replace")]
        second: Mode,
        #[arg(long, value_enum, default_value = "tick")]
        boundary: Boundary,
        #[arg(long)]
        report: PathBuf,
    },
    /// Combine report metadata/counts only, with separate content by revision.
    Union {
        #[arg(long, required = true, num_args = 1..)]
        reports: Vec<PathBuf>,
        #[arg(long)]
        out: PathBuf,
    },
    /// Produce a deterministic input-only session by observing the reference RAM.
    Bot {
        #[arg(long)]
        rom: PathBuf,
        #[arg(long, value_enum)]
        scenario: bot::Scenario,
        #[arg(long, default_value_t = 12000)]
        frames: u64,
        /// Replay this input-only prefix from power-on before choosing new inputs.
        #[arg(long)]
        prefix: Option<PathBuf>,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        report: PathBuf,
    },
    /// Export constant-connectivity sessions for the older tje-host hold-script API.
    ExportLegacy {
        #[arg(long)]
        session: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
}
fn main() -> Result<()> {
    match Args::parse().command {
        Command::Replay {
            rom,
            session,
            mode,
            report,
        } => {
            let bytes = fs::read(rom)?;
            let sessions = session
                .iter()
                .map(|path| runner::replay(&bytes, path, mode))
                .collect::<Result<Vec<_>>>()?;
            for session in &sessions {
                println!(
                    "{} revision={} frames={} entries={}/{} ticks={} lag={} checked={} replaced={} mismatches={}",
                    session.session,
                    session.revision,
                    session.frames,
                    session.entries_hit,
                    session.inventory_entries,
                    session.logic_ticks,
                    session.lag_frames.len(),
                    session.checked_calls,
                    session.replaced_calls,
                    session.mismatches
                );
            }
            let report_data = runner::Report::from_sessions(sessions);
            println!("union={}/648", report_data.union_entries_hit);
            runner::save_json(&report, &report_data)?;
        }
        Command::Compare {
            rom,
            session,
            second,
            boundary,
            report,
        } => {
            let result = runner::compare(
                &fs::read(rom)?,
                &session,
                second,
                matches!(boundary, Boundary::Tick),
            )?;
            runner::save_json(&report, &result)?;
            println!("{}", serde_json::to_string(&result)?);
            if result.divergence.is_some() {
                bail!("first divergence recorded in {}", report.display());
            }
            ensure!(
                result.compared > 0,
                "session reached no comparable boundary"
            );
        }
        Command::Union { reports, out } => {
            let mut sessions = Vec::new();
            for path in reports {
                let report: runner::Report = serde_json::from_str(&fs::read_to_string(path)?)?;
                sessions.extend(report.sessions);
            }
            let report = runner::Report::from_sessions(sessions);
            println!(
                "sessions={} union={}/648",
                report.sessions.len(),
                report.union_entries_hit
            );
            runner::save_json(&out, &report)?;
        }
        Command::Bot {
            rom,
            scenario,
            frames,
            prefix,
            out,
            report,
        } => {
            let result = bot::generate(&fs::read(rom)?, scenario, frames, &out, prefix.as_deref())?;
            println!(
                "{} frames={} entries={}/648 content={}",
                out.display(),
                result.frames,
                result.entries_hit,
                serde_json::to_string(&result.content)?
            );
            runner::save_json(&report, &runner::Report::from_sessions(vec![result]))?;
        }
        Command::ExportLegacy { session, out } => {
            let session = Session::from_json(&fs::read_to_string(session)?)?;
            ensure!(
                matches!(session.start, Start::PowerOn),
                "legacy holds cannot represent setup RAM writes"
            );
            let connectivity = session
                .runs
                .first()
                .map(|run| [run.1 & CONNECTED, run.2 & CONNECTED]);
            ensure!(
                connectivity.is_some_and(|pads| pads[0] == CONNECTED),
                "legacy API requires connected P1"
            );
            let mut frame = 0;
            let mut holds = Vec::new();
            const BUTTONS: [&str; 8] = ["up", "down", "left", "right", "a", "b", "c", "start"];
            for Run(frames, p1, p2) in session.runs {
                ensure!(
                    Some([p1 & CONNECTED, p2 & CONNECTED]) == connectivity,
                    "legacy API cannot represent connection changes"
                );
                if p1 & 255 != 0 || p2 & 255 != 0 {
                    let buttons = |pad: u16| {
                        BUTTONS
                            .iter()
                            .enumerate()
                            .filter(|(index, _)| pad & (1 << index) != 0)
                            .map(|(_, name)| *name)
                            .collect::<Vec<_>>()
                    };
                    holds.push(serde_json::json!({"start":frame,"end":frame+frames,"p1":buttons(p1),"p2":buttons(p2)}));
                }
                frame += frames;
            }
            runner::save_json(
                &out,
                &serde_json::json!({"p2_connected": connectivity.unwrap()[1] != 0, "holds":holds}),
            )?;
        }
    }
    Ok(())
}
