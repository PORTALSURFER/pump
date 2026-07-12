//! VST3 processor implementation for Pump.
//!
//! This module owns the real-time audio processing side of the VST3 adapter.
//! Splitting it from `vst3.rs` keeps adapter wiring and processor behavior
//! isolated so host-facing changes and DSP changes are easier to review.

use super::*;
use std::cell::UnsafeCell;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) struct RealtimeRuntime {
    inner: UnsafeCell<PumpVst3Runtime>,
    in_process: AtomicBool,
}

impl RealtimeRuntime {
    fn new(runtime: PumpVst3Runtime) -> Self {
        Self {
            inner: UnsafeCell::new(runtime),
            in_process: AtomicBool::new(false),
        }
    }

    pub(super) fn try_acquire(&self) -> Option<RealtimeRuntimeGuard<'_>> {
        self.in_process
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()
            .map(|_| RealtimeRuntimeGuard { owner: self })
    }
}

// SAFETY: `inner` is only exposed through `RealtimeRuntimeGuard`, and the
// atomic guard permits at most one mutable borrower. Setup/state callbacks do
// not access `inner`; they publish fixed-size messages through `RuntimeHandoff`.
unsafe impl Sync for RealtimeRuntime {}

pub(super) struct RealtimeRuntimeGuard<'a> {
    owner: &'a RealtimeRuntime,
}

impl Deref for RealtimeRuntimeGuard<'_> {
    type Target = PumpVst3Runtime;

    fn deref(&self) -> &Self::Target {
        // SAFETY: successful guard acquisition gives this guard exclusive
        // access until `Drop` clears `in_process`.
        unsafe { &*self.owner.inner.get() }
    }
}

impl DerefMut for RealtimeRuntimeGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: see the `Deref` implementation above.
        unsafe { &mut *self.owner.inner.get() }
    }
}

impl Drop for RealtimeRuntimeGuard<'_> {
    fn drop(&mut self) {
        self.owner.in_process.store(false, Ordering::Release);
    }
}

pub(super) struct PumpVst3Processor {
    shared: Arc<PumpVst3Shared>,
    pub(super) runtime: RealtimeRuntime,
    pub(super) runtime_handoff: RuntimeHandoff,
}

impl PumpVst3Processor {
    pub(super) fn new(shared: Arc<PumpVst3Shared>) -> Self {
        Self {
            runtime: RealtimeRuntime::new(PumpVst3Runtime::new(shared.params.as_ref())),
            runtime_handoff: RuntimeHandoff::new(),
            shared,
        }
    }
}

impl Drop for PumpVst3Processor {
    fn drop(&mut self) {
        release_shared_for_role(&self.shared, SharedRole::Processor);
    }
}

impl Class for PumpVst3Processor {
    type Interfaces = (IComponent, IAudioProcessor, IProcessContextRequirements);
}

impl IPluginBaseTrait for PumpVst3Processor {
    unsafe fn initialize(&self, _context: *mut FUnknown) -> tresult {
        kResultOk
    }

    unsafe fn terminate(&self) -> tresult {
        kResultOk
    }
}

impl IComponentTrait for PumpVst3Processor {
    unsafe fn getControllerClassId(&self, class_id: *mut TUID) -> tresult {
        if class_id.is_null() {
            return kInvalidArgument;
        }
        unsafe { *class_id = CONTROLLER_CID };
        kResultOk
    }

    unsafe fn setIoMode(&self, _mode: IoMode) -> tresult {
        kResultOk
    }

    unsafe fn getBusCount(&self, media_type: MediaType, dir: BusDirection) -> i32 {
        match media_type as MediaTypes {
            MediaTypes_::kAudio => match dir as BusDirections {
                BusDirections_::kInput | BusDirections_::kOutput => 1,
                _ => 0,
            },
            _ => 0,
        }
    }

    #[allow(clippy::unnecessary_cast)]
    unsafe fn getBusInfo(
        &self,
        media_type: MediaType,
        dir: BusDirection,
        index: i32,
        bus: *mut BusInfo,
    ) -> tresult {
        if bus.is_null() || index != 0 {
            return kInvalidArgument;
        }
        if media_type as MediaTypes != MediaTypes_::kAudio {
            return kInvalidArgument;
        }

        let label = match dir as BusDirections {
            BusDirections_::kInput => "Input",
            BusDirections_::kOutput => "Output",
            _ => return kInvalidArgument,
        };

        let bus = unsafe { &mut *bus };
        bus.mediaType = MediaTypes_::kAudio as MediaType;
        bus.direction = dir;
        bus.channelCount = 2;
        copy_wstring(label, &mut bus.name);
        bus.busType = BusTypes_::kMain as BusType;
        bus.flags = {
            #[cfg(windows)]
            {
                BusInfo_::BusFlags_::kDefaultActive as u32
            }
            #[cfg(not(windows))]
            {
                BusInfo_::BusFlags_::kDefaultActive as u32
            }
        };

        kResultOk
    }

