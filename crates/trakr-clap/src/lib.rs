//! Hosts local CLAP effect and instrument plugins (`.clap` shared libraries).
//!
//! Scope: audio effects with a mono or stereo main input and output, and instruments (note input, no
//! audio input, mono or stereo output). Parameters are exposed through the
//! plugin's `clap.params` extension and set with sample-accurate-enough parameter events at the start of
//! each block. Not supported yet: plugin GUIs, state save/restore, latency
//! reporting, multiple audio ports, 64-bit buffers, hosting extensions beyond the minimum.
//!
//! A plugin is native code running in this process: a crashing plugin takes trakr down with it.

use clap_sys::audio_buffer::clap_audio_buffer;
use clap_sys::entry::clap_plugin_entry;
use clap_sys::events::{
    clap_event_header, clap_event_note, clap_event_param_value, clap_input_events, clap_output_events,
    CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_NOTE_OFF, CLAP_EVENT_NOTE_ON, CLAP_EVENT_PARAM_VALUE,
};
use clap_sys::ext::audio_ports::{clap_audio_port_info, clap_plugin_audio_ports, CLAP_EXT_AUDIO_PORTS};
use clap_sys::ext::gui::{clap_plugin_gui, CLAP_EXT_GUI, CLAP_WINDOW_API_WAYLAND, CLAP_WINDOW_API_X11};
use clap_sys::ext::note_ports::{clap_plugin_note_ports, CLAP_EXT_NOTE_PORTS};
use clap_sys::ext::params::{
    clap_param_info, clap_plugin_params, CLAP_EXT_PARAMS, CLAP_PARAM_IS_HIDDEN, CLAP_PARAM_IS_READONLY,
    CLAP_PARAM_IS_STEPPED,
};
use clap_sys::ext::posix_fd_support::{clap_plugin_posix_fd_support, CLAP_EXT_POSIX_FD_SUPPORT};
use clap_sys::ext::state::{clap_plugin_state, CLAP_EXT_STATE};
use clap_sys::ext::timer_support::{clap_plugin_timer_support, CLAP_EXT_TIMER_SUPPORT};
use clap_sys::factory::plugin_factory::{clap_plugin_factory, CLAP_PLUGIN_FACTORY_ID};
use clap_sys::host::clap_host;
use clap_sys::plugin::clap_plugin;
use clap_sys::process::{clap_process, CLAP_PROCESS_ERROR};
use clap_sys::stream::{clap_istream, clap_ostream};
use clap_sys::version::CLAP_VERSION;
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, Weak};
use trakr_core::{PluginInfo, PluginParam, PluginRef, MAX_PLUGIN_PARAMS};
use trakr_engine::{Device, LoadedPlugin, PluginGui, PluginHost};

mod host;
use host::Shared;

/// Most frames handed to a plugin in one call.
const MAX_FRAMES: usize = 4096;
/// Notes queued between two blocks.
const MAX_NOTES: usize = 128;

/// A loaded `.clap` library with its entry point initialised.
struct Library {
    entry: *const clap_plugin_entry,
    // Dropped after `deinit` runs in `Drop`.
    _lib: libloading::Library,
}

// The library handle and entry table are only used behind `Arc`, for factory calls on the loading
// thread and for `deinit` on drop.
unsafe impl Send for Library {}
unsafe impl Sync for Library {}

impl Library {
    fn open(path: &Path) -> Result<Arc<Library>, String> {
        unsafe {
            let lib =
                libloading::Library::new(path).map_err(|e| format!("cannot load {}: {e}", path.display()))?;
            let entry: *const clap_plugin_entry = *lib
                .get::<*const clap_plugin_entry>(b"clap_entry\0")
                .map_err(|_| format!("{} is not a CLAP plugin (no clap_entry)", path.display()))?;
            if entry.is_null() {
                return Err("clap_entry is null".into());
            }
            let e = &*entry;
            if !clap_sys::version::clap_version_is_compatible(e.clap_version) {
                return Err("unsupported CLAP version".into());
            }
            let cpath =
                CString::new(path.to_string_lossy().as_bytes()).map_err(|_| "bad path".to_string())?;
            let init = e.init.ok_or("plugin has no init")?;
            if !init(cpath.as_ptr()) {
                return Err(format!("{} refused to initialise", path.display()));
            }
            Ok(Arc::new(Library { entry, _lib: lib }))
        }
    }

    fn factory(&self) -> Result<&clap_plugin_factory, String> {
        unsafe {
            let get = (*self.entry).get_factory.ok_or("plugin has no factory")?;
            let f = get(CLAP_PLUGIN_FACTORY_ID.as_ptr()) as *const clap_plugin_factory;
            f.as_ref().ok_or_else(|| "plugin has no plugin factory".to_string())
        }
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        unsafe {
            if let Some(deinit) = (*self.entry).deinit {
                deinit();
            }
        }
    }
}

fn cstr(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }
}

fn array_str(a: &[c_char]) -> String {
    let bytes: Vec<u8> = a.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Whether the null-terminated feature list of a descriptor contains `name`.
fn has_feature(mut list: *const *const c_char, name: &str) -> bool {
    if list.is_null() {
        return false;
    }
    unsafe {
        while !(*list).is_null() {
            if cstr(*list) == name {
                return true;
            }
            list = list.add(1);
        }
    }
    false
}

/// Plugins in one file.
fn describe(path: &Path) -> Result<Vec<PluginInfo>, String> {
    let lib = Library::open(path)?;
    let factory = lib.factory()?;
    let (count, desc) = unsafe {
        (
            factory.get_plugin_count.ok_or("no get_plugin_count")?(factory),
            factory.get_plugin_descriptor.ok_or("no get_plugin_descriptor")?,
        )
    };
    let mut out = Vec::new();
    for i in 0..count {
        let d = unsafe { desc(factory, i).as_ref() };
        if let Some(d) = d {
            out.push(PluginInfo {
                instrument: has_feature(d.features, "instrument"),
                path: path.to_string_lossy().into_owned(),
                plugin_id: cstr(d.id),
                name: cstr(d.name),
                vendor: cstr(d.vendor),
            });
        }
    }
    Ok(out)
}

/// The folders CLAP plugins are searched in: `$CLAP_PATH`, `~/.clap`, `/usr/lib/clap`, `/usr/local/lib/clap`.
pub fn default_search_paths() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> =
        std::env::var_os("CLAP_PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if let Some(home) = std::env::var_os("HOME") {
        v.push(PathBuf::from(home).join(".clap"));
    }
    v.push("/usr/lib/clap".into());
    v.push("/usr/local/lib/clap".into());
    v
}

