#[cfg(any(feature = "mock", not(target_arch = "wasm32")))]
mod components;
#[cfg(any(target_arch = "wasm32", test))]
mod inbound;
#[cfg(any(target_arch = "wasm32", test))]
mod kaomoji;
#[cfg(target_arch = "wasm32")]
mod live;
#[cfg(any(feature = "mock", not(target_arch = "wasm32")))]
mod model;
#[cfg(any(target_arch = "wasm32", test))]
mod nav_logic;
#[cfg(any(feature = "mock", not(target_arch = "wasm32")))]
mod views;
#[cfg(any(target_arch = "wasm32", test))]
mod wire;

#[cfg(target_arch = "wasm32")]
use leptos::prelude::*;

#[cfg(target_arch = "wasm32")]
fn main() {
    if let Some(boot) = document().get_element_by_id("boot") {
        boot.remove();
    }
    #[cfg(feature = "mock")]
    if mount_mock() {
        return;
    }
    live::mount();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    // Native builds exercise the pure model and render functions, never open a window.
    let mut vm = model::AppVm::fixture();
    vm.reduce("");
    let _ = views::app(vm);
}

#[cfg(all(target_arch = "wasm32", feature = "mock"))]
fn mount_mock() -> bool {
    use wasm_bindgen::{closure::Closure, JsCast};
    let mock = window()
        .location()
        .search()
        .unwrap_or_default()
        .trim_start_matches('?')
        .split('&')
        .any(|part| part.split('=').next() == Some("mock"));
    if !mock {
        return false;
    }
    leptos::mount::mount_to_body(|| {
        let state = RwSignal::new(model::AppVm::fixture());
        let listener = Closure::<dyn Fn(web_sys::Event)>::new(move |event: web_sys::Event| {
            if let Some(event) = event.dyn_ref::<web_sys::CustomEvent>() {
                if let Some(action) = event.detail().as_string() {
                    state.update(|vm| vm.reduce(&action));
                }
            }
        });
        window()
            .add_event_listener_with_callback("sticktalk-action", listener.as_ref().unchecked_ref())
            .expect("action listener");
        listener.forget();
        view! {
            {move || views::app(state.get())}
        }
    });
    true
}