    unsafe fn getRoutingInfo(
        &self,
        _in_info: *mut RoutingInfo,
        _out_info: *mut RoutingInfo,
    ) -> tresult {
        kNotImplemented
    }

    unsafe fn activateBus(
        &self,
        _media_type: MediaType,
        _dir: BusDirection,
        _index: i32,
        _state: TBool,
    ) -> tresult {
        kResultOk
    }

    unsafe fn setActive(&self, _state: TBool) -> tresult {
        kResultOk
    }

    unsafe fn setState(&self, state: *mut IBStream) -> tresult {
        let payload = unsafe { read_versioned_payload(state, STATE_MAGIC, &[STATE_VERSION]) };
        let Ok(payload) = payload else {
            return kInvalidArgument;
        };

        if decode_state_payload(self.shared.params.as_ref(), &payload.payload).is_err() {
            return kInvalidArgument;
        }

        self.runtime_handoff.publish_state_restore();

        kResultOk
    }

    unsafe fn getState(&self, state: *mut IBStream) -> tresult {
        let payload = encode_state_payload(self.shared.params.as_ref());
        match unsafe { write_versioned_payload(state, STATE_MAGIC, STATE_VERSION, &payload) } {
            Ok(()) => kResultOk,
            Err(_) => kResultFalse,
        }
    }
}

impl IAudioProcessorTrait for PumpVst3Processor {
    unsafe fn setBusArrangements(
        &self,
        inputs: *mut SpeakerArrangement,
        num_ins: i32,
        outputs: *mut SpeakerArrangement,
        num_outs: i32,
    ) -> tresult {
        if num_ins != 1 || num_outs != 1 {
            return kResultFalse;
        }
        if inputs.is_null() || outputs.is_null() {
            return kInvalidArgument;
        }

        if unsafe { *inputs } != SpeakerArr::kStereo || unsafe { *outputs } != SpeakerArr::kStereo {
            return kResultFalse;
        }

        kResultTrue
    }

    unsafe fn getBusArrangement(
        &self,
        dir: BusDirection,
        index: i32,
        arr: *mut SpeakerArrangement,
    ) -> tresult {
        if arr.is_null() || index != 0 {
            return kInvalidArgument;
        }

        match dir as BusDirections {
            BusDirections_::kInput | BusDirections_::kOutput => {
                unsafe { *arr = SpeakerArr::kStereo };
                kResultOk
            }
            _ => kInvalidArgument,
        }
    }

    unsafe fn canProcessSampleSize(&self, symbolic_sample_size: i32) -> tresult {
        match symbolic_sample_size as SymbolicSampleSizes {
            SymbolicSampleSizes_::kSample32 => kResultOk,
            SymbolicSampleSizes_::kSample64 => kNotImplemented,
            _ => kInvalidArgument,
        }
    }

    unsafe fn getLatencySamples(&self) -> u32 {
        0
    }

    unsafe fn setupProcessing(&self, setup: *mut ProcessSetup) -> tresult {
        if setup.is_null() {
            return kInvalidArgument;
        }

        let setup = unsafe { &*setup };
        self.runtime_handoff.publish_sample_rate(setup.sampleRate);

        kResultOk
    }

    unsafe fn setProcessing(&self, _state: TBool) -> tresult {
        kResultOk
    }

