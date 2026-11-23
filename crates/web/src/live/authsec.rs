//! Data export progress for public/authsec.js (docs/features/authsec.md). The
//! server sends `DataExportUpdated` only to streams that opened the Your Data
//! page; it is handed to the page script as a DOM event, never to the model.
use sticktalk_proto::Event;
use wasm_bindgen::JsValue;

pub(super) fn event(event: &Event) {
    let Event::DataExportUpdated(export) = event else {
        return;
    };
    let Ok(detail) = serde_json::to_string(export) else {
        return;
    };
    let init = web_sys::CustomEventInit::new();
    init.set_detail(&JsValue::from_str(&detail));
    if let (Some(window), Ok(e)) = (
        web_sys::window(),
        web_sys::CustomEvent::new_with_event_init_dict("sticktalk-authsec", &init),
    ) {
        let _ = window.dispatch_event(&e);
    }
}

/// The data export page's one entry point (`features.rs`).
#[leptos::wasm_split::wasm_split(
    authsec,
    wasm_split_path = ::leptos::wasm_split,
    fallible,
    preload(prefetch)
)]
pub(super) fn run(_app: super::Shared, call: super::features::Call) -> super::features::Outcome {
    if let super::features::Call::Event(raw) = call {
        let Ok(export) = super::inbound::de(&raw.data) else {
            return Ok(false);
        };
        event(&Event::DataExportUpdated(export));
    }
    Ok(true)
}
