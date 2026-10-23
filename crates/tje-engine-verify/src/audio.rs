//! ROM-free production-device audio inputs, shared by native CI and Chromium.
//! Golden hashes cover both sample bits and ordering, never cartridge output.

use anyhow::{Result, ensure};
use genesis_config::GenesisEmulatorConfig;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tje_mem::AddressSpace;
use tje_native_platform::{ClockedDevices, DeviceRuntime};
use tje_rom::Revision;

#[derive(Debug, Clone, Serialize)]
pub struct AudioHash {
    pub scenario: &'static str,
    pub sample_rate: u32,
    pub samples: usize,
    pub master_clocks: u64,
    pub sha256: String,
}

#[derive(Clone, Copy)]
enum Scenario {
    Psg,
    Ym2612Dac,
}

impl Scenario {
    fn name(self) -> &'static str {
        match self {
            Self::Psg => "psg-tone-noise",
            Self::Ym2612Dac => "ym2612-dac",
        }
    }
}

fn write(devices: &mut DeviceRuntime, address: u32, value: u16, word: bool) {
    loop {
        devices.begin_instruction(0x4E71, 56);
        if devices.take_wait_step(0x200).is_none() {
            break;
        }
    }
    if word {
        devices.write_bus_word(address, value);
    } else {
        devices.write_bus_byte(address, value as u8);
    }
    devices.finish_instruction(0x200, 4);
}

fn psg(devices: &mut DeviceRuntime, value: u8) {
    write(devices, 0xC00011, value.into(), false);
}

fn ym(devices: &mut DeviceRuntime, bank: u32, register: u8, value: u8) {
    write(devices, 0xA04000 + 2 * bank, register.into(), false);
    write(devices, 0xA04001 + 2 * bank, value.into(), false);
    // Authored register-input spacing; the real chip advances throughout and
    // the next write is later than its busy window, not a fabricated ready bit.
    devices.advance_master_clocks(2048);
}

fn render(scenario: Scenario, sample_rate: u32) -> Result<AudioHash> {
    let mut devices = DeviceRuntime::new(
        AddressSpace::new(vec![0; 1024], Revision::Rev02),
        &GenesisEmulatorConfig::default(),
    );
    devices.set_audio_sample_rate(sample_rate);
    // Hold the authored zero-filled Z80 memory off the bus while programming YM.
    write(&mut devices, 0xA11100, 0x100, true);
    write(&mut devices, 0xA11200, 0x100, true);
    match scenario {
        Scenario::Psg => {
            for value in [0x80, 0x10, 0x90] {
                psg(&mut devices, value);
            }
        }
        Scenario::Ym2612Dac => {
            for value in [0x9F, 0xBF, 0xDF, 0xFF] {
                psg(&mut devices, value);
            }
            ym(&mut devices, 1, 0xB6, 0xC0);
            ym(&mut devices, 0, 0x2B, 0x80);
        }
    }
    for phase in 0_u32..64 {
        match scenario {
            Scenario::Psg => match phase {
                16 => psg(&mut devices, 0x95),
                32 => {
                    psg(&mut devices, 0xE4);
                    psg(&mut devices, 0xF5);
                }
                48 => psg(&mut devices, 0x90),
                _ => {}
            },
            Scenario::Ym2612Dac => {
                ym(
                    &mut devices,
                    0,
                    0x2A,
                    phase.wrapping_mul(73).wrapping_add(19) as u8,
                );
            }
        }
        devices.advance_master_clocks(3420 * 4);
    }
    devices.advance_master_clocks(3420 * 262 * 9);
    let samples = devices.audio_samples();
    ensure!(
        samples.len() > sample_rate as usize / 10,
        "empty synthetic audio window"
    );
    ensure!(
        samples.iter().flatten().all(|sample| sample.is_finite()),
        "nonfinite chip audio"
    );
    ensure!(
        samples.iter().flatten().any(|sample| sample.abs() > 0.0001),
        "silent chip fixture"
    );
    let mut hash = Sha256::new();
    hash.update(b"tje-portable-scalar-audio-v1");
    hash.update(scenario.name());
    hash.update(sample_rate.to_le_bytes());
    hash.update((samples.len() as u64).to_le_bytes());
    for sample in samples {
        for channel in sample {
            hash.update(channel.to_bits().to_le_bytes());
        }
    }
    Ok(AudioHash {
        scenario: scenario.name(),
        sample_rate,
        samples: samples.len(),
        master_clocks: devices.clock(),
        sha256: format!("{:x}", hash.finalize()),
    })
}

