//! The right-click menu keeps what an app needs (copy, paste, spelling,
//! saving an image) and drops what only a browser needs.

/// WebView2 names its own menu items after their English labels, in camel
/// case. Unknown names are kept: they are editing or spelling entries.
const BROWSER_ONLY: &[&str] = &[
    "back",
    "forward",
    "reload",
    "refresh",
    "saveAs",
    "print",
    "share",
    "webCapture",
    "webSelect",
    "createQrCode",
    "readAloud",
    "translate",
    "viewSource",
    "viewPageSource",
    "openLinkInNewWindow",
    "saveLinkAs",
];
/// Developer tools exist only in debug builds.
const DEVELOPER: &[&str] = &["inspectElement", "inspect"];

/// For each `(name, is_separator)` entry, whether it stays. Separators stay
/// only between two entries that stay.
pub fn visible(items: &[(String, bool)], developer: bool) -> Vec<bool> {
    let mut keep = vec![false; items.len()];
    let mut pending_separator = None;
    let mut any_before = false;
    for (index, (name, separator)) in items.iter().enumerate() {
        if *separator {
            if any_before {
                pending_separator = Some(index);
            }
            continue;
        }
        let name = name.as_str();
        if BROWSER_ONLY.contains(&name) || (!developer && DEVELOPER.contains(&name)) {
            continue;
        }
        if let Some(separator) = pending_separator.take() {
            keep[separator] = true;
        }
        keep[index] = true;
        any_before = true;
    }
    keep
}

#[cfg(test)]
mod tests {
    use super::*;

    fn menu(names: &[&str]) -> Vec<(String, bool)> {
        names
            .iter()
            .map(|name| (name.to_string(), *name == "separator"))
            .collect()
    }

    fn shown(names: &[&str], developer: bool) -> Vec<String> {
        let items = menu(names);
        visible(&items, developer)
            .into_iter()
            .zip(items)
            .filter_map(|(keep, (name, _))| keep.then_some(name))
            .collect()
    }

    #[test]
    fn a_page_menu_made_of_browser_commands_disappears() {
        let page = [
            "back",
            "forward",
            "reload",
            "separator",
            "saveAs",
            "print",
            "separator",
            "inspectElement",
        ];
        assert!(shown(&page, false).is_empty());
        assert_eq!(shown(&page, true), ["inspectElement"]);
    }

    #[test]
    fn editing_and_spelling_entries_stay_with_tidy_separators() {
        let field = [
            "spellCheck",
            "spellCheck",
            "separator",
            "emoji",
            "separator",
            "undo",
            "redo",
            "separator",
            "cut",
            "copy",
            "paste",
            "pasteAndMatchStyle",
            "selectAll",
            "separator",
            "share",
            "separator",
            "inspectElement",
        ];
        assert_eq!(
            shown(&field, false),
            [
                "spellCheck",
                "spellCheck",
                "separator",
                "emoji",
                "separator",
                "undo",
                "redo",
                "separator",
                "cut",
                "copy",
                "paste",
                "pasteAndMatchStyle",
                "selectAll",
            ]
        );
    }

    #[test]
    fn link_and_image_menus_keep_copy_and_save_image() {
        assert_eq!(
            shown(
                &[
                    "openLinkInNewWindow",
                    "saveLinkAs",
                    "copyLinkLocation",
                    "separator",
                    "saveImageAs",
                    "copyImage",
                    "copyImageLocation",
                    "separator",
                    "separator",
                    "print",
                ],
                false
            ),
            [
                "copyLinkLocation",
                "separator",
                "saveImageAs",
                "copyImage",
                "copyImageLocation"
            ]
        );
        assert!(visible(&[], false).is_empty());
        assert!(shown(&["separator", "separator"], false).is_empty());
    }
}
