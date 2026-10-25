//! Runs in the normal Linux/Windows/Apple-ARM64 workspace CI matrix without a ROM.
//! The proof executable is a separate package, not even a frontend dev dependency.
use std::process::Command;

#[test]
fn shipping_release_graph_contains_no_68000_or_development_fallback() {
    for package in ["tje-native", "tje-web"] {
        // Native CI invokes its own Cargo normally. Cross-target tests may be run
        // through WSL interop; an optional runner can invoke the build-host Cargo
        // there without changing the graph or enabling any fallback feature.
        let mut command = if let Some(runner) = std::env::var_os("TJE_GRAPH_CARGO_RUNNER") {
            let mut command = Command::new(runner);
            command.arg(env!("CARGO"));
            command
        } else {
            Command::new(env!("CARGO"))
        };
        let output = command
            .args([
                "tree",
                "--locked",
                "--package",
                package,
                "--target",
                if package == "tje-web" {
                    // Its host-only dev oracle is not a browser dependency.
                    "wasm32-unknown-unknown"
                } else {
                    "all"
                },
                "--edges",
                "normal,build,dev",
                "--prefix",
                "none",
                "--format",
                "{p} features={f}",
            ])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("inspect the exact shipping package's default dependency graph");
        assert!(
            output.status.success(),
            "cargo tree failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let graph = String::from_utf8(output.stdout).unwrap();
        for forbidden in [
            "m68000-emu",
            "tje-host",
            "m68000-timing",
            "tje-engine-verify",
            "tje-native-proof",
            "oracle",
            "browser-proof",
            "call-window-proofs",
            "enhanced-mode",
        ] {
            assert!(
                !graph.contains(forbidden),
                "shipping graph contains {forbidden}:\n{graph}"
            );
        }
        // This feature includes the passive actual-read context journal as well
        // as reset/audio fixtures. No frontend may enable it, even by a dev edge.
        assert!(
            !graph.contains("reference-proof-hooks"),
            "shipping graph enables reference-only actual-read context/fixture hooks:\n{graph}"
        );
        // The standalone device adapter intentionally reuses genesis-core's audio,
        // controller and capture types. Its emulator module is feature-gated off;
        // checking that boundary is stronger than banning a peripheral crate name.
        for core in graph
            .lines()
            .filter(|line| line.starts_with("genesis-core "))
        {
            let (_, features) = core.split_once("features=").expect("Cargo feature listing");
            assert!(
                features.trim().trim_end_matches("(*)").trim().is_empty(),
                "the shared core must stay peripheral-only, never enable emulator/default: {core}"
            );
        }
        if package == "tje-native" {
            assert!(
                graph.contains("tje-engine-cranelift"),
                "shipping native game must use compiled native continuations"
            );
            assert!(
                graph.lines().any(|line| line.starts_with("egui-winit ")
                    && line
                        .split_once("features=")
                        .is_some_and(|(_, features)| features
                            .split(',')
                            .any(|feature| feature.split_whitespace().next() == Some("arboard")))),
                "native connection codes need an OS clipboard, not egui's per-window fallback"
            );
        }
        assert!(graph.contains("tje-engine-runtime"));
        assert!(graph.contains("tje-native-platform"));
    }
}

#[test]
#[ignore = "requires a desktop clipboard; verifies connection codes cross separate native window clipboard contexts"]
fn desktop_clipboard_roundtrip_for_connection_codes() {
    use egui_winit::clipboard::Clipboard;
    use tje_netplay::{Config, udp::Invitation};
    // SAFETY: None borrows no raw display handle. The enabled arboard backend
    // discovers the OS clipboard independently, exactly as egui-winit does.
    let mut host = unsafe { Clipboard::new(None) };
    // SAFETY: as above; a distinct instance detects accidental in-window-only
    // fallback, which cannot transfer a host code to a second game window.
    let mut join = unsafe { Clipboard::new(None) };
    let previous_text = host.get();
    let previous_image = if previous_text.is_none() {
        host.get_image()
    } else {
        None
    };
    let code = Invitation {
        address: "127.0.0.1:19791".parse().unwrap(),
        token: [17; 16],
        config: Config::default(),
        sample_rate: 48000,
    }
    .encode()
    .unwrap();
    host.set_text(code.clone());
    let received = join.get();
    if let Some(text) = previous_text {
        host.set_text(text);
    } else if let Some(image) = previous_image {
        host.set_image(&image);
    } else {
        host.set_text(String::new());
    }
    assert_eq!(
        received.as_deref(),
        Some(code.as_str()),
        "OS clipboard transfer must work across instances"
    );
}
