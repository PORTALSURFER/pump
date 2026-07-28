//! Pump: node-based beat-synced gain shaping.

#![warn(missing_docs)]

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use toybox::clack_extensions::audio_ports::*;
#[cfg(all(target_os = "macos", feature = "radiant-gui"))]
use toybox::clack_extensions::gui::PluginGui;
use toybox::clack_extensions::params::*;
use toybox::clack_extensions::state::{PluginState, PluginStateImpl};
use toybox::clack_plugin;
use toybox::clack_plugin::events::spaces::CoreEventSpace;
use toybox::clack_plugin::prelude::*;
use toybox::clack_plugin::stream::{InputStream, OutputStream};
use toybox::clap::automation::AutomationEvent;
use toybox::clap::prelude::apply_param_events;
use toybox::clap::process::{min_len, split_channel};
use toybox::clap::state::{read_versioned_payload, write_versioned_payload};
use toybox::clap::transport::transport_state_from_transport;
use toybox::dsp::{AtomicF32, TransportState};

use crate::automation_queue::PumpAutomationQueue;
use crate::dsp::{DspSettings, PumpEngine};
#[cfg(all(target_os = "macos", feature = "radiant-gui"))]
use crate::gui::HostParamFlushRequester;
#[cfg(all(target_os = "macos", feature = "radiant-gui"))]
use crate::gui::RadiantPumpEditor;
use crate::gui::{
    MAX_WINDOW_HEIGHT, MAX_WINDOW_WIDTH, MIN_WINDOW_HEIGHT, MIN_WINDOW_WIDTH, WINDOW_HEIGHT,
    WINDOW_WIDTH,
};
use crate::params::{
    apply_param_event, decode_state_payload, encode_state_payload, get_param_value, param_count,
    text_to_value, value_to_text, write_param_info, PumpParams,
};
use crate::plugin_metadata::{PLUGIN_ID, PLUGIN_NAME, VENDOR_NAME};
use crate::sample_automation::{
    dsp_settings_from_params, process_stereo_block, ParamEventSchedule,
};
use crate::time_utils::monotonic_micros;

mod automation_queue;
#[cfg(test)]
mod build_support;
mod curve;
mod curve_presets;
mod dsp;
mod gui;
mod gui_status;
mod incoming_waveform;
mod params;
mod plugin_main_thread_impl;
mod plugin_metadata;
mod plugin_processor;
mod sample_automation;
mod time_utils;
mod transport;
#[cfg(feature = "vst3")]
mod vst3;

#[cfg(test)]
mod test_alloc {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    thread_local! {
        static TRACKING: Cell<bool> = const { Cell::new(false) };
        static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
    }

    struct TrackingAllocator;

    #[global_allocator]
    static GLOBAL_ALLOCATOR: TrackingAllocator = TrackingAllocator;

    unsafe impl GlobalAlloc for TrackingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            record_allocation();
            unsafe { System.alloc(layout) }
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            record_allocation();
            unsafe { System.alloc_zeroed(layout) }
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            record_allocation();
            unsafe { System.realloc(ptr, layout, new_size) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            unsafe { System.dealloc(ptr, layout) }
        }
    }

    fn record_allocation() {
        TRACKING.with(|tracking| {
            if tracking.get() {
                ALLOCATIONS.with(|allocations| allocations.set(allocations.get() + 1));
            }
        });
    }

    struct TrackingGuard;

    impl Drop for TrackingGuard {
        fn drop(&mut self) {
            TRACKING.with(|tracking| tracking.set(false));
        }
    }

    pub(crate) fn assert_no_alloc<T>(operation: impl FnOnce() -> T) -> T {
        ALLOCATIONS.with(|allocations| allocations.set(0));
        TRACKING.with(|tracking| tracking.set(true));
        let guard = TrackingGuard;
        let result = operation();
        drop(guard);
        let allocations = ALLOCATIONS.with(Cell::get);
        assert_eq!(
            allocations, 0,
            "realtime callback allocated {allocations} times"
        );
        result
    }
}

