//! Optional benchmark-only native driver crash diagnostics. No application hook.
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::HMODULE,
        System::{Diagnostics::Debug::*, LibraryLoader::*},
    },
};
unsafe extern "system" fn handler(info: *const EXCEPTION_POINTERS) -> i32 {
    let record = &*(*info).ExceptionRecord;
    eprintln!(
        "Native exception {:#x} at {:p}",
        record.ExceptionCode.0, record.ExceptionAddress
    );
    print_address(record.ExceptionAddress);
    let mut stack = [std::ptr::null_mut(); 48];
    let count = RtlCaptureStackBackTrace(0, &mut stack, None);
    for address in &stack[..count as usize] {
        print_address(*address);
    }
    EXCEPTION_EXECUTE_HANDLER
}
unsafe fn print_address(address: *mut core::ffi::c_void) {
    let mut module = HMODULE::default();
    if GetModuleHandleExW(
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
        PCWSTR(address as *const u16),
        &mut module,
    )
    .is_ok()
    {
        let mut text = [0u16; 1024];
        let len = GetModuleFileNameW(Some(module), &mut text) as usize;
        eprintln!(
            "  {} + {:#x}",
            String::from_utf16_lossy(&text[..len]),
            address as usize - module.0 as usize
        );
    }
}
pub fn install() {
    unsafe {
        SetUnhandledExceptionFilter(Some(handler));
    }
}
