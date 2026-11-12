use sticktalk_desktop_platform::tray::*;
fn main() {
    let preferences = DesktopPreferences::default();
    println!(
        "Dry run: close={:?}, startup={}",
        preferences.close_action(true),
        preferences.start_with_windows
    );
    for (id, label, _) in MENU {
        println!("{label} -> {:?}", menu_action(id));
    }
    // Native: retain TrayIcon::add(hwnd, WM_APP+1, icon); route its callback to
    // tray.menu(muted, deafened), reduce TrayAction, handle CloseAction on close.
    // Only after user opt-in: windows::set_start_with_windows(&exe, true)?;
}
