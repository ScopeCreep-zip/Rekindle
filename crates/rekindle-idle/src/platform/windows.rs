//! Windows idle-time detection via `GetLastInputInfo`.

pub(crate) fn windows_idle_seconds() -> Option<u64> {
    #[repr(C)]
    struct LastInputInfo {
        cb_size: u32,
        dw_time: u32,
    }
    extern "system" {
        fn GetLastInputInfo(plii: *mut LastInputInfo) -> i32;
        fn GetTickCount() -> u32;
    }
    let mut lii = LastInputInfo {
        cb_size: 8,
        dw_time: 0,
    };
    // SAFETY: lii is a valid, initialized LastInputInfo with correct cb_size.
    // GetLastInputInfo and GetTickCount are safe Win32 query functions.
    if unsafe { GetLastInputInfo(&mut lii) } != 0 {
        let idle_ms = unsafe { GetTickCount() }.wrapping_sub(lii.dw_time);
        Some(u64::from(idle_ms) / 1000)
    } else {
        None
    }
}
