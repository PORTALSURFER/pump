//! VST3 processor implementation for Pump.
//!
//! This module owns the real-time audio processing side of the VST3 adapter.
//! Splitting it from `vst3.rs` keeps adapter wiring and processor behavior
//! isolated so host-facing changes and DSP changes are easier to review.

use super::*;
use crate::incoming_waveform::IncomingWaveformCapture;
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
    type Interfaces = (
        IComponent,
        IAudioProcessor,
        IAudioPresentationLatency,
        IProcessContextRequirements,
    );
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
                BusDirections_::kInput => 1,
                BusDirections_::kOutput => 1,
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
        if bus.is_null() || index < 0 {
            return kInvalidArgument;
        }
        if media_type as MediaTypes != MediaTypes_::kAudio {
            return kInvalidArgument;
        }

        let (label, bus_type, flags) = match dir as BusDirections {
            BusDirections_::kInput if index == 0 => (
                "Input",
                BusTypes_::kMain as BusType,
                BusInfo_::BusFlags_::kDefaultActive as u32,
            ),
            BusDirections_::kOutput if index == 0 => (
                "Output",
                BusTypes_::kMain as BusType,
                BusInfo_::BusFlags_::kDefaultActive as u32,
            ),
            _ => return kInvalidArgument,
        };

        let bus = unsafe { &mut *bus };
        bus.mediaType = MediaTypes_::kAudio as MediaType;
        bus.direction = dir;
        bus.channelCount = 2;
        copy_wstring(label, &mut bus.name);
        bus.busType = bus_type;
        bus.flags = flags;

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
        media_type: MediaType,
        dir: BusDirection,
        index: i32,
        _state: TBool,
    ) -> tresult {
        if media_type as MediaTypes != MediaTypes_::kAudio {
            return kInvalidArgument;
        }
        match dir as BusDirections {
            BusDirections_::kInput if index == 0 => {}
            BusDirections_::kOutput if index == 0 => {}
            _ => return kInvalidArgument,
        }
        kResultOk
    }

    unsafe fn setActive(&self, state: TBool) -> tresult {
        if state == 0 {
            self.runtime_handoff.reset_input_presentation_latency();
        }
        self.runtime_handoff.publish_processing_reset();
        kResultOk
    }

    unsafe fn setState(&self, state: *mut IBStream) -> tresult {
        let payload =
            unsafe { read_versioned_payload(state, STATE_MAGIC, ACCEPTED_STATE_VERSIONS) };
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
        if !(num_ins == 1 || num_ins == 2) || num_outs != 1 {
            return kResultFalse;
        }
        if inputs.is_null() || outputs.is_null() {
            return kInvalidArgument;
        }

        if unsafe { *inputs } != SpeakerArr::kStereo || unsafe { *outputs } != SpeakerArr::kStereo {
            return kResultFalse;
        }
        if num_ins == 2 && unsafe { *inputs.add(1) } != SpeakerArr::kStereo {
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
        if arr.is_null() || index < 0 {
            return kInvalidArgument;
        }

        match dir as BusDirections {
            BusDirections_::kInput if index == 0 || index == 1 => {
                unsafe { *arr = SpeakerArr::kStereo };
                kResultOk
            }
            BusDirections_::kOutput if index == 0 => {
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
        self.runtime_handoff.publish_processing_reset();
        kResultOk
    }

    unsafe fn process(&self, data: *mut ProcessData) -> tresult {
        if data.is_null() {
            return kInvalidArgument;
        }

        let process_data = unsafe { &*data };
        let frame_count = process_data.numSamples.max(0) as usize;
        let Some(mut runtime) = self.runtime.try_acquire() else {
            self.shared.status.mark_gain_reduction_inactive();
            apply_vst3_param_points_immediately(process_data, self.shared.params.as_ref());
            self.shared
                .status
                .incoming_waveform_buffer()
                .mark_unavailable();
            if process_data.numOutputs == 0 {
                return process_ok();
            }
            if process_data.numSamples > 0
                && process_data.symbolicSampleSize != SymbolicSampleSizes_::kSample32 as i32
            {
                return kInvalidArgument;
            }
            let Some(buffers) = (unsafe { raw_stereo_f32_buffers(process_data) }) else {
                return unsafe { silence_valid_stereo_output(process_data) };
            };
            unsafe { buffers.silence() };
            // SAFETY: successful stereo buffer extraction validated one
            // writable output bus above.
            unsafe { (*process_data.outputs).silenceFlags = 0b11 };
            return process_ok();
        };
        prepare_vst3_param_schedule(&mut runtime.param_schedule, process_data, frame_count);

        // VST3 hosts may omit a deactivated trailing output bus, including for
        // parameter-only flushes. With no output bus there is no writable audio
        // range or sample format to validate after consuming parameter changes.
        if process_data.numOutputs == 0 {
            self.shared.status.mark_gain_reduction_inactive();
            self.shared
                .status
                .incoming_waveform_buffer()
                .mark_unavailable();
            runtime.waveform_writer.reset();
            apply_scheduled_vst3_points_remaining(
                &mut runtime.param_schedule,
                self.shared.params.as_ref(),
            );
            return process_ok();
        }

        if process_data.numSamples > 0
            && process_data.symbolicSampleSize != SymbolicSampleSizes_::kSample32 as i32
        {
            self.shared.status.mark_gain_reduction_inactive();
            self.shared
                .status
                .incoming_waveform_buffer()
                .mark_unavailable();
            runtime.waveform_writer.reset();
            apply_scheduled_vst3_points_remaining(
                &mut runtime.param_schedule,
                self.shared.params.as_ref(),
            );
            return kInvalidArgument;
        }

        let Some(buffers) = (unsafe { raw_stereo_f32_buffers(process_data) }) else {
            self.shared.status.mark_gain_reduction_inactive();
            self.shared
                .status
                .incoming_waveform_buffer()
                .mark_unavailable();
            runtime.waveform_writer.reset();
            apply_scheduled_vst3_points_remaining(
                &mut runtime.param_schedule,
                self.shared.params.as_ref(),
            );
            return unsafe { silence_valid_stereo_output(process_data) };
        };

        let pending = self.runtime_handoff.take_pending();
        if let Some(sample_rate) = pending.sample_rate {
            runtime.set_sample_rate(sample_rate.into(), self.shared.params.as_ref());
        }
        if pending.processing_reset {
            runtime
                .engine
                .reset_with_bypass(self.shared.params.bypassed());
        }
        if pending.state_restored {
            runtime
                .engine
                .set_target_curve(self.shared.params.curve_snapshot());
            runtime.last_curve_revision = self.shared.params.curve_revision();
        }

        // Presentation latency is a host-side input property. Load it once
        // for the block so GUI, DSP, and waveform publication share one
        // compensated transport snapshot.
        let input_latency_samples = self.runtime_handoff.input_presentation_latency_samples();

        let revision = self.shared.params.curve_revision();
        if revision != runtime.last_curve_revision {
            runtime
                .engine
                .set_target_curve(self.shared.params.curve_snapshot());
            runtime.last_curve_revision = revision;
        }

        let mut settings = dsp_settings_from_params(self.shared.params.as_ref());
        runtime
            .param_schedule
            .apply_through(0, self.shared.params.as_ref(), &mut settings);
        let transport = crate::transport::compensate_input_presentation_latency(
            transport_state_from_vst3_process_context(process_data.processContext),
            Some(input_latency_samples),
            runtime.sample_rate.into(),
        );
        let gui_phase = gui_phase_from_transport(transport, settings, self.shared.status.phase());
        self.shared.status.update_transport(
            gui_phase,
            gui_transport_telemetry(
                transport,
                settings.beats_per_cycle,
                self.shared.status.beat_phase(),
            ),
        );

        let runtime = &mut *runtime;
        let telemetry = unsafe {
            process_stereo_block_raw(
                &mut runtime.engine,
                buffers,
                self.shared.params.as_ref(),
                &mut runtime.param_schedule,
                &mut settings,
                &mut runtime.last_curve_revision,
                transport,
                Some(IncomingWaveformCapture::new(
                    self.shared.status.incoming_waveform_buffer(),
                    &mut runtime.waveform_writer,
                )),
            )
        };
        let last_phase = telemetry.map(|telemetry| telemetry.phase).unwrap_or(0.0);
        // SAFETY: successful stereo buffer extraction validated one writable
        // output bus, and the full range now contains processed audio.
        unsafe { (*process_data.outputs).silenceFlags = 0 };
        self.shared.status.update_transport(
            last_phase,
            gui_transport_telemetry(
                transport,
                settings.beats_per_cycle,
                self.shared.status.beat_phase(),
            ),
        );
        if let Some(telemetry) = telemetry {
            self.shared
                .status
                .publish_gain_reduction(telemetry.reduction_gain, telemetry.input_active);
        } else {
            self.shared.status.mark_gain_reduction_inactive();
        }

        process_ok()
    }

    unsafe fn getTailSamples(&self) -> u32 {
        0
    }
}

impl IAudioPresentationLatencyTrait for PumpVst3Processor {
    unsafe fn setAudioPresentationLatencySamples(
        &self,
        dir: BusDirection,
        bus_index: int32,
        latency_in_samples: uint32,
    ) -> tresult {
        match dir as BusDirections {
            BusDirections_::kInput if bus_index == 0 => {
                self.runtime_handoff
                    .publish_input_presentation_latency(latency_in_samples);
                kResultOk
            }
            // Pump does not report or use output presentation latency, but a
            // valid main output bus is still accepted per the VST3 contract.
            BusDirections_::kOutput if bus_index == 0 => kResultOk,
            _ => kInvalidArgument,
        }
    }
}

/// Validate a VST3 stereo f32 block while retaining raw pointers so exact
/// in-place input/output channel aliases never become overlapping Rust slices.
unsafe fn raw_stereo_f32_buffers(data: &ProcessData) -> Option<RawStereoBlock> {
    if data.numInputs != 1
        || data.numOutputs != 1
        || data.inputs.is_null()
        || data.outputs.is_null()
    {
        return None;
    }

    let num_samples = usize::try_from(data.numSamples).ok()?;
    let input_bus = unsafe { &*data.inputs };
    let output_bus = unsafe { &*data.outputs };
    if input_bus.numChannels != 2
        || output_bus.numChannels != 2
        || input_bus.__field0.channelBuffers32.is_null()
        || output_bus.__field0.channelBuffers32.is_null()
    {
        return None;
    }

    let input_channels = unsafe { slice::from_raw_parts(input_bus.__field0.channelBuffers32, 2) };
    let output_channels = unsafe { slice::from_raw_parts(output_bus.__field0.channelBuffers32, 2) };
    if input_channels.iter().any(|channel| channel.is_null())
        || output_channels.iter().any(|channel| channel.is_null())
    {
        return None;
    }

    let main = RawStereoBlock {
        num_samples,
        input_left: input_channels[0],
        input_right: input_channels[1],
        output_left: output_channels[0],
        output_right: output_channels[1],
    };
    Some(main)
}

fn prepare_vst3_param_schedule(
    schedule: &mut ParamEventSchedule,
    process_data: &ProcessData,
    frame_count: usize,
) {
    schedule.begin_block(frame_count);
    unsafe {
        for_each_param_point(
            process_data.inputParameterChanges,
            |param_id, sample_offset, normalized| {
                schedule_vst3_param_point(schedule, param_id, sample_offset, normalized);
            },
        );
    }
    schedule.prepare();
}

fn apply_scheduled_vst3_points_remaining(schedule: &mut ParamEventSchedule, params: &PumpParams) {
    let mut settings = dsp_settings_from_params(params);
    schedule.apply_remaining(params, &mut settings);
}

fn schedule_vst3_param_point(
    schedule: &mut ParamEventSchedule,
    param_id: ParamID,
    sample_offset: i32,
    normalized: ParamValue,
) {
    let Some(clap_id) = clap_id_from_vst3_param_id(param_id) else {
        return;
    };
    let Some(plain) = plain_from_normalized_value(clap_id, normalized) else {
        return;
    };
    schedule.push(i64::from(sample_offset), clap_id, plain as f32);
}

fn apply_vst3_param_points_immediately(process_data: &ProcessData, params: &PumpParams) {
    unsafe {
        for_each_param_point(
            process_data.inputParameterChanges,
            |param_id, _sample_offset, normalized| {
                apply_normalized_param(params, param_id, normalized);
            },
        );
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

#[cfg(test)]
mod parameter_automation_tests {
    use super::*;
    use crate::params::{PARAM_BYPASS_NUM, PARAM_SYNC_DIVISION_NUM};

    #[test]
    fn vst3_points_share_offset_clamping_ordering_and_step_conversion() {
        let params = PumpParams::new();
        let mut schedule = ParamEventSchedule::default();
        schedule.begin_block(6);
        schedule_vst3_param_point(
            &mut schedule,
            PARAM_MIX_NUM,
            4,
            to_normalized(PARAM_MIX_NUM, 0.4),
        );
        schedule_vst3_param_point(
            &mut schedule,
            PARAM_SYNC_DIVISION_NUM,
            -5,
            to_normalized(PARAM_SYNC_DIVISION_NUM, 6.0),
        );
        schedule_vst3_param_point(
            &mut schedule,
            PARAM_MIX_NUM,
            99,
            to_normalized(PARAM_MIX_NUM, 0.8),
        );
        schedule_vst3_param_point(
            &mut schedule,
            PARAM_BYPASS_NUM,
            3,
            to_normalized(PARAM_BYPASS_NUM, 1.0),
        );
        schedule.prepare();
        let mut settings = dsp_settings_from_params(&params);

        schedule.apply_through(0, &params, &mut settings);
        assert_eq!(params.sync_division(), 6);
        assert!((params.mix() - 1.0).abs() < f32::EPSILON);
        schedule.apply_through(4, &params, &mut settings);
        assert!((params.mix() - 0.4).abs() < 1.0e-6);
        assert!(params.bypassed());
        assert!(settings.bypassed);
        schedule.apply_remaining(&params, &mut settings);
        assert!((params.mix() - 0.8).abs() < 1.0e-6);
    }
}
