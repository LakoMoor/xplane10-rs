//! The OpenGL strings Rendering Options shows. The original prints `glGetString(GL_VENDOR /
//! GL_RENDERER / GL_VERSION)` of its own context (reno_wind); the port renders with wgpu, so
//! on macOS it opens a throw-away legacy CGL context just to ask the same driver the same
//! question. Elsewhere it falls back to the wgpu adapter's description.

/// (vendor, renderer, version), or None when no GL context could be made.
#[cfg(target_os = "macos")]
pub fn query() -> Option<(String, String, String)> {
    use std::ffi::{c_char, c_int, c_void, CStr};
    #[link(name = "OpenGL", kind = "framework")]
    extern "C" {
        fn CGLChoosePixelFormat(attribs: *const c_int, pix: *mut *mut c_void, npix: *mut c_int) -> c_int;
        fn CGLCreateContext(pix: *mut c_void, share: *mut c_void, ctx: *mut *mut c_void) -> c_int;
        fn CGLDestroyPixelFormat(pix: *mut c_void) -> c_int;
        fn CGLDestroyContext(ctx: *mut c_void) -> c_int;
        fn CGLGetCurrentContext() -> *mut c_void;
        fn CGLSetCurrentContext(ctx: *mut c_void) -> c_int;
        fn glGetString(name: u32) -> *const c_char;
    }
    // kCGLPFAAccelerated (73), then the list's terminator: the default (legacy 2.1) profile,
    // as the original's context.
    let attribs: [c_int; 2] = [73, 0];
    unsafe {
        let (mut pix, mut n) = (std::ptr::null_mut(), 0);
        if CGLChoosePixelFormat(attribs.as_ptr(), &mut pix, &mut n) != 0 || pix.is_null() {
            return None;
        }
        let mut ctx = std::ptr::null_mut();
        let ok = CGLCreateContext(pix, std::ptr::null_mut(), &mut ctx) == 0 && !ctx.is_null();
        CGLDestroyPixelFormat(pix);
        if !ok {
            return None;
        }
        let prev = CGLGetCurrentContext();
        CGLSetCurrentContext(ctx);
        let s = |name: u32| {
            let p = glGetString(name);
            (!p.is_null()).then(|| CStr::from_ptr(p).to_string_lossy().into_owned())
        };
        // GL_VENDOR, GL_RENDERER, GL_VERSION.
        let out = (s(0x1F00), s(0x1F01), s(0x1F02));
        CGLSetCurrentContext(prev);
        CGLDestroyContext(ctx);
        Some((out.0?, out.1?, out.2?))
    }
}

#[cfg(not(target_os = "macos"))]
pub fn query() -> Option<(String, String, String)> {
    None
}
