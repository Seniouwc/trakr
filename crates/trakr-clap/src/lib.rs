//! Hosts local CLAP effect plugins (`.clap` shared libraries).
//!
//! Scope: audio effects with a mono or stereo main input and output. Parameters are exposed through the
//! plugin's `clap.params` extension and set with sample-accurate-enough parameter events at the start of
//! each block. Not supported yet: instruments/note ports, plugin GUIs, state save/restore, latency
//! reporting, multiple audio ports, 64-bit buffers, hosting extensions beyond the minimum.
//!
//! A plugin is native code running in this process: a crashing plugin takes trakr down with it.

use clap_sys::audio_buffer::clap_audio_buffer;
use clap_sys::entry::clap_plugin_entry;
use clap_sys::events::{
    clap_event_header, clap_event_param_value, clap_input_events, clap_output_events,
    CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_PARAM_VALUE,
};
use clap_sys::ext::audio_ports::{clap_audio_port_info, clap_plugin_audio_ports, CLAP_EXT_AUDIO_PORTS};
use clap_sys::ext::params::{
    clap_param_info, clap_plugin_params, CLAP_EXT_PARAMS, CLAP_PARAM_IS_HIDDEN, CLAP_PARAM_IS_READONLY,
    CLAP_PARAM_IS_STEPPED,
};
use clap_sys::factory::plugin_factory::{clap_plugin_factory, CLAP_PLUGIN_FACTORY_ID};
use clap_sys::host::clap_host;
use clap_sys::plugin::clap_plugin;
use clap_sys::process::{clap_process, CLAP_PROCESS_ERROR};
use clap_sys::version::CLAP_VERSION;
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use trakr_core::{PluginInfo, PluginParam, PluginRef, MAX_PLUGIN_PARAMS};
use trakr_engine::{Device, PluginHost};

/// Most frames handed to a plugin in one call.
const MAX_FRAMES: usize = 4096;

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
}

impl ClapHost {
    pub fn new() -> Self {
        Self { dirs: default_search_paths() }
    }

    pub fn with_paths(dirs: Vec<PathBuf>) -> Self {
        Self { dirs }
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
        let (dev, r) = ClapDevice::load(Path::new(path), plugin_id, sample_rate)?;
        Ok((Box::new(dev), r))
    }
}

// ---- host callbacks ----------------------------------------------------------------------------

unsafe extern "C" fn host_get_extension(_: *const clap_host, _: *const c_char) -> *const c_void {
    std::ptr::null()
}
unsafe extern "C" fn host_noop(_: *const clap_host) {}

/// Parameter events handed to the plugin for one block.
struct EventList {
    events: Vec<clap_event_param_value>,
}

unsafe extern "C" fn events_size(list: *const clap_input_events) -> u32 {
    let l = &*((*list).ctx as *const EventList);
    l.events.len() as u32
}

unsafe extern "C" fn events_get(list: *const clap_input_events, index: u32) -> *const clap_event_header {
    let l = &*((*list).ctx as *const EventList);
    match l.events.get(index as usize) {
        Some(e) => &e.header,
        None => std::ptr::null(),
    }
}

unsafe extern "C" fn out_events_push(_: *const clap_output_events, _: *const clap_event_header) -> bool {
    true
}

/// A plugin instance running as a track effect.
pub struct ClapDevice {
    plugin: *const clap_plugin,
    // Keeps the library loaded for as long as the plugin exists; dropped after `destroy`.
    _lib: Arc<Library>,
    // The plugin keeps a pointer to this; it must not move.
    _host: Box<clap_host>,
    ids: Vec<u32>,
    last: Vec<f32>,
    primed: bool,
    in_ch: u32,
    out_ch: u32,
    bufs: [Vec<f32>; 4],
    list: Box<EventList>,
    steady: i64,
}

