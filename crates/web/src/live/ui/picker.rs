//! The expression picker: Emoji, Stickers, GIFs and Kaomoji in one dialog,
//! and Add Reaction, which picks emoji only.
//!
//! The core draws the frame (the dialog at its final size and the tabs) as
//! soon as a picker opens, so nothing moves or disappears while code arrives;
//! what is inside a tab is in one lazily loaded module (`load_picker_body`),
//! with a quiet placeholder until it is there. The behavior that needs a
//! script (arrow keys between tabs, the category bar that jumps and follows
//! the scroll, drawing more of a section as it nears) is in
//! `public/expression.js`, loaded when a picker first shows.
use super::*;

/// The picker's kinds, in the order the tabs show them: (dialog, label).
pub(super) const KINDS: [(&str, &str); 4] = [
    ("emoji", "Emoji"),
    ("stickers", "Stickers"),
    ("gif", "GIFs"),
    ("kaomoji", "Kaomoji"),
];
const REACT: &str = "emoji-react:";

/// Whether `dialog` is drawn by the picker.
pub(in crate::live) fn owns(dialog: &str) -> bool {
    dialog.starts_with(REACT) || KINDS.iter().any(|(k, _)| *k == dialog)
}

/// The tab buttons: a segmented control (shadcn Tabs), one tab stop, arrow keys
/// move between them (`expression.js`). The open tab has no action: pressing
/// it again keeps what is typed and where the list is.
fn tabs(current: &str) -> AnyView {
    view!{<div class="picker-tabs mr-10 grid flex-none grid-cols-4 gap-1 rounded-lg border bg-field p-1 touch:mr-12" role="tablist" aria-label="Expression picker">
        {KINDS.iter().map(|(kind,label)|{let on=*kind==current;view!{<button type="button" class="picker-tab inline-flex h-8 min-w-0 items-center justify-center rounded-md px-2 text-sm font-medium whitespace-nowrap text-subtle transition-colors hover:bg-accent hover:text-foreground aria-selected:bg-secondary aria-selected:font-semibold aria-selected:text-foreground aria-selected:shadow-xs aria-selected:hover:bg-secondary touch:h-11" role="tab" id=format!("picker-tab-{kind}") aria-selected=on.to_string() aria-controls="picker-panel" tabindex=if on{"0"}else{"-1"} data-live=(!on).then(||format!("dialog:{kind}"))>{*label}</button>}}).collect_view()}
    </div>}.into_any()
}

/// Until the tab's contents arrive: the search field's place, nothing else.
fn placeholder(dialog: &str) -> AnyView {
    view! {
        {(dialog!="kaomoji").then(||view!{<div class="picker-search mx-6 mt-3 h-10 flex-none rounded-md border border-input bg-field phone:mx-4 touch:h-12" aria-hidden="true"></div>})}
        <p class="sr-only" role="status">"Loading…"</p>
    }
    .into_any()
}

/// The open picker, or None when the dialog is not one.
pub(super) fn view(a: &App) -> Option<AnyView> {
    let d = a.dialog.as_str();
    if !owns(d) {
        return None;
    }
    let react = d.starts_with(REACT);
    let label = match KINDS.iter().find(|(k, _)| *k == d) {
        Some((_, label)) => label,
        None => "Add Reaction",
    };
    let body = lazy_view(a, |a| &a.views.picker, || Box::pin(load_picker_body()))
        .map_or_else(|| placeholder(d), |body| body(a));
    // One size for every tab and for the moment before the contents arrive.
    const FRAME: &str = "picker-modal flex h-[min(640px,calc(var(--viewport-height)-48px))] flex-col overflow-hidden p-0 phone:h-[min(640px,calc(var(--viewport-height)-var(--safe-top)-16px))]";
    Some(modal(
        a,
        FRAME,
        label,
        view!{
            <div class="picker-head flex-none px-6 pt-6 phone:px-4 phone:pt-4">{if react{view!{<h2 class="mb-0">"Add Reaction"</h2>}.into_any()}else{tabs(d)}}</div>
            <div class="picker-panel flex min-h-0 flex-1 flex-col" id="picker-panel" role=if react{"group"}else{"tabpanel"} aria-labelledby=(!react).then(||format!("picker-tab-{d}")) aria-label=react.then_some("Emoji")>{body}</div>
        }.into_any(),
    ))
}

#[lazy]
pub(in crate::live) fn load_picker_body() -> fn(&App) -> AnyView {
    body
}

/// The contents of the open tab (in `load_picker_body`).
fn body(a: &App) -> AnyView {
    match a.dialog.as_str() {
        "stickers" => expression::sticker_body(a),
        "gif" => content::gif_body(a),
        "kaomoji" => kaomoji(),
        d => expression::emoji_body(a, d.strip_prefix(REACT).map(str::to_owned)),
    }
}