    unsafe fn process(&self, data: *mut ProcessData) -> tresult {
        if data.is_null() {
            return kInvalidArgument;
        }

        let process_data = unsafe { &*data };
        for id in [
            PARAM_MIX_NUM,
            PARAM_PHASE_OFFSET_NUM,
            PARAM_OUTPUT_GAIN_NUM,
            PARAM_SYNC_DIVISION_NUM,
        ] {
            if let Some((_, value)) =
                unsafe { latest_param_point(process_data.inputParameterChanges, id) }
            {
                apply_normalized_param(self.shared.params.as_ref(), id, value);
            }
        }

        // VST3 hosts may omit a deactivated trailing output bus, including for
        // parameter-only flushes. With no output bus there is no writable audio
        // range or sample format to validate after consuming parameter changes.
        if process_data.numOutputs == 0 {
            return process_ok();
        }

        if process_data.numSamples > 0
            && process_data.symbolicSampleSize != SymbolicSampleSizes_::kSample32 as i32
        {
            return kInvalidArgument;
        }

        let Some(buffers) = (unsafe { stereo_f32_buffers(process_data) }) else {
            return unsafe { silence_valid_stereo_output(process_data) };
        };

        let Some(mut runtime) = self.runtime.try_acquire() else {
            buffers.output_left.fill(0.0);
            buffers.output_right.fill(0.0);
            // SAFETY: successful stereo buffer extraction validated one
            // writable output bus above.
            unsafe { (*process_data.outputs).silenceFlags = 0b11 };
            return process_ok();
        };

        let pending = self.runtime_handoff.take_pending();
        if let Some(sample_rate) = pending.sample_rate {
            runtime.set_sample_rate(sample_rate.into(), self.shared.params.as_ref());
        }
        if pending.state_restored {
            runtime
                .engine
                .set_target_curve(self.shared.params.curve_snapshot());
            runtime.last_curve_revision = self.shared.params.curve_revision();
        }

        let revision = self.shared.params.curve_revision();
        if revision != runtime.last_curve_revision {
            runtime
                .engine
                .set_target_curve(self.shared.params.curve_snapshot());
            runtime.last_curve_revision = revision;
        }

        let settings = DspSettings {
            mix: self.shared.params.mix(),
            phase_offset: self.shared.params.phase_offset(),
            output_gain_db: self.shared.params.output_gain_db(),
            beats_per_cycle: self.shared.params.sync_beats_per_cycle(),
        };
        let transport = transport_state_from_vst3_process_context(process_data.processContext);
        let gui_phase = gui_phase_from_transport(transport, settings, self.shared.status.phase());
        self.shared.status.update(
            gui_phase,
            self.shared.status.gain(),
            gui_transport_telemetry(
                transport,
                settings.beats_per_cycle,
                self.shared.status.beat_phase(),
            ),
        );

        let mut last_phase = 0.0_f32;
        let mut last_gain = 1.0_f32;
        let mut transport_for_sample = transport;
        for sample_index in 0..buffers.num_samples {
            let mut left = buffers.input_left[sample_index];
            let mut right = buffers.input_right[sample_index];
            let telemetry = runtime.engine.process_sample(
                &mut left,
                &mut right,
                settings,
                transport_for_sample,
            );
            transport_for_sample.song_pos_beats = None;
            last_phase = telemetry.phase;
            last_gain = telemetry.gain;
            buffers.output_left[sample_index] = left;
            buffers.output_right[sample_index] = right;
        }
        // SAFETY: successful stereo buffer extraction validated one writable
        // output bus, and the full range now contains processed audio.
        unsafe { (*process_data.outputs).silenceFlags = 0 };
        self.shared.status.update(
            last_phase,
            last_gain,
            gui_transport_telemetry(
                transport,
                settings.beats_per_cycle,
                self.shared.status.beat_phase(),
            ),
        );

        process_ok()
    }

    unsafe fn getTailSamples(&self) -> u32 {
        0
    }
}

/// Silence a writable stereo f32 output when a full input/output block cannot
/// be formed. Invalid output descriptions return an error instead of claiming
/// successful processing while leaving host memory untouched.
unsafe fn silence_valid_stereo_output(data: &ProcessData) -> tresult {
    if data.numSamples < 0 {
        return kInvalidArgument;
    }
    if data.numSamples == 0 {
        return process_ok();
    }
    if data.symbolicSampleSize != SymbolicSampleSizes_::kSample32 as i32
        || data.numOutputs != 1
        || data.outputs.is_null()
    {
        return kInvalidArgument;
    }

    let output = unsafe { &mut *data.outputs };
    if output.numChannels != 2 || output.__field0.channelBuffers32.is_null() {
        return kInvalidArgument;
    }

    let channels = unsafe { slice::from_raw_parts(output.__field0.channelBuffers32, 2) };
    if channels.iter().any(|channel| channel.is_null()) {
        return kInvalidArgument;
    }

    let num_samples = data.numSamples as usize;
    unsafe { slice::from_raw_parts_mut(channels[0], num_samples) }.fill(0.0);
    unsafe { slice::from_raw_parts_mut(channels[1], num_samples) }.fill(0.0);
    output.silenceFlags = 0b11;
    process_ok()
}

impl IProcessContextRequirementsTrait for PumpVst3Processor {
    unsafe fn getProcessContextRequirements(&self) -> u32 {
        IProcessContextRequirements_::Flags_::kNeedTempo
            | IProcessContextRequirements_::Flags_::kNeedProjectTimeMusic
            | IProcessContextRequirements_::Flags_::kNeedTransportState
    }
}
