//! Bindings to the few browser helpers in `bridge.js`.

use wasm_bindgen::prelude::*;

#[wasm_bindgen(module = "/bridge.js")]
extern "C" {
    #[wasm_bindgen(js_name = passkeysSupported)]
    pub fn passkeys_supported() -> bool;
    #[wasm_bindgen(js_name = passkeyCreate, catch)]
    pub async fn passkey_create(options: &str) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = passkeyGet, catch)]
    pub async fn passkey_get(options: &str) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = setAwakeWanted)]
    pub fn set_awake_wanted(on: bool);
    #[wasm_bindgen(js_name = registerWorker)]
    pub fn register_worker();
    #[wasm_bindgen(js_name = reducedMotion)]
    pub fn reduced_motion() -> bool;
    #[wasm_bindgen(js_name = flipFirst)]
    pub fn flip_first(selector: &str);
    #[wasm_bindgen(js_name = flipPlay)]
    pub fn flip_play(selector: &str, duration: f64, stagger: f64);
    #[wasm_bindgen(js_name = copyText)]
    pub async fn copy_text(text: &str) -> JsValue;
    #[wasm_bindgen(js_name = unlockAudio)]
    pub fn unlock_audio();
    #[wasm_bindgen(js_name = chime)]
    pub fn chime();
    #[wasm_bindgen(js_name = timezoneOffset)]
    pub fn timezone_offset() -> i32;
    #[wasm_bindgen(js_name = hideBoot)]
    pub fn hide_boot();
    #[wasm_bindgen(js_name = openExternal)]
    pub fn open_external(url: &str);
    #[wasm_bindgen(js_name = downloadUrl)]
    pub fn download_url(url: &str);
}

/// The error text from a rejected passkey call, in words a person can act on.
pub fn passkey_error(e: &JsValue) -> String {
    let name = js_sys::Reflect::get(e, &JsValue::from_str("name")).ok().and_then(|v| v.as_string()).unwrap_or_default();
    match name.as_str() {
        "NotAllowedError" => "The passkey request was cancelled or timed out".into(),
        "InvalidStateError" => "This device already has a passkey for the app".into(),
        "SecurityError" => "Passkeys need the app's real address over HTTPS".into(),
        _ => "The passkey could not be used on this device".into(),
    }
}
