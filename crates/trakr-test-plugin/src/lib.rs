//! A minimal CLAP effect for testing the host: stereo in, stereo out, two parameters (gain, mute).

use clap_sys::audio_buffer::clap_audio_buffer;
use clap_sys::entry::clap_plugin_entry;
use clap_sys::events::{clap_event_param_value, CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_PARAM_VALUE};
use clap_sys::ext::audio_ports::{
    clap_audio_port_info, clap_plugin_audio_ports, CLAP_AUDIO_PORT_IS_MAIN, CLAP_EXT_AUDIO_PORTS,
    CLAP_PORT_STEREO,
};
use clap_sys::ext::params::{
    clap_param_info, clap_plugin_params, CLAP_EXT_PARAMS, CLAP_PARAM_IS_AUTOMATABLE, CLAP_PARAM_IS_STEPPED,
};
use clap_sys::factory::plugin_factory::{clap_plugin_factory, CLAP_PLUGIN_FACTORY_ID};
use clap_sys::host::clap_host;
use clap_sys::id::clap_id;
use clap_sys::plugin::{clap_plugin, clap_plugin_descriptor};
use clap_sys::process::{clap_process, CLAP_PROCESS_CONTINUE, CLAP_PROCESS_ERROR};
use clap_sys::version::CLAP_VERSION;
use std::ffi::{c_char, c_void, CStr};
use std::ptr;

const PLUGIN_ID: &CStr = c"org.trakr.test-gain";
const GAIN: clap_id = 7;
const MUTE: clap_id = 9;

#[allow(dead_code)]
struct Feature(*const c_char);
unsafe impl Sync for Feature {}

static FEATURES: [Feature; 3] =
    [Feature(c"audio-effect".as_ptr()), Feature(c"utility".as_ptr()), Feature(ptr::null())];

struct Descriptor(clap_plugin_descriptor);
unsafe impl Sync for Descriptor {}

static DESCRIPTOR: Descriptor = Descriptor(clap_plugin_descriptor {
    clap_version: CLAP_VERSION,
    id: PLUGIN_ID.as_ptr(),
    name: c"Trakr Test Gain".as_ptr(),
    vendor: c"trakr".as_ptr(),
    url: c"".as_ptr(),
    manual_url: c"".as_ptr(),
    support_url: c"".as_ptr(),
    version: c"1.0".as_ptr(),
    description: c"Gain and mute, for testing".as_ptr(),
    features: FEATURES.as_ptr() as *const *const c_char,
});

struct State {
    gain: f64,
    mute: f64,
}

unsafe extern "C" fn init(_: *const clap_plugin) -> bool {
    true
}

unsafe extern "C" fn destroy(plugin: *const clap_plugin) {
    drop(Box::from_raw((*plugin).plugin_data as *mut State));
    drop(Box::from_raw(plugin as *mut clap_plugin));
}

unsafe extern "C" fn activate(_: *const clap_plugin, _: f64, _: u32, _: u32) -> bool {
    true
}

unsafe extern "C" fn deactivate(_: *const clap_plugin) {}

unsafe extern "C" fn start_processing(_: *const clap_plugin) -> bool {
    true
}

unsafe extern "C" fn stop_processing(_: *const clap_plugin) {}

unsafe extern "C" fn reset(_: *const clap_plugin) {}

unsafe extern "C" fn on_main_thread(_: *const clap_plugin) {}

unsafe extern "C" fn process(plugin: *const clap_plugin, p: *const clap_process) -> i32 {
    let st = &mut *((*plugin).plugin_data as *mut State);
    let p = &*p;
    if p.audio_inputs_count == 0 || p.audio_outputs_count == 0 {
        return CLAP_PROCESS_ERROR;
    }
    if !p.in_events.is_null() {
        let ev = &*p.in_events;
        let (size, get) = (ev.size.unwrap(), ev.get.unwrap());
        for i in 0..size(ev) {
            let h = &*get(ev, i);
            if h.space_id == CLAP_CORE_EVENT_SPACE_ID && h.type_ == CLAP_EVENT_PARAM_VALUE {
                let e = &*(h as *const _ as *const clap_event_param_value);
                match e.param_id {
                    GAIN => st.gain = e.value,
                    MUTE => st.mute = e.value,
                    _ => {}
                }
            }
        }
    }
    let input: &clap_audio_buffer = &*p.audio_inputs;
    let output: &clap_audio_buffer = &*p.audio_outputs;
    let g = (st.gain * (1.0 - st.mute)) as f32;
    for ch in 0..output.channel_count.min(input.channel_count) as usize {
        let src = *input.data32.add(ch);
        let dst = *output.data32.add(ch);
        for i in 0..p.frames_count as usize {
            *dst.add(i) = *src.add(i) * g;
        }
    }
    CLAP_PROCESS_CONTINUE
}

unsafe extern "C" fn ports_count(_: *const clap_plugin, _is_input: bool) -> u32 {
    1
}

