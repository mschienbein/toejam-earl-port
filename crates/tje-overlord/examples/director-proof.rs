//! Synthetic authoritative-state parity only; no cartridge resources or audio.
#[cfg(not(target_arch = "wasm32"))]
fn main() {
    for revision in [0, 2] {
        let hash = tje_overlord::proof::scenario(revision);
        print!("authored-rev{revision:02} ");
        for byte in hash {
            print!("{byte:02x}");
        }
        println!();
    }
}
#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn overlord_proof_word(revision: u32, index: u32) -> u32 {
    if !matches!(revision, 0 | 2) || index >= 8 {
        return 0;
    }
    let hash = tje_overlord::proof::scenario(revision as u8);
    let index = index as usize * 4;
    u32::from_le_bytes(hash[index..index + 4].try_into().expect("digest word"))
}
