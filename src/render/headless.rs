//! A GL context with no window and no display, on the right GPU.

use std::sync::Arc;
use three_d::{Context, context};

/// Runs `f` with stderr closed. Mesa prints `pci id for fd N: 10de:…, driver
/// (null)` for every GPU node it has no driver for (the NVIDIA one) while
/// EGL starts, before any of its log settings apply. Real failures still come
/// back through `f`'s result.
pub fn quiet_stderr<T>(f: impl FnOnce() -> T) -> T {
    if std::env::var_os("TRIDI_DEBUG").is_some() {
        return f();
    }
    // SAFETY: plain fd juggling on 2; the saved copy is restored and closed
    unsafe {
        let saved = libc::dup(2);
        let null = libc::open(c"/dev/null".as_ptr(), libc::O_WRONLY);
        if saved < 0 || null < 0 {
            return f();
        }
        libc::dup2(null, 2);
        libc::close(null);
        let r = f();
        libc::dup2(saved, 2);
        libc::close(saved);
        r
    }
}

/// GL context with no window and no display: EGL device + surfaceless.
/// The second value keeps the EGL context and display alive.
pub fn headless() -> Result<(Context, impl Sized), String> {
    quiet_stderr(open_headless)
}

fn open_headless() -> Result<(Context, impl Sized), String> {
    use glutin::api::egl::{device::Device, display::Display};
    use glutin::config::{ConfigSurfaceTypes, ConfigTemplateBuilder};
    use glutin::context::{ContextApi, ContextAttributesBuilder, Version};
    use glutin::prelude::*;
    let devs: Vec<Device> = Device::query_devices().map_err(|e| e.to_string())?.collect();
    let software = |d: &Device| d.extensions().contains("EGL_MESA_device_software");
    // a GPU with a name that isn't NVIDIA (waking the dGPU costs ~2.7 s),
    // else llvmpipe; a device without a name is a node Mesa has no driver for
    let gpu = |d: &Device| {
        d.vendor().is_some_and(|v| !v.to_ascii_lowercase().contains("nvidia"))
            && !software(d)
            && !d.extensions().contains("EGL_NV_device_cuda")
    };
    let dev = match std::env::var("TRIDI_EGL_DEVICE").ok().and_then(|s| s.parse::<usize>().ok()) {
        Some(i) => devs.get(i),
        None => devs.iter().find(|d| gpu(d)).or(devs.iter().find(|d| software(d))),
    }
    .ok_or("no usable EGL device")?;
    if std::env::var_os("TRIDI_DEBUG").is_some() {
        for (i, d) in devs.iter().enumerate() {
            let tag = if std::ptr::eq(d, dev) { "  <- used" } else { "" };
            eprintln!("egl device {i}: {:?} {:?} software={}{tag}", d.vendor(), d.name(), software(d));
        }
    }
    let display = unsafe { Display::with_device(dev, None) }.map_err(|e| e.to_string())?;
    let tmpl = ConfigTemplateBuilder::new().with_surface_type(ConfigSurfaceTypes::empty()).build();
    let config = unsafe { display.find_configs(tmpl) }
        .map_err(|e| e.to_string())?
        .next()
        .ok_or("no EGL config")?;
    let attrs = ContextAttributesBuilder::new()
        .with_context_api(ContextApi::OpenGl(Some(Version::new(3, 3))))
        .build(None);
    let gl_ctx = unsafe { display.create_context(&config, &attrs) }
        .map_err(|e| e.to_string())?
        .make_current_surfaceless()
        .map_err(|e| e.to_string())?;
    let gl = unsafe { context::Context::from_loader_function_cstr(|s| display.get_proc_address(s)) };
    let ctx = Context::from_gl_context(Arc::new(gl)).map_err(|e| e.to_string())?;
    Ok((ctx, (gl_ctx, display)))
}
