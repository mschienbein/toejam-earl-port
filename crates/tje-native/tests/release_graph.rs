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
            "ram-dma-observer",
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
        }
        assert!(graph.contains("tje-engine-runtime"));
        assert!(graph.contains("tje-native-platform"));
    }
}
