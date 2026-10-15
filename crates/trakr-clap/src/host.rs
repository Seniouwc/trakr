//! The host side of CLAP: the `clap_host` a plugin sees, and the extensions it may ask for (GUI, timers,
//! file descriptors, state, logging, thread checks). One [`Shared`] exists per plugin instance; the
//! plugin's `host_data` points at it.

use clap_sys::ext::gui::{clap_host_gui, CLAP_EXT_GUI};
use clap_sys::ext::latency::{clap_host_latency, CLAP_EXT_LATENCY};
use clap_sys::ext::log::{clap_host_log, CLAP_EXT_LOG, CLAP_LOG_WARNING};
use clap_sys::ext::params::{clap_host_params, CLAP_EXT_PARAMS};
use clap_sys::ext::posix_fd_support::{clap_host_posix_fd_support, CLAP_EXT_POSIX_FD_SUPPORT};
use clap_sys::ext::state::{clap_host_state, CLAP_EXT_STATE};
use clap_sys::ext::thread_check::{clap_host_thread_check, CLAP_EXT_THREAD_CHECK};
use clap_sys::ext::timer_support::{clap_host_timer_support, CLAP_EXT_TIMER_SUPPORT};
use clap_sys::host::clap_host;
use std::ffi::{c_char, c_void, CStr};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

pub(crate) struct Timer {
    pub id: u32,
    pub period: Duration,
    pub next: Instant,
}

pub(crate) struct FdReg {
    pub fd: i32,
    pub flags: u32,
}

/// State shared between a plugin, the audio thread and the UI thread.
pub(crate) struct Shared {
    pub main_thread: ThreadId,
    pub audio_thread: OnceLock<ThreadId>,
    /// The plugin asked for `on_main_thread`.
    pub callback: AtomicBool,
    /// The plugin's window was closed or destroyed by the user.
    pub gui_closed: AtomicBool,
    pub show_requested: AtomicBool,
    pub hide_requested: AtomicBool,
    pub state_dirty: AtomicBool,
    pub timers: Mutex<Vec<Timer>>,
    pub fds: Mutex<Vec<FdReg>>,
    next_timer: AtomicU32,
    /// Parameter ids in song order, and the values the plugin reported since the UI last looked
    /// (NaN bits = nothing new).
    pub param_ids: OnceLock<Vec<u32>>,
    pub param_out: Vec<AtomicU32>,
}

impl Shared {
    pub fn new() -> Self {
        Shared {
            main_thread: std::thread::current().id(),
            audio_thread: OnceLock::new(),
            callback: AtomicBool::new(false),
            gui_closed: AtomicBool::new(false),
            show_requested: AtomicBool::new(false),
            hide_requested: AtomicBool::new(false),
            state_dirty: AtomicBool::new(false),
            timers: Mutex::new(Vec::new()),
            fds: Mutex::new(Vec::new()),
            next_timer: AtomicU32::new(1),
            param_out: (0..trakr_core::MAX_PLUGIN_PARAMS)
                .map(|_| AtomicU32::new(f32::NAN.to_bits()))
                .collect(),
            param_ids: OnceLock::new(),
        }
    }

    /// Record a parameter value the plugin changed itself (audio thread or UI thread).
    pub fn param_changed(&self, id: u32, value: f64) {
        if let Some(i) = self.param_ids.get().and_then(|ids| ids.iter().position(|p| *p == id)) {
            if let Some(slot) = self.param_out.get(i) {
                slot.store((value as f32).to_bits(), Ordering::Release);
            }
        }
    }
}

unsafe fn shared<'a>(host: *const clap_host) -> Option<&'a Shared> {
    ((*host).host_data as *const Shared).as_ref()
}

unsafe extern "C" fn request_callback(host: *const clap_host) {
    if let Some(s) = shared(host) {
        s.callback.store(true, Ordering::Release);
    }
}

unsafe extern "C" fn noop(_: *const clap_host) {}

pub(crate) unsafe extern "C" fn request_restart(h: *const clap_host) {
    noop(h)
}

pub(crate) unsafe extern "C" fn request_process(h: *const clap_host) {
    noop(h)
}

pub(crate) unsafe extern "C" fn request_callback_fn(h: *const clap_host) {
    request_callback(h)
}

// ---- gui ---------------------------------------------------------------------------------------

unsafe extern "C" fn gui_resize_hints_changed(_: *const clap_host) {}

unsafe extern "C" fn gui_request_resize(_: *const clap_host, _: u32, _: u32) -> bool {
    true // floating windows size themselves
}

unsafe extern "C" fn gui_request_show(host: *const clap_host) -> bool {
    if let Some(s) = shared(host) {
        s.show_requested.store(true, Ordering::Release);
    }
    true
}

unsafe extern "C" fn gui_request_hide(host: *const clap_host) -> bool {
    if let Some(s) = shared(host) {
        s.hide_requested.store(true, Ordering::Release);
    }
    true
}

unsafe extern "C" fn gui_closed(host: *const clap_host, _was_destroyed: bool) {
    if let Some(s) = shared(host) {
        s.gui_closed.store(true, Ordering::Release);
    }
}

static HOST_GUI: clap_host_gui = clap_host_gui {
    resize_hints_changed: Some(gui_resize_hints_changed),
    request_resize: Some(gui_request_resize),
    request_show: Some(gui_request_show),
    request_hide: Some(gui_request_hide),
    closed: Some(gui_closed),
};