pub fn render_audio_hashes() -> Result<Vec<AudioHash>> {
    [Scenario::Psg, Scenario::Ym2612Dac]
        .into_iter()
        .flat_map(|scenario| [44_100, 48_000].map(move |rate| (scenario, rate)))
        .map(|(scenario, rate)| render(scenario, rate))
        .collect()
}

struct ExpectedAudio {
    scenario: Scenario,
    rate: u32,
    samples: usize,
    master_clocks: u64,
    sha256: &'static str,
}

// Authored register-input goldens, measured with the canonical portable scalar
// path. These digests contain no retail-ROM or derived game-output material.
const EXPECTED: [ExpectedAudio; 4] = [
    ExpectedAudio {
        scenario: Scenario::Psg,
        rate: 44_100,
        samples: 7312,
        master_clocks: 8_940_132,
        sha256: "d70762b93e6a866a10bfd7c55f7c3270230f0cd7975bc099aba535c5c933ff3d",
    },
    ExpectedAudio {
        scenario: Scenario::Psg,
        rate: 48_000,
        samples: 7960,
        master_clocks: 8_940_132,
        sha256: "0d7b7b035a6c27de58d16971fd9b4bf9de30128593d7aec4726525115fc6d9f3",
    },
    ExpectedAudio {
        scenario: Scenario::Ym2612Dac,
        rate: 44_100,
        samples: 7426,
        master_clocks: 9_079_871,
        sha256: "397f23ebe585c3529a958b3ddb83ba18a2500952597a1992e3a790741d333c31",
    },
    ExpectedAudio {
        scenario: Scenario::Ym2612Dac,
        rate: 48_000,
        samples: 8084,
        master_clocks: 9_079_871,
        sha256: "dba1fb5830fb14bfbccf62d59c552562f80bae4deaf215f1a5f0f0296fc6fb09",
    },
];

pub fn verify_audio_hashes() -> Result<Vec<AudioHash>> {
    let hashes = render_audio_hashes()?;
    ensure!(
        hashes.len() == EXPECTED.len(),
        "missing canonical audio case"
    );
    for (actual, expected) in hashes.iter().zip(&EXPECTED) {
        ensure!(
            actual.scenario == expected.scenario.name()
                && actual.sample_rate == expected.rate
                && actual.samples == expected.samples
                && actual.master_clocks == expected.master_clocks
                && actual.sha256 == expected.sha256,
            "canonical audio {} at {} Hz: expected {} samples / {} clocks / {}, got {} samples / {} clocks / {}; all observed cases: {hashes:?}",
            expected.scenario.name(),
            expected.rate,
            expected.samples,
            expected.master_clocks,
            expected.sha256,
            actual.samples,
            actual.master_clocks,
            actual.sha256,
        );
    }
    Ok(hashes)
}

#[cfg(test)]
mod tests {
    #[test]
    fn canonical_audio_hashes_match_production_devices() {
        let hashes = super::verify_audio_hashes().unwrap();
        assert_eq!(hashes.len(), 4);
        for hash in hashes {
            println!(
                "{} {} {} {} {}",
                hash.scenario, hash.sample_rate, hash.samples, hash.master_clocks, hash.sha256
            );
        }
    }
}