unsafe extern "C" fn ports_get(
    _: *const clap_plugin,
    index: u32,
    _is_input: bool,
    info: *mut clap_audio_port_info,
) -> bool {
    if index != 0 {
        return false;
    }
    let i = &mut *info;
    i.id = 0;
    i.name = [0; 256];
    for (d, s) in i.name.iter_mut().zip(b"main") {
        *d = *s as c_char;
    }
    i.flags = CLAP_AUDIO_PORT_IS_MAIN;
    i.channel_count = 2;
    i.port_type = CLAP_PORT_STEREO.as_ptr();
    i.in_place_pair = u32::MAX;
    true
}

static AUDIO_PORTS: clap_plugin_audio_ports =
    clap_plugin_audio_ports { count: Some(ports_count), get: Some(ports_get) };

unsafe extern "C" fn params_count(_: *const clap_plugin) -> u32 {
    2
}

unsafe extern "C" fn params_info(_: *const clap_plugin, index: u32, info: *mut clap_param_info) -> bool {
    let i = &mut *info;
    let (id, name, min, max, def, flags) = match index {
        0 => (GAIN, "Gain", 0.0, 2.0, 1.0, CLAP_PARAM_IS_AUTOMATABLE),
        1 => (MUTE, "Mute", 0.0, 1.0, 0.0, CLAP_PARAM_IS_AUTOMATABLE | CLAP_PARAM_IS_STEPPED),
        _ => return false,
    };
    i.id = id;
    i.flags = flags;
    i.cookie = ptr::null_mut();
    i.name = [0; 256];
    for (d, s) in i.name.iter_mut().zip(name.bytes()) {
        *d = s as c_char;
    }
    i.module = [0; 1024];
    i.min_value = min;
    i.max_value = max;
    i.default_value = def;
    true
}

unsafe extern "C" fn params_get_value(plugin: *const clap_plugin, id: clap_id, out: *mut f64) -> bool {
    let st = &*((*plugin).plugin_data as *const State);
    match id {
        GAIN => *out = st.gain,
        MUTE => *out = st.mute,
        _ => return false,
    }
    true
}

unsafe extern "C" fn params_flush(
    _: *const clap_plugin,
    _: *const clap_sys::events::clap_input_events,
    _: *const clap_sys::events::clap_output_events,
) {
}

static PARAMS: clap_plugin_params = clap_plugin_params {
    count: Some(params_count),
    get_info: Some(params_info),
    get_value: Some(params_get_value),
    value_to_text: None,
    text_to_value: None,
    flush: Some(params_flush),
};

unsafe extern "C" fn get_extension(_: *const clap_plugin, id: *const c_char) -> *const c_void {
    let id = CStr::from_ptr(id);
    if id == CLAP_EXT_AUDIO_PORTS {
        &AUDIO_PORTS as *const _ as *const c_void
    } else if id == CLAP_EXT_PARAMS {
        &PARAMS as *const _ as *const c_void
    } else {
        ptr::null()
    }
}

unsafe extern "C" fn factory_count(_: *const clap_plugin_factory) -> u32 {
    1
}

unsafe extern "C" fn factory_descriptor(
    _: *const clap_plugin_factory,
    index: u32,
) -> *const clap_plugin_descriptor {
    if index == 0 {
        &DESCRIPTOR.0
    } else {
        ptr::null()
    }
}

unsafe extern "C" fn factory_create(
    _: *const clap_plugin_factory,
    _host: *const clap_host,
    id: *const c_char,
) -> *const clap_plugin {
    if CStr::from_ptr(id) != PLUGIN_ID {
        return ptr::null();
    }
    let state = Box::into_raw(Box::new(State { gain: 1.0, mute: 0.0 }));
    Box::into_raw(Box::new(clap_plugin {
        desc: &DESCRIPTOR.0,
        plugin_data: state as *mut c_void,
        init: Some(init),
        destroy: Some(destroy),
        activate: Some(activate),
        deactivate: Some(deactivate),
        start_processing: Some(start_processing),
        stop_processing: Some(stop_processing),
        reset: Some(reset),
        process: Some(process),
        get_extension: Some(get_extension),
        on_main_thread: Some(on_main_thread),
    }))
}

static FACTORY: clap_plugin_factory = clap_plugin_factory {
    get_plugin_count: Some(factory_count),
    get_plugin_descriptor: Some(factory_descriptor),
    create_plugin: Some(factory_create),
};

unsafe extern "C" fn entry_init(_: *const c_char) -> bool {
    true
}

unsafe extern "C" fn entry_deinit() {}

unsafe extern "C" fn entry_get_factory(id: *const c_char) -> *const c_void {
    if CStr::from_ptr(id) == CLAP_PLUGIN_FACTORY_ID {
        &FACTORY as *const _ as *const c_void
    } else {
        ptr::null()
    }
}

#[no_mangle]
#[used]
pub static clap_entry: clap_plugin_entry = clap_plugin_entry {
    clap_version: CLAP_VERSION,
    init: Some(entry_init),
    deinit: Some(entry_deinit),
    get_factory: Some(entry_get_factory),
};
