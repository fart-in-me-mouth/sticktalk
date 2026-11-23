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