/// Find `.clap` files in `dirs` (one level of sub-folders deep) and list the plugins inside.
pub fn scan(dirs: &[PathBuf]) -> Vec<PluginInfo> {
    let mut files = Vec::new();
    for d in dirs {
        collect(d, 0, &mut files);
    }
    files.sort();
    files.dedup();
    files.iter().filter_map(|f| describe(f).ok()).flatten().collect()
}

fn collect(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() && depth < 1 {
            collect(&p, depth + 1, out);
        } else if p.extension().is_some_and(|x| x == "clap") && p.is_file() {
            out.push(p);
        }
    }
}

/// [`PluginHost`] backed by CLAP libraries on disk.
pub struct ClapHost {
    dirs: Vec<PathBuf>,
    live: Vec<Weak<Instance>>,
}

impl ClapHost {
    pub fn new() -> Self {
        Self { dirs: default_search_paths(), live: Vec::new() }
    }

    pub fn with_paths(dirs: Vec<PathBuf>) -> Self {
        Self { dirs, live: Vec::new() }
    }
}

impl Default for ClapHost {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginHost for ClapHost {
    fn scan(&mut self) -> Vec<PluginInfo> {
        scan(&self.dirs)
    }

    fn load(
        &mut self,
        path: &str,
        plugin_id: &str,
        sample_rate: u32,
    ) -> Result<(Box<dyn Device>, PluginRef), String> {
        let (dev, r, _gui) = ClapDevice::load_full(Path::new(path), plugin_id, sample_rate, &[])?;
        Ok((Box::new(dev), r))
    }

    fn load_with_gui(
        &mut self,
        path: &str,
        plugin_id: &str,
        sample_rate: u32,
        state: &[u8],
    ) -> Result<LoadedPlugin, String> {
        let (dev, r, gui) = ClapDevice::load_full(Path::new(path), plugin_id, sample_rate, state)?;
        self.live.retain(|w| w.strong_count() > 0);
        self.live.push(Arc::downgrade(&dev.inst));
        Ok((Box::new(dev), r, Some(Box::new(gui))))
    }

    fn poll(&mut self) {
        self.live.retain(|w| w.strong_count() > 0);
        for w in &self.live {
            if let Some(inst) = w.upgrade() {
                inst.pump();
            }
        }
    }
}

// ---- plugin instance -----------------------------------------------------------------------------

/// A plugin instance, shared by its audio device and its UI handle; the last one to go destroys it.
struct Instance {
    plugin: *const clap_plugin,
    // The plugin keeps a pointer to the host; both must outlive it.
    _host: Box<clap_host>,
    shared: Arc<Shared>,
    window: Mutex<bool>,
    // Keeps the library loaded for as long as the plugin exists; dropped after `destroy`.
    _lib: Arc<Library>,
}

// The plugin is created on the UI thread, processed on the audio thread and shown from the UI thread, as
// the CLAP threading rules allow.
unsafe impl Send for Instance {}
unsafe impl Sync for Instance {}

impl Instance {
    fn ext<T>(&self, id: &CStr) -> Option<&T> {
        unsafe {
            let get = (*self.plugin).get_extension?;
            (get(self.plugin, id.as_ptr()) as *const T).as_ref()
        }
    }

    fn gui_ext(&self) -> Option<&clap_plugin_gui> {
        self.ext(CLAP_EXT_GUI)
    }

    /// The window system to ask the plugin for, if it can open a floating window on one.
    fn window_api(&self) -> Option<&'static CStr> {
        let g = self.gui_ext()?;
        let supported = g.is_api_supported?;
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
        let order = if wayland {
            [CLAP_WINDOW_API_WAYLAND, CLAP_WINDOW_API_X11]
        } else {
            [CLAP_WINDOW_API_X11, CLAP_WINDOW_API_WAYLAND]
        };
        order.into_iter().find(|api| unsafe { supported(self.plugin, api.as_ptr(), true) })
    }

    fn show_window(&self) -> Result<(), String> {
        let g = self.gui_ext().ok_or("this plugin has no window")?;
        let api = self.window_api().ok_or("the plugin cannot open a window on this display system")?;
        let mut created = self.window.lock().map_err(|_| "window state poisoned")?;
        unsafe {
            if !*created {
                if !g.create.ok_or("plugin gui has no create")?(self.plugin, api.as_ptr(), true) {
                    return Err("the plugin could not create its window".into());
                }
                *created = true;
                if let Some(title) = g.suggest_title {
                    let name = cstr((*(*self.plugin).desc).name);
                    if let Ok(c) = CString::new(format!("trakr: {name}")) {
                        title(self.plugin, c.as_ptr());
                    }
                }
            }
            self.shared.gui_closed.store(false, Ordering::Release);
            if !g.show.ok_or("plugin gui has no show")?(self.plugin) {
                return Err("the plugin refused to show its window".into());
            }
        }
        Ok(())
    }

    fn hide_window(&self) {
        let Some(g) = self.gui_ext() else { return };
        let Ok(created) = self.window.lock() else { return };
        if *created {
            if let Some(hide) = g.hide {
                unsafe { hide(self.plugin) };
            }
        }
        self.shared.gui_closed.store(true, Ordering::Release);
    }

    fn destroy_window(&self) {
        let Ok(mut created) = self.window.lock() else { return };
        if *created {
            if let Some(d) = self.gui_ext().and_then(|g| g.destroy) {
                unsafe { d(self.plugin) };
            }
            *created = false;
        }
    }

