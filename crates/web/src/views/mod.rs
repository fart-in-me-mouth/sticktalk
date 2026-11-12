mod overlays;
mod screens;
mod shell;

use crate::{components::*, model::*};
use leptos::prelude::*;

pub fn app(mut vm: AppVm) -> AnyView {
    // The DM fixture represents a one-to-one conversation, not server members.
    if vm.screen == "dm" {
        vm.typing.truncate(1);
        for message in &mut vm.messages {
            if message.author > 1 {
                message.author = 1;
            }
        }
    }
    let screen = match vm.screen.as_str() {
        "login" | "register" => screens::auth(&vm),
        "user-settings" | "server-settings" | "roles" | "role-editor" => screens::settings(&vm),
        "stream" => screens::stream(&vm),
        _ => shell::shell(&vm),
    };
    view! {
        <div id="sticktalk" class=if vm.keyboard {"keyboard-fixture"}else{""} data-screen=vm.screen.clone()>
            <div inner_html=include_str!("../../public/icons/sprite.svg")></div>
            {screen}
            {overlays::overlays(&vm)}
            {if vm.toast.is_empty(){().into_any()}else{view!{<div class="toast" role="status">{icon("check")}{vm.toast}</div>}.into_any()}}
            <div id="announcer" class="sr-only" aria-live="polite"></div>
        </div>
    }.into_any()
}