// The plugin is created on the UI thread and then used from the audio thread only; the engine never
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
        let lib = Library::open(path)?;
        let factory = lib.factory()?;
        let id = CString::new(plugin_id).map_err(|_| "bad plugin id".to_string())?;
        let host = Box::new(clap_host {
            clap_version: CLAP_VERSION,
            host_data: std::ptr::null_mut(),
            name: c"trakr".as_ptr(),
            vendor: c"trakr".as_ptr(),
            url: c"".as_ptr(),
            version: c"0.0.1".as_ptr(),
            get_extension: Some(host_get_extension),
            request_restart: Some(host_noop),
            request_process: Some(host_noop),
            request_callback: Some(host_noop),
        });
        unsafe {
            let create = factory.create_plugin.ok_or("no create_plugin")?;
            let plugin = create(factory, &*host, id.as_ptr());
            let p = plugin.as_ref().ok_or_else(|| format!("{plugin_id} is not in {}", path.display()))?;
            if !p.init.ok_or("no init")?(plugin) {
                (p.destroy.ok_or("no destroy")?)(plugin);
                return Err("plugin failed to initialise".into());
            }
            let fail = |msg: &str| -> Result<(ClapDevice, PluginRef), String> {
                (p.destroy.unwrap())(plugin);
                Err(msg.to_string())
            };
            let get_ext = p.get_extension.ok_or("no get_extension")?;
            let ports =
                (get_ext(plugin, CLAP_EXT_AUDIO_PORTS.as_ptr()) as *const clap_plugin_audio_ports).as_ref();
            let Some(ports) = ports else { return fail("not an audio effect (no audio ports)") };
            let Some(in_ch) = port_channels(p, ports, true) else {
                return fail("instrument plugins are not supported yet (effects need an audio input)");
            };
            let Some(out_ch) = port_channels(p, ports, false) else {
                return fail("plugin has no audio output");
            };
            if !(1..=2).contains(&in_ch) || !(1..=2).contains(&out_ch) {
                return fail("only mono and stereo effects are supported");
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

            let activate = p.activate.ok_or("no activate")?;
            if !activate(plugin, sample_rate as f64, 1, MAX_FRAMES as u32) {
                return fail("plugin could not be activated");
            }
            if let Some(start) = p.start_processing {
                if !start(plugin) {
                    (p.deactivate.unwrap())(plugin);
                    return fail("plugin refused to start processing");
                }
            }
            let name = cstr(p.desc.as_ref().map_or(std::ptr::null(), |d| d.name));
            let n = ids.len();
            let device = ClapDevice {
                plugin,
                _lib: lib,
                _host: host,
                ids,
                last: vec![f32::NAN; n],
                primed: false,
                in_ch,
                out_ch,
                bufs: std::array::from_fn(|_| vec![0.0; MAX_FRAMES]),
                list: Box::new(EventList { events: Vec::with_capacity(MAX_PLUGIN_PARAMS) }),
                steady: 0,
            };
            let reference = PluginRef {
                path: path.to_string_lossy().into_owned(),
                plugin_id: plugin_id.into(),
                name,
                params,
            };
            Ok((device, reference))
        }
    }
}

impl Device for ClapDevice {
    fn id(&self) -> &'static str {
        "clap"
    }

    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let total = buf.len() / 2;
        let mut done = 0;
        while done < total {
            let n = (total - done).min(MAX_FRAMES);
            let frame = &mut buf[done * 2..(done + n) * 2];

            // changed parameters become events at the start of the block
            self.list.events.clear();
            for (i, id) in self.ids.iter().enumerate().take(params.len()) {
                let v = params[i];
                if !v.is_finite() || (self.primed && self.last[i] == v) {
                    continue;
                }
                self.last[i] = v;
                self.list.events.push(clap_event_param_value {
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
                });
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
            let out_events =
                clap_output_events { ctx: std::ptr::null_mut(), try_push: Some(out_events_push) };
            let process = clap_process {
                steady_time: self.steady,
                frames_count: n as u32,
                transport: std::ptr::null(),
                audio_inputs: &input,
                audio_outputs: &mut output,
                audio_inputs_count: 1,
                audio_outputs_count: 1,
                in_events: &in_events,
                out_events: &out_events,
            };
            self.steady += n as i64;
            let status = unsafe {
                match (*self.plugin).process {
                    Some(f) => f(self.plugin, &process),
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

impl Drop for ClapDevice {
    fn drop(&mut self) {
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
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].plugin_id, "org.trakr.test-gain");
        assert_eq!(found[0].name, "Trakr Test Gain");
        assert_eq!(found[0].vendor, "trakr");
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
}