    /// Main-thread work: callbacks, timers, file descriptors and window requests.
    fn pump(&self) {
        let sh = &self.shared;
        unsafe {
            if sh.callback.swap(false, Ordering::AcqRel) {
                if let Some(f) = (*self.plugin).on_main_thread {
                    f(self.plugin);
                }
            }
            if sh.show_requested.swap(false, Ordering::AcqRel) {
                let _ = self.show_window();
            }
            if sh.hide_requested.swap(false, Ordering::AcqRel) {
                self.hide_window();
            }
            let due: Vec<u32> = {
                let now = std::time::Instant::now();
                match sh.timers.lock() {
                    Ok(mut t) => t
                        .iter_mut()
                        .filter(|t| t.next <= now)
                        .map(|t| {
                            t.next = now + t.period;
                            t.id
                        })
                        .collect(),
                    Err(_) => Vec::new(),
                }
            };
            if !due.is_empty() {
                if let Some(f) =
                    self.ext::<clap_plugin_timer_support>(CLAP_EXT_TIMER_SUPPORT).and_then(|e| e.on_timer)
                {
                    for id in due {
                        f(self.plugin, id);
                    }
                }
            }
            let regs: Vec<(i32, u32)> =
                sh.fds.lock().map(|f| f.iter().map(|r| (r.fd, r.flags)).collect()).unwrap_or_default();
            if !regs.is_empty() {
                if let Some(f) =
                    self.ext::<clap_plugin_posix_fd_support>(CLAP_EXT_POSIX_FD_SUPPORT).and_then(|e| e.on_fd)
                {
                    let mut fds: Vec<libc::pollfd> = regs
                        .iter()
                        .map(|(fd, flags)| libc::pollfd {
                            fd: *fd,
                            events: (if flags & clap_sys::ext::posix_fd_support::CLAP_POSIX_FD_READ != 0 {
                                libc::POLLIN
                            } else {
                                0
                            }) | (if flags & clap_sys::ext::posix_fd_support::CLAP_POSIX_FD_WRITE != 0
                            {
                                libc::POLLOUT
                            } else {
                                0
                            }),
                            revents: 0,
                        })
                        .collect();
                    if libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, 0) > 0 {
                        for p in &fds {
                            let mut flags = 0;
                            if p.revents & libc::POLLIN != 0 {
                                flags |= clap_sys::ext::posix_fd_support::CLAP_POSIX_FD_READ;
                            }
                            if p.revents & libc::POLLOUT != 0 {
                                flags |= clap_sys::ext::posix_fd_support::CLAP_POSIX_FD_WRITE;
                            }
                            if p.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                                flags |= clap_sys::ext::posix_fd_support::CLAP_POSIX_FD_ERROR;
                            }
                            if flags != 0 {
                                f(self.plugin, p.fd, flags);
                            }
                        }
                    }
                }
            }
        }
    }

    fn save_state(&self) -> Option<Vec<u8>> {
        let ext = self.ext::<clap_plugin_state>(CLAP_EXT_STATE)?;
        let save = ext.save?;
        let mut bytes: Vec<u8> = Vec::new();
        unsafe extern "C" fn write(s: *const clap_ostream, buf: *const c_void, size: u64) -> i64 {
            let out = &mut *((*s).ctx as *mut Vec<u8>);
            if out.len() as u64 + size > trakr_core::plugin::MAX_PLUGIN_STATE as u64 {
                return -1;
            }
            out.extend_from_slice(std::slice::from_raw_parts(buf as *const u8, size as usize));
            size as i64
        }
        let stream = clap_ostream { ctx: &mut bytes as *mut Vec<u8> as *mut c_void, write: Some(write) };
        let ok = unsafe { save(self.plugin, &stream) };
        (ok && !bytes.is_empty()).then_some(bytes)
    }

    fn load_state(&self, bytes: &[u8]) -> bool {
        let Some(load) = self.ext::<clap_plugin_state>(CLAP_EXT_STATE).and_then(|e| e.load) else {
            return false;
        };
        struct Cursor<'a> {
            data: &'a [u8],
            at: usize,
        }
        unsafe extern "C" fn read(s: *const clap_istream, buf: *mut c_void, size: u64) -> i64 {
            let c = &mut *((*s).ctx as *mut Cursor);
            let n = (size as usize).min(c.data.len() - c.at);
            std::ptr::copy_nonoverlapping(c.data.as_ptr().add(c.at), buf as *mut u8, n);
            c.at += n;
            n as i64
        }
        let mut cur = Cursor { data: bytes, at: 0 };
        let stream = clap_istream { ctx: &mut cur as *mut Cursor as *mut c_void, read: Some(read) };
        unsafe { load(self.plugin, &stream) }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        self.destroy_window();
        unsafe {
            let p = &*self.plugin;
            if let Some(stop) = p.stop_processing {
                stop(self.plugin);
            }
            if let Some(d) = p.deactivate {
                d(self.plugin);
            }
            if let Some(d) = p.destroy {
                d(self.plugin);
            }
        }
    }
}

/// Main-thread handle of a plugin: window, state, parameter changes.
pub struct ClapGui {
    inst: Arc<Instance>,
    window: bool,
}

impl PluginGui for ClapGui {
    fn has_window(&self) -> bool {
        self.window
    }

    fn show(&mut self) -> Result<(), String> {
        self.inst.show_window()
    }

    fn hide(&mut self) {
        self.inst.hide_window();
    }

    fn is_visible(&self) -> bool {
        self.window
            && self.inst.window.lock().map(|c| *c).unwrap_or(false)
            && !self.inst.shared.gui_closed.load(Ordering::Acquire)
    }

    fn take_state_dirty(&mut self) -> bool {
        self.inst.shared.state_dirty.swap(false, Ordering::AcqRel)
    }

    fn save_state(&mut self) -> Option<Vec<u8>> {
        self.inst.save_state()
    }

    fn take_param_changes(&mut self, out: &mut Vec<(usize, f32)>) {
        for (i, slot) in self.inst.shared.param_out.iter().enumerate() {
            let bits = slot.swap(f32::NAN.to_bits(), Ordering::AcqRel);
            let v = f32::from_bits(bits);
            if v.is_finite() {
                out.push((i, v));
            }
        }
    }
}

// ---- audio device ----------------------------------------------------------------------------------

/// One event handed to the plugin.
#[derive(Clone, Copy)]
enum Ev {
    Param(clap_event_param_value),
    Note(clap_event_note),
}

/// Events handed to the plugin for one block: notes first, then changed parameters.
struct EventList {
    events: Vec<Ev>,
}

unsafe extern "C" fn events_size(list: *const clap_input_events) -> u32 {
    let l = &*((*list).ctx as *const EventList);
    l.events.len() as u32
}

unsafe extern "C" fn events_get(list: *const clap_input_events, index: u32) -> *const clap_event_header {
    let l = &*((*list).ctx as *const EventList);
    match l.events.get(index as usize) {
        Some(Ev::Param(e)) => &e.header,
        Some(Ev::Note(e)) => &e.header,
        None => std::ptr::null(),
    }
}

/// Events the plugin sends back: parameter changes made in its own window reach the UI.
unsafe extern "C" fn out_events_push(list: *const clap_output_events, ev: *const clap_event_header) -> bool {
    let h = &*ev;
    if h.space_id == CLAP_CORE_EVENT_SPACE_ID && h.type_ == CLAP_EVENT_PARAM_VALUE {
        let e = &*(ev as *const clap_event_param_value);
        if let Some(sh) = ((*list).ctx as *const Shared).as_ref() {
            sh.param_changed(e.param_id, e.value);
        }
    }
    true
}

