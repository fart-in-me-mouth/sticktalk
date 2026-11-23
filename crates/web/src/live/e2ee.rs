//! End-to-end encrypted calls (`docs/E2EE.md`): whether this browser can take
//! part, said to the server just before `Join`. Everything else runs in
//! `public/e2ee.js` and its worker, loaded only once a call starts.
use sticktalk_proto::*;
use wasm_bindgen::JsValue;

fn get(o: &JsValue, key: &str) -> Option<JsValue> {
    js_sys::Reflect::get(o, &JsValue::from_str(key))
        .ok()
        .filter(|v| !v.is_undefined() && !v.is_null())
}

/// WebRTC encoded transforms (or Chromium's older encoded streams), WebCrypto
/// and workers. Without them the client is refused from encrypted calls.
pub(super) fn capable() -> bool {
    let g = js_sys::global();
    let script = get(&g, "RTCRtpScriptTransform").is_some();
    let legacy = get(&g, "RTCRtpSender")
        .and_then(|s| get(&s, "prototype"))
        .and_then(|p| get(&p, "createEncodedStreams"))
        .is_some();
    let subtle = get(&g, "crypto").and_then(|c| get(&c, "subtle")).is_some();
    (script || legacy) && subtle && get(&g, "Worker").is_some()
}

/// Sent before every `Join` from a capable browser.
pub(super) fn hello() -> Option<VoiceSignal> {
    capable().then_some(VoiceSignal::E2ee(E2eeSignal::Hello {
        version: E2EE_VERSION,
    }))
}
