//! Where the mouse pointer is on the desktop (physical pixels), so the launcher can open on the
//! screen the user is working on. Wayland does not tell apps where the pointer is; there the
//! compositor places new windows itself (KDE and GNOME put them on the screen with the pointer).

#[cfg(windows)]
pub fn global_cursor() -> Option<(i32, i32)> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut p = POINT { x: 0, y: 0 };
    // SAFETY: GetCursorPos only writes the POINT it is given.
    (unsafe { GetCursorPos(&mut p) } != 0).then_some((p.x, p.y))
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn global_cursor() -> Option<(i32, i32)> {
    // winit uses Wayland whenever it is available and compiled in.
    if cfg!(feature = "wayland") && std::env::var_os("WAYLAND_DISPLAY").is_some() {
        return None;
    }
    let xlib = x11_dl::xlib::Xlib::open().ok()?;
    // SAFETY: plain Xlib calls on a display we open and close here; every out-pointer is a
    // local that lives for the call.
    unsafe {
        let display = (xlib.XOpenDisplay)(std::ptr::null());
        if display.is_null() {
            return None;
        }
        let root = (xlib.XDefaultRootWindow)(display);
        let (mut root_ret, mut child) = (0, 0);
        let (mut x, mut y, mut wx, mut wy) = (0, 0, 0, 0);
        let mut mask = 0;
        let ok = (xlib.XQueryPointer)(display, root, &mut root_ret, &mut child, &mut x, &mut y, &mut wx, &mut wy, &mut mask);
        (xlib.XCloseDisplay)(display);
        (ok != 0).then_some((x, y))
    }
}

#[cfg(target_os = "macos")]
pub fn global_cursor() -> Option<(i32, i32)> {
    None
}
