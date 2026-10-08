//! A GL context with no window and no display, on the right GPU.

use std::{env, ptr, sync::Arc};
use three_d::{Context, context};

use crate::render::{RenderError, RenderResult};

/// Runs `f` with stderr closed. Mesa prints `pci id for fd N: 10de:…, driver
/// (null)` for every GPU node it has no driver for (the NVIDIA one) while
/// EGL starts, before any of its log settings apply. Real failures still come
/// back through `f`'s result.
pub fn quiet_stderr<T, F: FnOnce() -> T>(f: F) -> T {
    if env::var_os("TRIDI_DEBUG").is_some() {
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
pub fn headless() -> RenderResult<(Context, impl Sized)> {
    quiet_stderr(open_headless)
}

fn open_headless() -> RenderResult<(Context, impl Sized)> {
    use glutin::api::egl::{device::Device, display::Display};
    use glutin::config::{ConfigSurfaceTypes, ConfigTemplateBuilder};
    use glutin::context::{ContextApi, ContextAttributesBuilder, Version};
    use glutin::prelude::*;
    let devs: Vec<Device> = Device::query_devices()
        .map_err(|source| RenderError::Egl {
            step: "list the devices",
            source,
        })?
        .collect();
    let software = |d: &Device| d.extensions().contains("EGL_MESA_device_software");
    // a GPU with a name that isn't NVIDIA (waking the dGPU costs ~2.7 s),
    // else llvmpipe; a device without a name is a node Mesa has no driver for
    let gpu = |d: &Device| {
        d.vendor().is_some_and(|v| !v.to_ascii_lowercase().contains("nvidia"))
            && !software(d)
            && !d.extensions().contains("EGL_NV_device_cuda")
    };
    let dev = match env::var("TRIDI_EGL_DEVICE").ok().and_then(|s| s.parse::<usize>().ok()) {
        Some(i) => devs.get(i),
        None => devs.iter().find(|d| gpu(d)).or(devs.iter().find(|d| software(d))),
    }
    .ok_or(RenderError::NoDevice)?;
    for (i, d) in devs.iter().enumerate() {
        let (vendor, name) = (d.vendor().unwrap_or("-"), d.name().unwrap_or("-"));
        debug!(used = ptr::eq(d, dev), software = software(d), "egl device {i}: {vendor} {name}");
    }
    // SAFETY: a device EGL itself listed, no native display
    let display = unsafe { Display::with_device(dev, None) }.map_err(|source| RenderError::Egl {
        step: "open the display",
        source,
    })?;
    let tmpl = ConfigTemplateBuilder::new().with_surface_type(ConfigSurfaceTypes::empty()).build();
    // SAFETY: the display opened above
    let config = unsafe { display.find_configs(tmpl) }
        .map_err(|source| RenderError::Egl {
            step: "list the configs",
            source,
        })?
        .next()
        .ok_or(RenderError::NoConfig)?;
    let attrs = ContextAttributesBuilder::new()
        .with_context_api(ContextApi::OpenGl(Some(Version::new(3, 3))))
        .build(None);
    // SAFETY: a config of this display, no window
    let gl_ctx = unsafe { display.create_context(&config, &attrs) }
        .map_err(|source| RenderError::Egl {
            step: "create a context",
            source,
        })?
        .make_current_surfaceless()
        .map_err(|source| RenderError::Egl {
            step: "make the context current",
            source,
        })?;
    // SAFETY: the context made current above, whose functions EGL returns
    let gl = unsafe { context::Context::from_loader_function_cstr(|s| display.get_proc_address(s)) };
    let ctx = Context::from_gl_context(Arc::new(gl)).map_err(|source| RenderError::Gl { step: "load GL", source })?;
    Ok((ctx, (gl_ctx, display)))
}