/// The part of a tab that scrolls, under its search and category bar.
pub(super) const SCROLL: &str = "picker-scroll min-h-0 flex-1 overflow-y-auto overscroll-contain px-6 pb-6 phone:px-4 phone:pb-[calc(16px+var(--safe-bottom))]";
/// A tab's search field row.
pub(super) const SEARCH: &str = "picker-search mx-6 mt-3 flex flex-none phone:mx-4";

/// A category bar: each choice jumps to its section of the list below, and
/// the one whose section is at the top is marked (`aria-current`) as the list
/// scrolls (`expression.js`). (key, name, icon).
pub(super) fn categories(items: Vec<(String, String, AnyView)>) -> AnyView {
    view!{<nav class="picker-nav mx-6 mt-3 flex flex-none gap-1 overflow-x-auto [scrollbar-width:none] phone:mx-4" aria-label="Categories">
        {items.into_iter().enumerate().map(|(i,(key,name,glyph))|view!{<button type="button" class="picker-jump grid size-9 flex-none place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground aria-[current=true]:bg-secondary aria-[current=true]:text-primary touch:size-11" data-emoji-jump=key aria-label=name.clone() title=name aria-current=(i==0).to_string()>{glyph}</button>}).collect_view()}
    </nav>}.into_any()
}

/// Lucide icons (ISC) that only the picker uses, kept out of the page's sprite.
const GLYPHS: [(&str, &[&str]); 5] = [
    (
        "pizza",
        &[
            "M15 11h.01",
            "M11 15h.01",
            "M16 16h.01",
            "m2 16 20 6-6-20A20 20 0 0 0 2 16",
            "M5.71 17.11a17.04 17.04 0 0 1 11.4-11.4",
        ],
    ),
    (
        "trophy",
        &[
            "M6 9H4.5a2.5 2.5 0 0 1 0-5H6",
            "M18 9h1.5a2.5 2.5 0 0 0 0-5H18",
            "M4 22h16",
            "M10 14.66V17c0 .55-.47.98-.97 1.21C7.85 18.75 7 20.24 7 22",
            "M14 14.66V17c0 .55.47.98.97 1.21C16.15 18.75 17 20.24 17 22",
            "M18 2H6v7a6 6 0 0 0 12 0V2Z",
        ],
    ),
    (
        "plane",
        &["M17.8 19.2 16 11l3.5-3.5C21 6 21.5 4 21 3c-1-.5-3 0-4.5 1.5L13 8 4.8 6.2c-.5-.1-.9.1-1.1.5l-.3.5c-.2.5-.1 1 .3 1.3L9 12l-2 3H4l-1 1 3 2 2 3 1-1v-3l3-2 3.5 5.3c.3.4.8.5 1.3.3l.5-.2c.4-.3.6-.7.5-1.2z"],
    ),
    (
        "lightbulb",
        &[
            "M15 14c.2-1 .7-1.7 1.5-2.5 1-.9 1.5-2.2 1.5-3.5A6 6 0 0 0 6 8c0 1 .2 2.2 1.5 3.5.7.7 1.3 1.5 1.5 2.5",
            "M9 18h6",
            "M10 22h4",
        ],
    ),
    (
        "heart",
        &["M19 14c1.49-1.46 3-3.21 3-5.5A5.5 5.5 0 0 0 16.5 3c-1.76 0-3 .5-4.5 2-1.5-1.5-2.74-2-4.5-2A5.5 5.5 0 0 0 2 8.5c0 2.3 1.5 4.05 3 5.5l7 7Z"],
    ),
];
/// The icon for a bundled emoji category (in `sticktalk_emoji::categories` order).
pub(super) fn category_icon(index: usize) -> AnyView {
    let name = [
        "smile",
        "sprout",
        "pizza",
        "trophy",
        "plane",
        "lightbulb",
        "heart",
        "flag",
    ]
    .get(index)
    .copied()
    .unwrap_or("smile");
    match GLYPHS.iter().find(|(n, _)| *n == name) {
        Some((_, paths)) => view!{<svg class="icon" aria-hidden="true" viewBox="0 0 24 24">{paths.iter().map(|d|view!{<path d=*d></path>}).collect_view()}</svg>}.into_any(),
        None => icon(name),
    }
}

/// Text faces, by mood: insert one into the message.
fn kaomoji() -> AnyView {
    view!{<div class=format!("{SCROLL} mt-1")>{crate::kaomoji::GROUPS.iter().map(|(group,faces)|view!{<section class="picker-section mt-3" aria-label=*group><h3 class="category mb-2">{*group}</h3><div class="kaomoji-grid">{faces.iter().map(|face|view!{<button type="button" class="kaomoji" data-live=format!("emoji:{face}")>{*face}</button>}).collect_view()}</div></section>}).collect_view()}</div>}.into_any()
}