/// A plugin instance running on a track (effect) or an instrument slot.
pub struct ClapDevice {
    inst: Arc<Instance>,
    ids: Vec<u32>,
    last: Vec<f32>,
    primed: bool,
    in_ch: u32,
    out_ch: u32,
    bufs: [Vec<f32>; 4],
    list: Box<EventList>,
    steady: i64,
    /// Instrument plugins: notes waiting for the next block, as (on, key, velocity).
    notes: Vec<(bool, u8, f32)>,
    instrument: bool,
    next_note_id: i32,
}

// The device is created on the UI thread and then used from the audio thread only; the engine never
// shares it. Plugins must tolerate that (CLAP: process() runs on the audio thread).
unsafe impl Send for ClapDevice {}

fn port_channels(p: &clap_plugin, ports: &clap_plugin_audio_ports, input: bool) -> Option<u32> {
    unsafe {
        let count = ports.count?(p, input);
        if count == 0 {
            return None;
        }
        let mut info: clap_audio_port_info = std::mem::zeroed();
        if !ports.get?(p, 0, input, &mut info) {
            return None;
        }
        Some(info.channel_count)
    }
}

impl ClapDevice {
    /// Load plugin `plugin_id` from `path`, activate it and describe its parameters.
    pub fn load(path: &Path, plugin_id: &str, sample_rate: u32) -> Result<(ClapDevice, PluginRef), String> {
        let (d, r, _) = Self::load_full(path, plugin_id, sample_rate, &[])?;
        Ok((d, r))
    }

    /// Like `load`, restoring `state` (empty = fresh) and returning the main-thread handle too.
    pub fn load_full(
        path: &Path,
        plugin_id: &str,
        sample_rate: u32,
        state: &[u8],
    ) -> Result<(ClapDevice, PluginRef, ClapGui), String> {
        let lib = Library::open(path)?;
        let factory = lib.factory()?;
        let id = CString::new(plugin_id).map_err(|_| "bad plugin id".to_string())?;
        let shared = Arc::new(Shared::new());
        let host = Box::new(clap_host {
            clap_version: CLAP_VERSION,
            host_data: Arc::as_ptr(&shared) as *mut c_void,
            name: c"trakr".as_ptr(),
            vendor: c"trakr".as_ptr(),
            url: c"".as_ptr(),
            version: c"0.0.1".as_ptr(),
            get_extension: Some(host::get_extension),
            request_restart: Some(host::request_restart),
            request_process: Some(host::request_process),
            request_callback: Some(host::request_callback_fn),
        });
        unsafe {
            let create = factory.create_plugin.ok_or("no create_plugin")?;
            let plugin = create(factory, &*host, id.as_ptr());
            let p = plugin.as_ref().ok_or_else(|| format!("{plugin_id} is not in {}", path.display()))?;
            if !p.init.ok_or("no init")?(plugin) {
                (p.destroy.ok_or("no destroy")?)(plugin);
                return Err("plugin failed to initialise".into());
            }
            let fail = |msg: &str| -> Result<(ClapDevice, PluginRef, ClapGui), String> {
                (p.destroy.unwrap())(plugin);
                Err(msg.to_string())
            };
            let get_ext = p.get_extension.ok_or("no get_extension")?;
            let ports =
                (get_ext(plugin, CLAP_EXT_AUDIO_PORTS.as_ptr()) as *const clap_plugin_audio_ports).as_ref();
            let Some(ports) = ports else { return fail("not an audio plugin (no audio ports)") };
            let note_in = (get_ext(plugin, CLAP_EXT_NOTE_PORTS.as_ptr()) as *const clap_plugin_note_ports)
                .as_ref()
                .is_some_and(|n| n.count.is_some_and(|c| c(plugin, true) > 0));
            let in_ch = port_channels(p, ports, true);
            let instrument = in_ch.is_none() && note_in;
            let Some(in_ch) = in_ch.or(instrument.then_some(0)) else {
                return fail("not usable: an effect needs an audio input, an instrument needs a note input");
            };
            let Some(out_ch) = port_channels(p, ports, false) else {
                return fail("plugin has no audio output");
            };
            if (!instrument && !(1..=2).contains(&in_ch)) || !(1..=2).contains(&out_ch) {
                return fail("only mono and stereo plugins are supported");
            }

            let mut params = Vec::new();
            let mut ids = Vec::new();
            if let Some(pe) =
                (get_ext(plugin, CLAP_EXT_PARAMS.as_ptr()) as *const clap_plugin_params).as_ref()
            {
                let (count, get_info) = (pe.count.map_or(0, |c| c(plugin)), pe.get_info);
                for i in 0..count {
                    if params.len() >= MAX_PLUGIN_PARAMS {
                        break;
                    }
                    let Some(get_info) = get_info else { break };
                    let mut info: clap_param_info = std::mem::zeroed();
                    if !get_info(plugin, i, &mut info) {
                        continue;
                    }
                    if info.flags & (CLAP_PARAM_IS_HIDDEN | CLAP_PARAM_IS_READONLY) != 0 {
                        continue;
                    }
                    let (min, max) = (info.min_value as f32, info.max_value as f32);
                    if !min.is_finite() || !max.is_finite() || max <= min {
                        continue;
                    }
                    params.push(PluginParam {
                        id: info.id,
                        name: array_str(&info.name),
                        min,
                        max,
                        default: (info.default_value as f32).clamp(min, max),
                        stepped: info.flags & CLAP_PARAM_IS_STEPPED != 0,
                    });
                    ids.push(info.id);
                }
            }
            let _ = shared.param_ids.set(ids.clone());

            let activate = p.activate.ok_or("no activate")?;
            if !activate(plugin, sample_rate as f64, 1, MAX_FRAMES as u32) {
                return fail("plugin could not be activated");
            }
            let window = (get_ext(plugin, CLAP_EXT_GUI.as_ptr()) as *const clap_plugin_gui)
                .as_ref()
                .is_some_and(|g| {
                    g.is_api_supported.is_some_and(|f| {
                        f(plugin, CLAP_WINDOW_API_X11.as_ptr(), true)
                            || f(plugin, CLAP_WINDOW_API_WAYLAND.as_ptr(), true)
                    })
                });
            let inst =
                Arc::new(Instance { plugin, _host: host, shared, window: Mutex::new(false), _lib: lib });
            if !state.is_empty() {
                inst.load_state(state);
            }
            if let Some(start) = p.start_processing {
                if !start(plugin) {
                    // Dropping the instance deactivates and destroys the plugin.
                    return Err("plugin refused to start processing".into());
                }
            }
            let name = cstr(p.desc.as_ref().map_or(std::ptr::null(), |d| d.name));
            let n = ids.len();
            let device = ClapDevice {
                inst: inst.clone(),
                ids,
                last: vec![f32::NAN; n],
                primed: false,
                in_ch,
                out_ch,
                bufs: std::array::from_fn(|_| vec![0.0; MAX_FRAMES]),
                list: Box::new(EventList { events: Vec::with_capacity(MAX_PLUGIN_PARAMS + MAX_NOTES * 2) }),
                steady: 0,
                notes: Vec::with_capacity(MAX_NOTES),
                instrument,
                next_note_id: 0,
            };
            let reference = PluginRef {
                path: path.to_string_lossy().into_owned(),
                plugin_id: plugin_id.into(),
                name,
                params,
            };
            Ok((device, reference, ClapGui { inst, window }))
        }
    }
}

