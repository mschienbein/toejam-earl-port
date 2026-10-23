//! Execute the same production-device audio goldens as native CI.
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn browser_verify_audio() -> Result<String, JsValue> {
    let hashes = tje_engine_verify::audio::verify_audio_hashes()
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    serde_json::to_string(&hashes).map_err(|error| JsValue::from_str(&error.to_string()))
}
