//! Stateless design-system primitives, parameterized only by view data.
use crate::model::{PersonVm, RoleVm};
use leptos::prelude::*;

pub fn icon(name: &str) -> AnyView {
    view! { <svg class="icon" aria-hidden="true" viewBox="0 0 24 24"><use href=format!("#i-{name}")></use></svg> }.into_any()
}

pub fn icon_button(name: &str, label: &str, action: &str) -> AnyView {
    view! { <button class="icon-button" type="button" aria-label=label.to_owned() data-tooltip=label.to_owned() data-action=action.to_owned()>{icon(name)}</button> }.into_any()
}

pub fn button(label: &str, action: &str, variant: &str) -> AnyView {
    view! { <button type="button" class=format!("button {variant}") data-action=action.to_owned()>{label.to_owned()}</button> }.into_any()
}

pub fn input(label: &str, value: &str, kind: &str, help: &str) -> AnyView {
    let id = label.to_lowercase().replace(' ', "-");
    view! { <div class="field"><label for=id.clone()>{label.to_owned()}</label><input id=id type=kind.to_owned() value=value.to_owned() autocomplete=match kind {"password"=>"current-password","email"=>"email",_=>"off"} /><small>{help.to_owned()}</small></div> }.into_any()
}

pub fn search(placeholder: &str, target: &str) -> AnyView {
    view! { <label class="search"><span class="sr-only">{placeholder.to_owned()}</span><input type="search" placeholder=placeholder.to_owned() data-filter=target.to_owned() />{icon("search")}</label> }.into_any()
}

pub fn avatar(person: &PersonVm, size: &str) -> AnyView {
    view! { <span class=format!("avatar {size}") style=format!("--avatar-color:{}",person.color) aria-label=format!("{}, {}",person.name,person.presence)><span class="avatar-face" aria-hidden="true">{person.initials.clone()}</span><span class=format!("presence {}",person.presence)></span></span> }.into_any()
}

pub fn person_button(person: &PersonVm, index: usize, size: &str) -> AnyView {
    view! { <button class="avatar-button" aria-label=format!("{}: {}'s profile",person.initials,person.name) data-action=format!("profile:{index}")>{avatar(person,size)}</button> }.into_any()
}

pub fn badge(text: &str, kind: &str) -> AnyView {
    view! { <span class=format!("badge {kind}")>{text.to_owned()}</span> }.into_any()
}

pub fn role_chip(role: &RoleVm) -> AnyView {
    view! { <span class="role-chip"><span class="role-dot" style=format!("background:{}",role.color)></span>{role.name.clone()}</span> }.into_any()
}

pub fn toggle(label: &str, description: &str, checked: bool, action: &str) -> AnyView {
    view! { <button class="toggle-row" role="switch" aria-checked=checked.to_string() data-action=action.to_owned()><span><strong>{label.to_owned()}</strong><small>{description.to_owned()}</small></span><span class=if checked {"toggle on"}else{"toggle"}><span>{if checked{icon("check")}else{().into_any()}}</span></span></button> }.into_any()
}

pub fn tabs(labels: &[&str], selected: &str, action: &str) -> AnyView {
    view! { <div class="tabs" role="tablist">{labels.iter().map(|label| view! {<button role="tab" aria-selected=(*label==selected).to_string() class=if *label==selected{"tab active"}else{"tab"} data-action=format!("{action}:{label}")>{label.to_string()}</button>}).collect_view()}</div> }.into_any()
}

pub fn menu_item(label: &str, glyph: &str, action: &str, danger: bool) -> AnyView {
    view! { <button role="menuitem" class=if danger {"menu-item danger-text"}else{"menu-item"} data-action=action.to_owned()><span>{label.to_owned()}</span>{icon(glyph)}</button> }.into_any()
}

pub fn empty(glyph: &str, title: &str, label: &str, action: &str) -> AnyView {
    view! { <div class="empty-state">{icon(glyph)}<h2>{title.to_owned()}</h2>{button(label,action,"primary")}</div> }.into_any()
}
