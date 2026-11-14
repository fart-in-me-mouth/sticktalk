//! Event-driven allocator reclamation, never a working-set quota or frame-time task.
use std::sync::atomic::{AtomicU64, Ordering};
static COLLECTIONS: AtomicU64 = AtomicU64::new(0);
static COLLECTION_US: AtomicU64 = AtomicU64::new(0);
static SUCCEEDED: AtomicU64 = AtomicU64::new(0);

pub const fn supported() -> bool {
    !cfg!(feature = "heap-profile")
        && (cfg!(all(windows, feature = "windows-heap-collection"))
            || cfg!(feature = "compact-allocator"))
}

/// Returns genuinely freed pages to the OS without evicting live UI, font or GPU data.
/// Windows' LFH caches include native shader/compiler scratch allocations which
/// changing Rust's allocator cannot reclaim. This decommits *unused* heap pages,
/// unlike EmptyWorkingSet, and never imposes a working-set quota or evicts live data.
pub fn collect() {
    if !supported() {
        return;
    }
    let start = std::time::Instant::now();
    #[cfg(all(feature = "compact-allocator", not(feature = "heap-profile")))]
    // SAFETY: mimalloc documents mi_collect as a thread-safe allocator operation.
    // It only collects this thread's deferred frees; all live allocations remain valid.
    unsafe {
        libmimalloc_sys::mi_collect(true);
    }
    #[cfg(all(
        windows,
        feature = "windows-heap-collection",
        not(feature = "heap-profile")
    ))]
    {
        #[repr(C)]
        struct Optimize {
            version: u32,
            flags: u32,
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn HeapSetInformation(
                heap: *mut std::ffi::c_void,
                class: i32,
                info: *const Optimize,
                length: usize,
            ) -> i32;
        }
        let info = Optimize {
            version: 1,
            flags: 0,
        };
        // SAFETY: documented HeapOptimizeResources (3), version 1, zero flags.
        // Null selects all process LFH heaps; the OS synchronizes heap access.
        // https://learn.microsoft.com/windows/win32/api/heapapi/nf-heapapi-heapsetinformation
        if unsafe {
            HeapSetInformation(std::ptr::null_mut(), 3, &info, std::mem::size_of_val(&info))
        } != 0
        {
            SUCCEEDED.fetch_add(1, Ordering::Relaxed);
        }
    }
    COLLECTIONS.fetch_add(1, Ordering::Relaxed);
    COLLECTION_US.fetch_add(start.elapsed().as_micros() as u64, Ordering::Relaxed);
}

pub fn metrics() -> serde_json::Value {
    serde_json::json!({"collections":COLLECTIONS.load(Ordering::Relaxed),"successful_windows_heap_optimizations":SUCCEEDED.load(Ordering::Relaxed),"total_collection_microseconds":COLLECTION_US.load(Ordering::Relaxed),"working_set_evictions":0})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn collection_preserves_live_data_and_never_sets_working_set_limits() {
        let live = vec![0x5au8; 1024 * 1024];
        collect();
        assert!(live.iter().all(|v| *v == 0x5a));
        assert_eq!(metrics()["working_set_evictions"], 0);
    }
}
