//! Event-driven allocator reclamation, never a working-set quota or frame-time task.
/// Returns genuinely freed pages to the OS without evicting live UI, font or GPU data.
pub fn collect() {
    #[cfg(all(feature = "compact-allocator", not(feature = "heap-profile")))]
    // SAFETY: mimalloc documents mi_collect as a thread-safe allocator operation.
    // It only collects this thread's deferred frees; all live allocations remain valid.
    unsafe {
        libmimalloc_sys::mi_collect(true);
    }
}