// ---- timers and file descriptors -----------------------------------------------------------------

unsafe extern "C" fn timer_register(host: *const clap_host, period_ms: u32, out: *mut u32) -> bool {
    let Some(s) = shared(host) else { return false };
    if out.is_null() {
        return false;
    }
    let id = s.next_timer.fetch_add(1, Ordering::Relaxed);
    let period = Duration::from_millis(period_ms.clamp(10, 60_000) as u64);
    if let Ok(mut t) = s.timers.lock() {
        t.push(Timer { id, period, next: Instant::now() + period });
    }
    *out = id;
    true
}

unsafe extern "C" fn timer_unregister(host: *const clap_host, id: u32) -> bool {
    let Some(s) = shared(host) else { return false };
    s.timers.lock().map(|mut t| t.retain(|x| x.id != id)).is_ok()
}

static HOST_TIMER: clap_host_timer_support = clap_host_timer_support {
    register_timer: Some(timer_register),
    unregister_timer: Some(timer_unregister),
};

unsafe extern "C" fn fd_register(host: *const clap_host, fd: i32, flags: u32) -> bool {
    let Some(s) = shared(host) else { return false };
    if let Ok(mut f) = s.fds.lock() {
        f.retain(|x| x.fd != fd);
        f.push(FdReg { fd, flags });
    }
    true
}

unsafe extern "C" fn fd_modify(host: *const clap_host, fd: i32, flags: u32) -> bool {
    let Some(s) = shared(host) else { return false };
    s.fds
        .lock()
        .map(|mut f| {
            if let Some(x) = f.iter_mut().find(|x| x.fd == fd) {
                x.flags = flags;
            }
        })
        .is_ok()
}

unsafe extern "C" fn fd_unregister(host: *const clap_host, fd: i32) -> bool {
    let Some(s) = shared(host) else { return false };
    s.fds.lock().map(|mut f| f.retain(|x| x.fd != fd)).is_ok()
}

static HOST_FD: clap_host_posix_fd_support = clap_host_posix_fd_support {
    register_fd: Some(fd_register),
    modify_fd: Some(fd_modify),
    unregister_fd: Some(fd_unregister),
};

// ---- state, log, thread check, params, latency -------------------------------------------------

unsafe extern "C" fn state_mark_dirty(host: *const clap_host) {
    if let Some(s) = shared(host) {
        s.state_dirty.store(true, Ordering::Release);
    }
}

static HOST_STATE: clap_host_state = clap_host_state { mark_dirty: Some(state_mark_dirty) };

unsafe extern "C" fn log_message(_: *const clap_host, severity: i32, msg: *const c_char) {
    if severity >= CLAP_LOG_WARNING && !msg.is_null() {
        eprintln!("[clap plugin] {}", CStr::from_ptr(msg).to_string_lossy());
    }
}

static HOST_LOG: clap_host_log = clap_host_log { log: Some(log_message) };

unsafe extern "C" fn is_main_thread(host: *const clap_host) -> bool {
    shared(host).is_some_and(|s| s.main_thread == std::thread::current().id())
}

unsafe extern "C" fn is_audio_thread(host: *const clap_host) -> bool {
    shared(host).is_some_and(|s| s.audio_thread.get().is_some_and(|t| *t == std::thread::current().id()))
}

static HOST_THREAD_CHECK: clap_host_thread_check =
    clap_host_thread_check { is_main_thread: Some(is_main_thread), is_audio_thread: Some(is_audio_thread) };

unsafe extern "C" fn params_rescan(_: *const clap_host, _: u32) {}
unsafe extern "C" fn params_clear(_: *const clap_host, _: u32, _: u32) {}
unsafe extern "C" fn params_request_flush(_: *const clap_host) {}

static HOST_PARAMS: clap_host_params = clap_host_params {
    rescan: Some(params_rescan),
    clear: Some(params_clear),
    request_flush: Some(params_request_flush),
};

unsafe extern "C" fn latency_changed(_: *const clap_host) {}

static HOST_LATENCY: clap_host_latency = clap_host_latency { changed: Some(latency_changed) };

pub(crate) unsafe extern "C" fn get_extension(_: *const clap_host, id: *const c_char) -> *const c_void {
    if id.is_null() {
        return std::ptr::null();
    }
    let id = CStr::from_ptr(id);
    if id == CLAP_EXT_GUI {
        &HOST_GUI as *const _ as *const c_void
    } else if id == CLAP_EXT_TIMER_SUPPORT {
        &HOST_TIMER as *const _ as *const c_void
    } else if id == CLAP_EXT_POSIX_FD_SUPPORT {
        &HOST_FD as *const _ as *const c_void
    } else if id == CLAP_EXT_STATE {
        &HOST_STATE as *const _ as *const c_void
    } else if id == CLAP_EXT_LOG {
        &HOST_LOG as *const _ as *const c_void
    } else if id == CLAP_EXT_THREAD_CHECK {
        &HOST_THREAD_CHECK as *const _ as *const c_void
    } else if id == CLAP_EXT_PARAMS {
        &HOST_PARAMS as *const _ as *const c_void
    } else if id == CLAP_EXT_LATENCY {
        &HOST_LATENCY as *const _ as *const c_void
    } else {
        std::ptr::null()
    }
}
