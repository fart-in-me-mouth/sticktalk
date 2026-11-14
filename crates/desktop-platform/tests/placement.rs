#![cfg(windows)]
//! Invisible native geometry tests. Never activate, show or focus a window.
use sticktalk_desktop_platform::windows::placement;
use windows::{
    core::w,
    Win32::{Foundation::HWND, UI::WindowsAndMessaging::*},
};
struct Hidden(HWND);
impl Drop for Hidden {
    fn drop(&mut self) {
        let _ = unsafe { DestroyWindow(self.0) };
    }
}
#[test]
fn native_hidden_placement_roundtrips_and_invalid_data_stays_hidden() {
    let root = tempfile::tempdir().unwrap();
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!("StickTalk invisible placement test"),
            WS_OVERLAPPEDWINDOW,
            100,
            100,
            1000,
            650,
            None,
            None,
            None,
            None,
        )
        .unwrap()
    };
    let _window = Hidden(hwnd);
    assert!(!unsafe { IsWindowVisible(hwnd) }.as_bool());
    placement::save(hwnd, root.path());
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.path().join("window.json")).unwrap()).unwrap();
    assert_eq!(saved["maximized"], false);
    placement::restore(hwnd, root.path(), true);
    assert!(!unsafe { IsWindowVisible(hwnd) }.as_bool());
    let mut state = WINDOWPLACEMENT {
        length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    unsafe {
        GetWindowPlacement(hwnd, &mut state).unwrap();
    }
    assert!(state.rcNormalPosition.right - state.rcNormalPosition.left >= 940);
    assert!(state.rcNormalPosition.bottom - state.rcNormalPosition.top >= 600);
    std::fs::write(root.path().join("window.json"), b"invalid json").unwrap();
    placement::restore(hwnd, root.path(), true);
    assert!(!unsafe { IsWindowVisible(hwnd) }.as_bool());
}