impl Device for ClapDevice {
    fn id(&self) -> &'static str {
        "clap"
    }

    fn is_instrument(&self) -> bool {
        self.instrument
    }

    fn note_on(&mut self, key: u8, velocity: f32) {
        if self.instrument && self.notes.len() < MAX_NOTES {
            self.notes.push((true, key, velocity));
        }
    }

    fn note_off(&mut self, key: u8) {
        if self.instrument && self.notes.len() < MAX_NOTES {
            self.notes.push((false, key, 0.0));
        }
    }

    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let _ = self.inst.shared.audio_thread.get_or_init(|| std::thread::current().id());
        let total = buf.len() / 2;
        let mut done = 0;
        while done < total {
            let n = (total - done).min(MAX_FRAMES);
            let frame = &mut buf[done * 2..(done + n) * 2];

            // queued notes first, then changed parameters, all at the start of the block
            self.list.events.clear();
            for (on, key, vel) in self.notes.drain(..) {
                let id = if on {
                    self.next_note_id = self.next_note_id.wrapping_add(1).max(0);
                    self.next_note_id
                } else {
                    -1
                };
                self.list.events.push(Ev::Note(clap_event_note {
                    header: clap_event_header {
                        size: std::mem::size_of::<clap_event_note>() as u32,
                        time: 0,
                        space_id: CLAP_CORE_EVENT_SPACE_ID,
                        type_: if on { CLAP_EVENT_NOTE_ON } else { CLAP_EVENT_NOTE_OFF },
                        flags: 0,
                    },
                    note_id: id,
                    port_index: 0,
                    channel: 0,
                    key: key as i16 + 12,
                    velocity: vel as f64,
                }));
            }
            for (i, id) in self.ids.iter().enumerate().take(params.len()) {
                let v = params[i];
                if !v.is_finite() || (self.primed && self.last[i] == v) {
                    continue;
                }
                self.last[i] = v;
                self.list.events.push(Ev::Param(clap_event_param_value {
                    header: clap_event_header {
                        size: std::mem::size_of::<clap_event_param_value>() as u32,
                        time: 0,
                        space_id: CLAP_CORE_EVENT_SPACE_ID,
                        type_: CLAP_EVENT_PARAM_VALUE,
                        flags: 0,
                    },
                    param_id: *id,
                    cookie: std::ptr::null_mut(),
                    note_id: -1,
                    port_index: -1,
                    channel: -1,
                    key: -1,
                    value: v as f64,
                }));
            }
            self.primed = true;

            let [in_l, in_r, out_l, out_r] = &mut self.bufs;
            for (i, f) in frame.chunks_exact(2).enumerate() {
                in_l[i] = f[0];
                in_r[i] = f[1];
            }
            let mut in_ptrs = [in_l.as_mut_ptr(), in_r.as_mut_ptr()];
            let mut out_ptrs = [out_l.as_mut_ptr(), out_r.as_mut_ptr()];
            let input = clap_audio_buffer {
                data32: in_ptrs.as_mut_ptr(),
                data64: std::ptr::null_mut(),
                channel_count: self.in_ch,
                latency: 0,
                constant_mask: 0,
            };
            let (in_ptr, in_count) =
                if self.instrument { (std::ptr::null(), 0) } else { (&input as *const _, 1) };
            let mut output = clap_audio_buffer {
                data32: out_ptrs.as_mut_ptr(),
                data64: std::ptr::null_mut(),
                channel_count: self.out_ch,
                latency: 0,
                constant_mask: 0,
            };
            let in_events = clap_input_events {
                ctx: &*self.list as *const EventList as *mut c_void,
                size: Some(events_size),
                get: Some(events_get),
            };
            let out_events = clap_output_events {
                ctx: Arc::as_ptr(&self.inst.shared) as *mut c_void,
                try_push: Some(out_events_push),
            };
            let process = clap_process {
                steady_time: self.steady,
                frames_count: n as u32,
                transport: std::ptr::null(),
                audio_inputs: in_ptr,
                audio_outputs: &mut output,
                audio_inputs_count: in_count,
                audio_outputs_count: 1,
                in_events: &in_events,
                out_events: &out_events,
            };
            self.steady += n as i64;
            let status = unsafe {
                match (*self.inst.plugin).process {
                    Some(f) => f(self.inst.plugin, &process),
                    None => CLAP_PROCESS_ERROR,
                }
            };
            if status != CLAP_PROCESS_ERROR {
                let [_, _, out_l, out_r] = &self.bufs;
                for (i, f) in frame.chunks_exact_mut(2).enumerate() {
                    let l = out_l[i];
                    // a mono output feeds both channels
                    let r = if self.out_ch == 1 { l } else { out_r[i] };
                    f[0] = if l.is_finite() { l } else { 0.0 };
                    f[1] = if r.is_finite() { r } else { 0.0 };
                }
            }
            done += n;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, OnceLock};
    use trakr_core::{AutoTarget, Automation, DeviceSpec, Instrument, LoopMode, Sample, Song};
    use trakr_engine::Command;

    /// Build the test plugin with cargo (once) and return a copy named `.clap` in a temp folder.
    fn plugin_dir() -> &'static Path {
        static DIR: OnceLock<PathBuf> = OnceLock::new();
        DIR.get_or_init(|| {
            let exe = std::env::current_exe().unwrap();
            let profile_dir = exe.parent().unwrap().parent().unwrap().to_path_buf();
            let mut cmd =
                std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
            cmd.args(["build", "-p", "trakr-test-plugin"]);
            if profile_dir.file_name().is_some_and(|n| n == "release") {
                cmd.arg("--release");
            }
            assert!(cmd.status().expect("run cargo").success(), "building the test plugin");
            let lib = ["libtrakr_test_plugin.so", "libtrakr_test_plugin.dylib", "trakr_test_plugin.dll"]
                .iter()
                .map(|n| profile_dir.join(n))
                .find(|p| p.exists())
                .expect("test plugin artifact");
            let dir = std::env::temp_dir().join(format!("trakr-clap-test-{}", std::process::id()));
            std::fs::create_dir_all(dir.join("nested")).unwrap();
            std::fs::copy(&lib, dir.join("nested/gain.clap")).unwrap();
            std::fs::write(dir.join("broken.clap"), b"not a library").unwrap();
            dir
        })
    }

    fn gain_path() -> String {
        plugin_dir().join("nested/gain.clap").to_string_lossy().into_owned()
    }

    #[test]
    fn scan_finds_plugins_and_skips_broken_files() {
        let found = scan(&[plugin_dir().to_path_buf(), PathBuf::from("/nonexistent")]);
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[0].plugin_id, "org.trakr.test-gain");
        assert_eq!(found[0].name, "Trakr Test Gain");
        assert_eq!(found[0].vendor, "trakr");
        assert!(!found[0].instrument);
        assert_eq!(found[1].plugin_id, "org.trakr.test-synth");
        assert!(found[1].instrument, "the synth announces the instrument feature");
        let mut host = ClapHost::with_paths(vec![plugin_dir().to_path_buf()]);
        assert_eq!(host.scan(), found);
    }

    #[test]
    fn load_reports_parameters() {
        let (_dev, r) = ClapDevice::load(Path::new(&gain_path()), "org.trakr.test-gain", 44_100).unwrap();
        assert_eq!(r.name, "Trakr Test Gain");
        assert_eq!(r.params.len(), 2);
        assert_eq!(
            (r.params[0].id, r.params[0].name.as_str(), r.params[0].min, r.params[0].max),
            (7, "Gain", 0.0, 2.0)
        );
        assert_eq!(r.params[0].default, 1.0);
        assert!(r.params[1].stepped && r.params[1].id == 9);
    }

    #[test]
    fn load_errors_are_messages_not_crashes() {
        let ok = gain_path();
        for (path, id, needle) in [
            ("/nonexistent/x.clap".to_string(), "org.trakr.test-gain", "cannot load"),
            (plugin_dir().join("broken.clap").to_string_lossy().into_owned(), "x", "cannot load"),
            (ok.clone(), "no.such.plugin", "is not in"),
        ] {
            let e = ClapDevice::load(Path::new(&path), id, 44_100).err().expect("must fail");
            assert!(e.contains(needle), "{e}");
        }
        assert!(ClapDevice::load(Path::new(&ok), "bad\0id", 44_100).is_err());
    }

    #[test]
    fn processes_audio_and_applies_parameters() {
        let (mut dev, _) = ClapDevice::load(Path::new(&gain_path()), "org.trakr.test-gain", 44_100).unwrap();
        assert_eq!(dev.id(), "clap");
        let src: Vec<f32> = (0..2000).map(|i| ((i / 2) as f32 * 0.01).sin() * 0.8).collect();
        let mut a = src.clone();
        dev.process(&[0.5, 0.0], &mut a);
        for (x, y) in src.iter().zip(&a) {
            assert!((x * 0.5 - y).abs() < 1e-6);
        }
        let mut b = src.clone();
        dev.process(&[1.0, 1.0], &mut b); // muted
        assert!(b.iter().all(|s| *s == 0.0));
        let mut c = src.clone();
        dev.process(&[2.0, 0.0], &mut c);
        assert!((c[10] - src[10] * 2.0).abs() < 1e-6, "parameters apply after mute is released");
        // NaN parameters are ignored, unknown extra values are harmless, and big buffers are chunked
        let big: Vec<f32> = vec![0.25; 2 * 10_000];
        let mut d = big.clone();
        dev.process(&[f32::NAN, 0.0, 9.0, 9.0], &mut d);
        assert!(d.iter().all(|s| (s - 0.5).abs() < 1e-6), "{} {}", d[0], d[19_999]);
    }

    fn song_with_plugin(params: Vec<f32>) -> Song {
        let (_, r) = ClapDevice::load(Path::new(&gain_path()), "org.trakr.test-gain", 1000).unwrap();
        let mut song = Song { bpm: 60.0, lpb: 1, tpl: 4, ..Song::default() };
        let mut inst = Instrument::new("t");
        let mut s = Sample::new("t", 1000, 1, vec![0.8; 5000]);
        s.loop_mode = LoopMode::Off;
        inst.samples.push(s);
        song.instruments.push(inst);
        let n = &mut Arc::make_mut(&mut song.patterns[0]).line_mut(0, 0).notes[0];
        n.note = 48;
        n.instrument = 0;
        let mut spec = DeviceSpec::from_plugin(r);
        spec.params = params;
        song.tracks[0].devices.push(spec);
        song
    }

    fn render(song: Song, frames: usize) -> Vec<f32> {
        let mut host = ClapHost::with_paths(vec![]);
        let (mut engine, mut h) = trakr_engine::create(1000);
        let chain: Vec<Box<dyn Device>> = song.tracks[0]
            .devices
            .iter()
            .map(|d| {
                let p = d.plugin.as_ref().unwrap();
                host.load(&p.path, &p.plugin_id, 1000).unwrap().0
            })
            .collect();
        h.send(Command::SetSong(Arc::new(song)));
        h.send(Command::SetChain { track: 0, chain });
        h.send(Command::Play { seq_pos: 0, line: 0, loop_pattern: false });
        let mut out = vec![0.0; frames * 2];
        for c in out.chunks_mut(2 * 100) {
            engine.render(c);
            h.collect_garbage();
        }
        out.chunks_exact(2).map(|f| f[0]).collect()
    }

    fn peak(v: &[f32]) -> f32 {
        v.iter().fold(0.0, |m, s| s.abs().max(m))
    }

    #[test]
    fn a_hosted_plugin_shapes_a_track_in_the_engine() {
        let loud = peak(&render(song_with_plugin(vec![1.0, 0.0]), 1500));
        let quiet = peak(&render(song_with_plugin(vec![0.25, 0.0]), 1500));
        let muted = peak(&render(song_with_plugin(vec![1.0, 1.0]), 1500));
        assert!(loud > 0.5, "{loud}");
        assert!((quiet / loud - 0.25).abs() < 0.02, "{quiet} vs {loud}");
        assert_eq!(muted, 0.0);
        // a plugin whose chain slot was never filled passes audio through
        let mut song = song_with_plugin(vec![1.0, 0.0]);
        song.tracks[0].devices[0].bypass = true;
        assert!((peak(&render(song, 1500)) - loud).abs() < 0.01);
    }

    #[test]
    fn plugin_parameters_can_be_automated() {
        let mut song = song_with_plugin(vec![1.0, 0.0]);
        // gain (range 0..2) automated from normalized 0 to 0: silence; mute stays off
        let mut a = Automation::new(0, AutoTarget::Device { device: 0, param: 0 });
        a.set_point(0.0, 0.0);
        a.set_point(10.0, 0.0);
        Arc::make_mut(&mut song.patterns[0]).automation.push(a);
        let out = render(song, 1500);
        assert!(peak(&out) < 1e-4, "{}", peak(&out));
    }

    #[test]
    fn missing_plugin_files_fall_back_gracefully_in_the_engine() {
        // The editor substitutes a pass-through when loading fails; the engine skips it.
        let song = song_with_plugin(vec![0.1, 0.0]);
        let (mut engine, mut h) = trakr_engine::create(1000);
        h.send(Command::SetSong(Arc::new(song)));
        h.send(Command::SetChain {
            track: 0,
            chain: vec![trakr_engine::devices::create_or_passthrough("clap:org.trakr.test-gain", 1000)],
        });
        h.send(Command::Play { seq_pos: 0, line: 0, loop_pattern: false });
        let mut out = vec![0.0; 2 * 500];
        engine.render(&mut out);
        assert!(out.iter().any(|s| s.abs() > 0.5), "audio passes through, not silenced");
    }

    fn synth_path() -> String {
        gain_path() // both plugins live in the same library
    }

    const SYNTH: &str = "org.trakr.test-synth";

    #[test]
    fn instruments_load_and_report_themselves() {
        let (dev, r) = ClapDevice::load(Path::new(&synth_path()), SYNTH, 44_100).unwrap();
        assert!(dev.is_instrument());
        assert_eq!(
            (r.name.as_str(), r.params.len(), r.params[0].name.as_str()),
            ("Trakr Test Synth", 1, "Level")
        );
        let (fx, _) = ClapDevice::load(Path::new(&gain_path()), "org.trakr.test-gain", 44_100).unwrap();
        assert!(!fx.is_instrument());
    }

    fn rms(v: &[f32]) -> f32 {
        (v.iter().map(|x| x * x).sum::<f32>() / v.len().max(1) as f32).sqrt()
    }

    #[test]
    fn an_instrument_plays_notes_and_stops_on_note_off() {
        let (mut dev, _) = ClapDevice::load(Path::new(&synth_path()), SYNTH, 44_100).unwrap();
        let mut silent = vec![0.0f32; 2048];
        dev.process(&[0.8], &mut silent);
        assert!(silent.iter().all(|s| *s == 0.0), "no note, no sound");
        dev.note_on(57, 1.0); // internal 57 = A-4 = MIDI 69 = 440 Hz
        let mut a = vec![0.0f32; 8820]; // 0.1 s stereo
        dev.process(&[0.8], &mut a);
        let left: Vec<f32> = a.chunks(2).map(|f| f[0]).collect();
        let crossings = left.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
        assert!(
            (43..=45).contains(&crossings),
            "a 440 Hz tone has about 44 cycles in 0.1 s, got {crossings}"
        );
        assert!(rms(&left) > 0.1);
        assert!(a.chunks(2).all(|f| f[0] == f[1]), "stereo output");
        dev.note_off(57);
        let mut b = vec![0.0f32; 1000];
        dev.process(&[0.8], &mut b);
        assert!(b.iter().all(|s| *s == 0.0), "the note ended");
        // a stuck-down queue is bounded and the level parameter scales the output
        for _ in 0..1000 {
            dev.note_on(60, 0.5);
        }
        dev.process(&[0.8], &mut b);
        let (mut quiet, _) = ClapDevice::load(Path::new(&synth_path()), SYNTH, 44_100).unwrap();
        let (mut loud, _) = ClapDevice::load(Path::new(&synth_path()), SYNTH, 44_100).unwrap();
        quiet.note_on(57, 1.0);
        loud.note_on(57, 1.0);
        let (mut q, mut l) = (vec![0.0f32; 4000], vec![0.0f32; 4000]);
        quiet.process(&[0.2], &mut q);
        loud.process(&[0.8], &mut l);
        assert!((rms(&q) / rms(&l) - 0.25).abs() < 0.02, "{} {}", rms(&q), rms(&l));
    }

    #[test]
    fn effects_ignore_notes() {
        let (mut dev, _) = ClapDevice::load(Path::new(&gain_path()), "org.trakr.test-gain", 44_100).unwrap();
        dev.note_on(60, 1.0);
        let mut buf = vec![0.5f32; 400];
        dev.process(&[1.0, 0.0], &mut buf);
        assert!(buf.iter().all(|s| (s - 0.5).abs() < 1e-6));
    }

    fn instrument_song(vol_param: f32) -> Song {
        let (_, r) = ClapDevice::load(Path::new(&synth_path()), SYNTH, 1000).unwrap();
        let mut song = Song { bpm: 60.0, lpb: 1, tpl: 4, ..Song::default() };
        let mut inst = Instrument::new("synth");
        let mut spec = DeviceSpec::from_plugin(r);
        spec.params = vec![vol_param];
        inst.plugin = Some(spec);
        song.instruments.push(inst);
        let pat = Arc::make_mut(&mut song.patterns[0]);
        let n = &mut pat.line_mut(0, 0).notes[0];
        n.note = 57;
        n.instrument = 0;
        pat.line_mut(0, 2).notes[0].note = trakr_core::NOTE_OFF;
        song
    }

    fn render_instrument(song: Song, frames: usize) -> Vec<f32> {
        let mut host = ClapHost::with_paths(vec![]);
        let (mut engine, mut h) = trakr_engine::create(1000);
        let p = song.instruments[0].plugin.as_ref().unwrap().plugin.clone().unwrap();
        let dev = host.load(&p.path, &p.plugin_id, 1000).unwrap().0;
        h.send(Command::SetSong(Arc::new(song)));
        h.send(Command::SetInstrumentPlugin { instrument: 0, device: Some(dev) });
        h.send(Command::Play { seq_pos: 0, line: 0, loop_pattern: false });
        let mut out = vec![0.0; frames * 2];
        for c in out.chunks_mut(2 * 100) {
            engine.render(c);
            h.collect_garbage();
        }
        out.chunks_exact(2).map(|f| f[0]).collect()
    }

    #[test]
    fn the_sequencer_plays_an_instrument_plugin_and_releases_it() {
        let out = render_instrument(instrument_song(0.8), 4000);
        assert!(peak(&out[100..1900]) > 0.05, "the note sounds: {}", peak(&out[100..1900]));
        assert!(
            peak(&out[2100..4000]) < 1e-6,
            "the note-off at line 2 silences it: {}",
            peak(&out[2100..4000])
        );
        let quiet = render_instrument(instrument_song(0.2), 1500);
        let loud = render_instrument(instrument_song(0.8), 1500);
        assert!(
            (peak(&quiet) / peak(&loud) - 0.25).abs() < 0.03,
            "plugin parameters come from the instrument"
        );
        // stopping releases held notes
        let (mut engine, mut h) = trakr_engine::create(1000);
        let mut host = ClapHost::with_paths(vec![]);
        let song = instrument_song(0.8);
        let p = song.instruments[0].plugin.as_ref().unwrap().plugin.clone().unwrap();
        h.send(Command::SetSong(Arc::new(song)));
        h.send(Command::SetInstrumentPlugin {
            instrument: 0,
            device: Some(host.load(&p.path, &p.plugin_id, 1000).unwrap().0),
        });
        h.send(Command::Play { seq_pos: 0, line: 0, loop_pattern: false });
        let mut buf = vec![0.0; 400];
        engine.render(&mut buf);
        h.send(Command::Stop);
        engine.render(&mut buf);
        engine.render(&mut buf);
        assert!(peak(&buf) < 1e-6, "Stop ends plugin notes");
        // removing the plugin drops the device through the garbage queue
        h.send(Command::SetInstrumentPlugin { instrument: 0, device: None });
        engine.render(&mut buf);
        h.collect_garbage();
    }

    #[test]
    fn live_notes_reach_the_instrument_on_their_track() {
        let (mut engine, mut h) = trakr_engine::create(1000);
        let mut host = ClapHost::with_paths(vec![]);
        let mut song = instrument_song(0.8);
        song.patterns = vec![Arc::new(trakr_core::Pattern::new(8, 1))];
        let p = song.instruments[0].plugin.as_ref().unwrap().plugin.clone().unwrap();
        h.send(Command::SetSong(Arc::new(song)));
        h.send(Command::SetInstrumentPlugin {
            instrument: 0,
            device: Some(host.load(&p.path, &p.plugin_id, 1000).unwrap().0),
        });
        h.send(Command::NoteOn { track: 0, instrument: 0, note: 57, volume: 1.0 });
        let mut buf = vec![0.0; 800];
        engine.render(&mut buf);
        assert!(peak(&buf[200..]) > 0.05, "a live note sounds");
        h.send(Command::NoteOff { track: 0, note: 57 });
        engine.render(&mut buf);
        engine.render(&mut buf);
        assert!(peak(&buf) < 1e-6);
    }

    /// Read one of the test plugin's call counters (the library is already loaded, so this is the same copy).
    fn counter(i: u32) -> u32 {
        // Keep one handle open so the library (and its counters) is never unloaded between calls.
        static LIB: OnceLock<libloading::Library> = OnceLock::new();
        let lib = LIB.get_or_init(|| unsafe { libloading::Library::new(gain_path()).unwrap() });
        unsafe {
            let f: libloading::Symbol<unsafe extern "C" fn(u32) -> u32> =
                lib.get(b"trakr_test_counter").unwrap();
            f(i)
        }
    }

    #[test]
    fn a_plugin_window_opens_runs_timers_and_reports_changes() {
        let mut host = ClapHost::with_paths(vec![]);
        let (dev, _, gui) = host.load_with_gui(&synth_path(), SYNTH, 44_100, &[]).unwrap();
        let mut gui = gui.expect("the host hands out a UI handle");
        let mut dev = dev;
        assert!(gui.has_window());
        assert!(!gui.is_visible());
        let before = (counter(0), counter(1), counter(2), counter(3), counter(4));
        gui.show().unwrap();
        assert_eq!((counter(0), counter(1)), (before.0 + 1, before.1 + 1), "created and shown once");
        assert!(gui.is_visible());
        gui.show().unwrap(); // showing again raises, it does not create a second window
        assert_eq!(counter(0), before.0 + 1);
        // the plugin said its state changed and moved a parameter itself
        assert!(gui.take_state_dirty());
        assert!(!gui.take_state_dirty());
        let mut buf = vec![0.0f32; 512];
        dev.process(&[], &mut buf);
        let mut changes = Vec::new();
        gui.take_param_changes(&mut changes);
        assert_eq!(changes.len(), 1, "{changes:?}");
        assert_eq!(changes[0].0, 0);
        assert!((changes[0].1 - 0.33).abs() < 1e-6);
        gui.take_param_changes(&mut changes);
        assert_eq!(changes.len(), 1, "changes are reported once");
        // main-thread timers run from poll()
        std::thread::sleep(std::time::Duration::from_millis(60));
        host.poll();
        assert!(counter(4) > before.4, "the 20 ms timer fired");
        gui.hide();
        assert_eq!(counter(2), before.2 + 1);
        assert!(!gui.is_visible());
        drop(gui);
        drop(dev);
        assert_eq!(counter(3), before.3 + 1, "the window is destroyed with the plugin");
    }

    #[test]
    fn plugin_state_is_saved_and_restored() {
        let mut host = ClapHost::with_paths(vec![]);
        let (mut fresh, _, gui) = host.load_with_gui(&synth_path(), SYNTH, 44_100, &[]).unwrap();
        let mut gui = gui.unwrap();
        let saved = gui.save_state().expect("the plugin saves its state");
        assert_eq!(f64::from_le_bytes(saved[..8].try_into().unwrap()), 0.8);
        let level = |dev: &mut Box<dyn Device>| {
            dev.note_on(57, 1.0);
            let mut b = vec![0.0f32; 4000];
            dev.process(&[], &mut b); // no parameter events, so the plugin's own state decides
            rms(&b)
        };
        let base = level(&mut fresh);
        let custom = 0.2f64.to_le_bytes();
        let (mut restored, _, _) = host.load_with_gui(&synth_path(), SYNTH, 44_100, &custom).unwrap();
        let low = level(&mut restored);
        assert!((low / base - 0.25).abs() < 0.02, "restored level 0.2 vs 0.8: {low} / {base}");
        // garbage state is refused by the plugin without harming the load
        assert!(host.load_with_gui(&synth_path(), SYNTH, 44_100, &[1, 2, 3]).is_ok());
    }
}