/// Versioned state payload magic (`PUMP`).
const STATE_MAGIC: u32 = u32::from_le_bytes(*b"PUMP");
/// Versioned state payload format version.
const STATE_VERSION: u32 = 1;

/// CLAP plugin type for Pump.
pub struct PumpPlugin;

impl Plugin for PumpPlugin {
    type AudioProcessor<'a> = PumpAudioProcessor<'a>;
    type Shared<'a> = PumpShared;
    type MainThread<'a> = PumpMainThread<'a>;

    fn declare_extensions(builder: &mut PluginExtensions<Self>, _shared: Option<&PumpShared>) {
        builder
            .register::<PluginAudioPorts>()
            .register::<PluginParams>()
            .register::<PluginState>();
        #[cfg(all(target_os = "macos", feature = "radiant-gui"))]
        builder.register::<PluginGui>();
    }
}

impl DefaultPluginFactory for PumpPlugin {
    fn get_descriptor() -> PluginDescriptor {
        use clack_plugin::plugin::features::*;

        PluginDescriptor::new(PLUGIN_ID, PLUGIN_NAME)
            .with_vendor(VENDOR_NAME)
            .with_features([AUDIO_EFFECT, STEREO])
            .with_description("Node-based beat-synced gain ducking effect")
    }

    fn new_shared(_host: HostSharedHandle<'_>) -> Result<Self::Shared<'_>, PluginError> {
        Ok(PumpShared {
            params: Arc::new(PumpParams::new()),
            status: Arc::new(GuiStatus::default()),
            automation_queue: Arc::new(PumpAutomationQueue::default()),
        })
    }

    fn new_main_thread<'a>(
        host: HostMainThreadHandle<'a>,
        shared: &'a Self::Shared<'a>,
    ) -> Result<Self::MainThread<'a>, PluginError> {
        let host_shared = host.shared();
        Ok(PumpMainThread {
            shared,
            #[cfg(all(target_os = "macos", feature = "radiant-gui"))]
            gui: toybox::radiant_gui::RadiantHostedGui::new(
                "PumpRadiantClapEditorView",
                RadiantPumpEditor::new(
                    Arc::clone(&shared.params),
                    Arc::clone(&shared.status),
                    Arc::clone(&shared.automation_queue),
                    HostParamFlushRequester::new(host_shared),
                    WINDOW_WIDTH,
                    WINDOW_HEIGHT,
                ),
                WINDOW_WIDTH,
                WINDOW_HEIGHT,
            )
            .with_size_contract(
                (MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT),
                (WINDOW_WIDTH, WINDOW_HEIGHT),
                (MAX_WINDOW_WIDTH, MAX_WINDOW_HEIGHT),
            ),
            automation_drain: Vec::new(),
        })
    }
}

/// Shared thread-safe plugin resources.
pub struct PumpShared {
    /// Shared parameter and curve state.
    pub params: Arc<PumpParams>,
    /// Real-time telemetry exposed to GUI.
    pub status: Arc<GuiStatus>,
    /// Pending GUI automation events destined for the host.
    pub automation_queue: Arc<PumpAutomationQueue>,
}

impl PluginShared<'_> for PumpShared {}

/// Main-thread state for parameters, GUI, and state I/O.
pub struct PumpMainThread<'a> {
    /// Shared plugin resources.
    shared: &'a PumpShared,
    /// Host-parented editor wrapper.
    #[cfg(all(target_os = "macos", feature = "radiant-gui"))]
    gui: toybox::radiant_gui::RadiantHostedGui,
    /// Scratch vector for draining queued automation events.
    automation_drain: Vec<AutomationEvent>,
}

pub use gui_status::{GuiStatus, GuiTransportTelemetry};
use plugin_processor::PumpAudioProcessor;
use transport::{gui_phase_from_transport, gui_transport_telemetry};

toybox::clap_plugin_entry!(PumpPlugin);
