//! Monitor-specific pacing. DXGI/DWM may otherwise pace a secondary window at
//! the PRIMARY display's refresh (observed 280 submissions/s on a 75 Hz display).
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
static INTERVAL_NS: AtomicU64 = AtomicU64::new(16_666_667);
pub fn set_refresh_millihertz(rate: u32) {
    if (10_000..=1_000_000).contains(&rate) {
        INTERVAL_NS.store(1_000_000_000_000 / rate as u64, Ordering::Relaxed);
    }
}
pub fn next(now: Instant) -> iced::window::RedrawRequest {
    iced::window::RedrawRequest::At(now + Duration::from_nanos(INTERVAL_NS.load(Ordering::Relaxed)))
}
pub fn input_request(last: Option<Instant>, now: Instant) -> iced::window::RedrawRequest {
    match last.map(next) {
        Some(iced::window::RedrawRequest::At(at)) if at > now => {
            iced::window::RedrawRequest::At(at)
        }
        _ => iced::window::RedrawRequest::NextFrame,
    }
}
pub fn monitor_rate(window: &dyn iced_runtime::window::Window) -> u32 {
    #[cfg(windows)]
    {
        use iced_runtime::window::raw_window_handle::RawWindowHandle;
        use windows_sys::Win32::{Foundation::*, Graphics::Gdi::*};
        let Ok(handle) = window.window_handle() else {
            return 60_000;
        };
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return 60_000;
        };
        // SAFETY: live iced HWND; initialized, correctly sized Win32 output buffers.
        unsafe {
            let monitor = MonitorFromWindow(handle.hwnd.get() as HWND, MONITOR_DEFAULTTONEAREST);
            let mut info: MONITORINFOEXW = std::mem::zeroed();
            info.monitorInfo.cbSize = std::mem::size_of_val(&info) as u32;
            let mut mode: DEVMODEW = std::mem::zeroed();
            mode.dmSize = std::mem::size_of_val(&mode) as u16;
            if GetMonitorInfoW(monitor, (&mut info as *mut MONITORINFOEXW).cast()) != 0
                && EnumDisplaySettingsW(info.szDevice.as_ptr(), ENUM_CURRENT_SETTINGS, &mut mode)
                    != 0
                && mode.dmDisplayFrequency > 1
            {
                mode.dmDisplayFrequency * 1000
            } else {
                60_000
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        60_000
    }
}
