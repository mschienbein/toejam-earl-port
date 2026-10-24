//! Actual windowed crash/replay acceptance. Opt in with a private ROM and display.
use std::{collections::HashSet, fs, path::PathBuf, process::Command};
use tje_session::report::Bundle;

#[test]
#[ignore = "requires TJE_REV02_ROM and a real native display/audio environment"]
fn forced_crash_is_rom_free_and_replays_to_the_captured_state() {
    if !cfg!(debug_assertions) {
        return;
    } // Release builds intentionally have no crash trigger.
    let rom = PathBuf::from(std::env::var_os("TJE_REV02_ROM").expect("private TJE_REV02_ROM"));
    let directory = std::env::temp_dir().join(format!("tje-forced-crash-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let binary = env!("CARGO_BIN_EXE_toejam-earl");
    let crash = Command::new(binary)
        .args(["--rom"])
        .arg(&rom)
        .arg("--data-dir")
        .arg(&directory)
        .args(["--gpu-backend", "vulkan", "--run-frames", "121"])
        .env("TJE_DEBUG_CRASH_FRAME", "120")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .output()
        .unwrap();
    assert!(!crash.status.success(), "debug trigger did not panic");
    let name = fs::read_to_string(directory.join("reports/pending-crash.txt")).unwrap();
    let path = directory.join("reports").join(name);
    let bytes = fs::read(&path).unwrap();
    let bundle = Bundle::from_json(std::str::from_utf8(&bytes).unwrap()).unwrap();
    assert_eq!(bundle.failure.source, "rust_panic");
    assert_eq!(bundle.engine.as_ref().unwrap().frame, 120);
    assert_eq!(
        bundle
            .recording
            .as_ref()
            .unwrap()
            .replay_session()
            .unwrap()
            .frames,
        120
    );
    assert!(
        bundle.screenshot.is_none(),
        "screenshots require prior consent"
    );
    // Scan every exact 32-byte ROM window, not merely the full-ROM hash. The
    // compact bundle contains no whitespace padding that needs an exemption.
    let image = fs::read(rom).unwrap();
    let windows: HashSet<&[u8]> = image.windows(32).collect();
    assert!(
        !bytes.windows(32).any(|window| windows.contains(window)),
        "bundle contains a raw ROM fragment"
    );
    let replay = Command::new(binary)
        .arg("--data-dir")
        .arg(&directory)
        .arg("--replay-report")
        .arg(&path)
        .args(["--gpu-backend", "vulkan"])
        .env_remove("TJE_DEBUG_CRASH_FRAME")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .output()
        .unwrap();
    assert!(
        replay.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&replay.stdout),
        String::from_utf8_lossy(&replay.stderr)
    );
    assert!(String::from_utf8_lossy(&replay.stdout).contains("REPORT_REPLAY_MATCH frame=120"));
    fs::remove_dir_all(directory).unwrap();
}
