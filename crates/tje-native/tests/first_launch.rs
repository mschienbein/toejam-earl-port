use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use tje_engine_cranelift::{CacheConfig, CacheStatus};
use tje_native::{
    backend::{GameBackend, NativeBackend},
    config::Storage,
    input::Controllers,
};
use tje_rom::Rom;

#[test]
#[ignore = "requires the user's private ROM; compiles it locally, then proves the second launch auto-loads a cache hit"]
fn second_launch_remembers_the_cartridge_and_executes_cached_native_code() {
    let path = std::env::var_os("TJE_REV02_ROM")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("../../roms/rev02.bin"));
    let rom = Rom::import(&fs::read(path).unwrap()).unwrap();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("tje-native-launch-{}-{nonce}", std::process::id()));
    let first = Storage::new(Some(root.clone())).unwrap();
    assert!(first.startup_rom(None).is_none());
    let cache = CacheConfig {
        data_dir: first.cache_directory.clone(),
    };
    let mut backend = NativeBackend::load(rom, &cache, |_| {}).unwrap();
    assert_eq!(backend.compile_stats().cache_status, CacheStatus::Miss);
    first.store_rom(backend.rom_bytes()).unwrap();
    let pads = Controllers {
        connected: [true, false],
        ..Controllers::default()
    };
    for _ in 0..120 {
        backend.advance(pads).unwrap();
    }
    let expected_pixels = backend.video().rgba.to_vec();
    let expected_status = backend.status();
    drop(backend);

    let second = Storage::new(Some(root.clone())).unwrap();
    let (remembered, remember) = second
        .startup_rom(None)
        .expect("second launch must not prompt for a cartridge");
    assert!(!remember);
    let rom = Rom::import(&fs::read(remembered).unwrap()).unwrap();
    let mut backend = NativeBackend::load(rom, &cache, |progress| {
        assert!(
            !matches!(
                progress,
                tje_engine_cranelift::CompileProgress::Compiling { .. }
            ),
            "cache hit must bypass code generation"
        );
    })
    .unwrap();
    assert_eq!(backend.compile_stats().cache_status, CacheStatus::Hit);
    for _ in 0..120 {
        backend.advance(pads).unwrap();
    }
    assert_eq!(backend.video().rgba, expected_pixels);
    assert_eq!(backend.status().frame, expected_status.frame);
    assert_eq!(backend.status().pc, expected_status.pc);
    assert_eq!(backend.status().master_clock, expected_status.master_clock);
    second.forget_rom().unwrap();
    assert!(second.startup_rom(None).is_none());
    fs::remove_dir_all(root).unwrap();
}
