mod components;
mod model;
mod views;

#[cfg(target_arch = "wasm32")]
use leptos::prelude::*;

#[cfg(target_arch = "wasm32")]
fn main() {
    use wasm_bindgen::{closure::Closure, JsCast};
    if let Some(boot) = document().get_element_by_id("boot") {
        boot.remove();
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
            <style>{include_str!(concat!(env!("OUT_DIR"), "/tokens.css"))}</style>
            {move || views::app(state.get())}
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    // Native builds exercise the pure model and render functions, never open a window.
    let mut vm = model::AppVm::fixture();
    vm.reduce("");
    let _ = views::app(vm);
}
