//! Allocation-site diagnostics only. Does not change the shipping allocator/profile.
use std::cell::RefCell;
thread_local! {
    static PROFILER: RefCell<Option<dhat::Profiler>> = const { RefCell::new(None) };
}
pub struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        finish();
    }
}
pub fn start() -> Guard {
    let path = std::env::var("STICKTALK_HEAP_PROFILE").unwrap_or_else(|_| "dhat-heap.json".into());
    // Release is stripped by workspace policy. An MSVC /MAP file permits offline
    // symbolization of allocation stack addresses without changing optimization/debug info.
    #[cfg(windows)]
    let image_base = unsafe {
        // SAFETY: null requests the process executable's existing module handle.
        windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null()) as usize
    };
    #[cfg(not(windows))]
    let image_base = 0usize;
    let _ = std::fs::write(
        format!("{path}.image-base.json"),
        serde_json::json!({"image_base":image_base}).to_string(),
    );
    let profiler = dhat::Profiler::builder().file_name(path).build();
    PROFILER.with(|slot| *slot.borrow_mut() = Some(profiler));
    Guard
}
/// End at the actual retained-view checkpoint, before App/renderer/cache teardown.
pub fn finish() {
    let profiler = PROFILER.with(|slot| slot.borrow_mut().take());
    drop(profiler);
}
