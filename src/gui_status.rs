use super::*;

/// Extrapolate normalized phase from an anchor phase and elapsed time.
fn extrapolate_phase(
    anchor_phase: f32,
    frequency_hz: f32,
    is_playing: bool,
    last_update_micros: u64,
    now_micros: u64,
) -> f32 {
    let anchor = anchor_phase.rem_euclid(1.0);
    if !is_playing || now_micros <= last_update_micros {
        return anchor;
    }
    let elapsed_seconds = (now_micros - last_update_micros) as f32 / 1_000_000.0;
    (anchor + elapsed_seconds * frequency_hz.max(0.0)).rem_euclid(1.0)
}

/// Transport telemetry payload mirrored from the audio thread to the GUI.
#[derive(Debug, Copy, Clone)]
pub struct GuiTransportTelemetry {
    /// Whether phase feedback should continue advancing.
    pub is_playing: bool,
    /// Whether the host transport itself is currently running.
    pub transport_is_playing: bool,
    /// Whether host beat timeline is currently available.
    pub has_host_beats_timeline: bool,
    /// Normalized host beat phase in `[0, 1)`.
    pub beat_phase: f32,
    /// Host tempo in beats per minute.
    pub tempo_bpm: f32,
    /// Pump cycle length in quarter-note beats.
    pub beats_per_cycle: f32,
    /// Active timing source used by Pump's phase generator.
    pub timing_mode: usize,
    /// Effective cycle frequency in hertz for GUI phase extrapolation.
    pub effective_cycle_rate_hz: f32,
}

/// The last DSP-owned phase pair published to the GUI.
#[derive(Debug, Copy, Clone, PartialEq)]
pub(crate) struct GuiDspSnapshot {
    pub(crate) effective_phase: f32,
    pub(crate) applied_phase_offset: f32,
}

/// Shared GUI telemetry values updated by the audio thread.
pub struct GuiStatus {
    phase: AtomicF32,
    dsp_snapshot: AtomicU64,
    dsp_snapshot_valid: AtomicBool,
    gain_reduction_linear: AtomicF32,
    gain_reduction_active: AtomicBool,
    gain_reduction_last_update_micros: AtomicU64,
    gain_reduction_display_db: AtomicF32,
    gain_reduction_display_update_micros: AtomicU64,
    gain_reduction_clear_repaint_pending: AtomicBool,
    is_playing: AtomicBool,
    transport_is_playing: AtomicBool,
    has_host_beats_timeline: AtomicBool,
    beat_phase: AtomicF32,
    tempo_bpm: AtomicF32,
    cycle_hz: AtomicF32,
    last_update_micros: AtomicU64,
    incoming_waveform: crate::incoming_waveform::IncomingWaveformBuffer,
}

const GAIN_REDUCTION_STALE_MICROS: u64 = 250_000;
const GAIN_REDUCTION_ATTACK_SECONDS: f32 = 0.025;
const GAIN_REDUCTION_RELEASE_SECONDS: f32 = 0.180;
pub(crate) const GAIN_REDUCTION_METER_MAX_DB: f32 = 36.0;

impl Default for GuiStatus {
    fn default() -> Self {
        Self {
            phase: AtomicF32::new(0.0),
            dsp_snapshot: AtomicU64::new(0),
            dsp_snapshot_valid: AtomicBool::new(false),
            gain_reduction_linear: AtomicF32::new(1.0),
            gain_reduction_active: AtomicBool::new(false),
            gain_reduction_last_update_micros: AtomicU64::new(0),
            gain_reduction_display_db: AtomicF32::new(0.0),
            gain_reduction_display_update_micros: AtomicU64::new(0),
            gain_reduction_clear_repaint_pending: AtomicBool::new(false),
            is_playing: AtomicBool::new(false),
            transport_is_playing: AtomicBool::new(false),
            has_host_beats_timeline: AtomicBool::new(false),
            beat_phase: AtomicF32::new(0.0),
            tempo_bpm: AtomicF32::new(120.0),
            cycle_hz: AtomicF32::new(0.0),
            last_update_micros: AtomicU64::new(0),
            incoming_waveform: crate::incoming_waveform::IncomingWaveformBuffer::default(),
        }
    }
}

impl GuiStatus {
    /// Update transport plus linear reduction gain for deterministic UI tests.
    #[cfg(test)]
    pub fn update(&self, phase: f32, reduction_gain: f32, transport: GuiTransportTelemetry) {
        self.update_transport(phase, transport);
        self.publish_gain_reduction(reduction_gain, true);
    }

    pub(crate) fn incoming_waveform_buffer(
        &self,
    ) -> &crate::incoming_waveform::IncomingWaveformBuffer {
        &self.incoming_waveform
    }

    pub(crate) fn incoming_waveform_snapshot(
        &self,
    ) -> Option<crate::incoming_waveform::IncomingWaveformSnapshot> {
        self.incoming_waveform.snapshot()
    }

    pub(crate) fn waveform_live_mode(&self) -> bool {
        self.incoming_waveform.live_mode()
    }

    pub(crate) fn set_waveform_live_mode(&self, enabled: bool) {
        self.incoming_waveform.set_live_mode(enabled);
    }
    /// Update telemetry from the latest processed frame.
    pub fn update_transport(&self, phase: f32, transport: GuiTransportTelemetry) {
        let safe_tempo = transport.tempo_bpm.clamp(20.0, 320.0);
        let safe_beats_per_cycle = transport.beats_per_cycle.max(1.0e-4);
        self.phase.store(phase, Ordering::Relaxed);
        self.is_playing
            .store(transport.is_playing, Ordering::Relaxed);
        self.transport_is_playing
            .store(transport.transport_is_playing, Ordering::Relaxed);
        self.has_host_beats_timeline
            .store(transport.has_host_beats_timeline, Ordering::Relaxed);
        self.beat_phase
            .store(transport.beat_phase.rem_euclid(1.0), Ordering::Relaxed);
        self.tempo_bpm.store(safe_tempo, Ordering::Relaxed);
        self.cycle_hz.store(
            if transport.timing_mode == crate::params::TIMING_MODE_FREE {
                transport.effective_cycle_rate_hz.max(0.0)
            } else {
                (safe_tempo / 60.0) / safe_beats_per_cycle
            },
            Ordering::Relaxed,
        );
        self.last_update_micros
            .store(monotonic_micros(), Ordering::Relaxed);
    }

    /// Publish the effective phase and applied phase offset from one DSP block.
    pub(crate) fn publish_dsp_telemetry(&self, telemetry: crate::dsp::DspTelemetry) {
        self.dsp_snapshot.store(
            pack_dsp_snapshot(GuiDspSnapshot {
                effective_phase: telemetry.phase,
                applied_phase_offset: telemetry.applied_phase_offset,
            }),
            Ordering::Release,
        );
        self.dsp_snapshot_valid.store(true, Ordering::Release);
    }

    /// Read the latest coherent DSP phase pair, if a block has completed.
    pub(crate) fn dsp_snapshot(&self) -> Option<GuiDspSnapshot> {
        if !self.dsp_snapshot_valid.load(Ordering::Acquire) {
            return None;
        }
        Some(unpack_dsp_snapshot(
            self.dsp_snapshot.load(Ordering::Acquire),
        ))
    }

    /// Publish one block's strongest Pump attenuation without touching the audio path.
    pub fn publish_gain_reduction(&self, reduction_gain: f32, input_active: bool) {
        if !input_active {
            self.mark_gain_reduction_inactive();
            return;
        }
        self.gain_reduction_linear
            .store(reduction_gain.clamp(0.0, 1.0), Ordering::Relaxed);
        let was_active = self.gain_reduction_active.swap(true, Ordering::Relaxed);
        if !was_active {
            self.gain_reduction_display_update_micros
                .store(0, Ordering::Relaxed);
        }
        self.gain_reduction_last_update_micros
            .store(monotonic_micros(), Ordering::Relaxed);
    }

    /// Clear meter activity for silence, missing input, stopped processing, or bypass.
    pub fn mark_gain_reduction_inactive(&self) {
        self.gain_reduction_linear.store(1.0, Ordering::Relaxed);
        let was_active = self.gain_reduction_active.swap(false, Ordering::Relaxed);
        let displayed = self.gain_reduction_display_db.load(Ordering::Relaxed);
        self.gain_reduction_display_db.store(0.0, Ordering::Relaxed);
        self.gain_reduction_display_update_micros
            .store(monotonic_micros(), Ordering::Relaxed);
        if was_active || displayed > 0.0 {
            self.gain_reduction_clear_repaint_pending
                .store(true, Ordering::Release);
        }
    }

    /// Project phase from a caller-captured DSP snapshot, preserving fallback
    /// and extrapolation when no DSP block has completed yet.
    pub(crate) fn phase_from_dsp_snapshot(&self, snapshot: Option<GuiDspSnapshot>) -> f32 {
        let phase = snapshot
            .map(|snapshot| snapshot.effective_phase)
            .unwrap_or_else(|| self.phase.load(Ordering::Relaxed));
        extrapolate_phase(
            phase,
            self.cycle_hz.load(Ordering::Relaxed),
            self.is_playing(),
            self.last_update_micros.load(Ordering::Relaxed),
            monotonic_micros(),
        )
    }

    /// Read latest phase value.
    pub fn phase(&self) -> f32 {
        self.phase_from_dsp_snapshot(self.dsp_snapshot())
    }

    /// Read the smoothed live gain reduction in positive decibels.
    pub fn gain_reduction_db(&self) -> f32 {
        self.gain_reduction_db_at(monotonic_micros())
    }

    /// Keep the editor clock alive until an active or releasing meter reaches zero.
    pub fn gain_reduction_needs_redraw(&self) -> bool {
        self.gain_reduction_active.load(Ordering::Relaxed)
            || self.gain_reduction_display_db.load(Ordering::Relaxed) > 0.0
            || self
                .gain_reduction_clear_repaint_pending
                .load(Ordering::Acquire)
    }

    fn gain_reduction_db_at(&self, now_micros: u64) -> f32 {
        // Reading the meter means this frame can paint the cleared value.
        self.gain_reduction_clear_repaint_pending
            .store(false, Ordering::Release);
        let last_audio_update = self
            .gain_reduction_last_update_micros
            .load(Ordering::Relaxed);
        let current = self.gain_reduction_display_db.load(Ordering::Relaxed);
        let active = self.gain_reduction_active.load(Ordering::Relaxed)
            && self.transport_is_playing.load(Ordering::Relaxed)
            && last_audio_update > 0
            && now_micros.saturating_sub(last_audio_update) <= GAIN_REDUCTION_STALE_MICROS;
        if !active {
            self.gain_reduction_active.store(false, Ordering::Relaxed);
            self.gain_reduction_display_db.store(0.0, Ordering::Relaxed);
            self.gain_reduction_display_update_micros
                .store(now_micros, Ordering::Relaxed);
            return 0.0;
        }

        let target =
            gain_reduction_db_from_linear(self.gain_reduction_linear.load(Ordering::Relaxed));
        let last_display_update = self
            .gain_reduction_display_update_micros
            .swap(now_micros, Ordering::Relaxed);
        let displayed = if last_display_update == 0 || now_micros <= last_display_update {
            target
        } else {
            let elapsed = (now_micros - last_display_update) as f32 / 1_000_000.0;
            smooth_gain_reduction_db(current, target, elapsed)
        };
        self.gain_reduction_display_db
            .store(displayed, Ordering::Relaxed);
        displayed
    }

    /// Read whether host transport is currently playing.
    pub fn is_playing(&self) -> bool {
        self.is_playing.load(Ordering::Relaxed)
    }

    /// Read whether host beat timeline is currently available.
    pub fn has_host_beats_timeline(&self) -> bool {
        self.has_host_beats_timeline.load(Ordering::Relaxed)
    }

    /// Read normalized host beat phase in `[0, 1)`.
    pub fn beat_phase(&self) -> f32 {
        extrapolate_phase(
            self.beat_phase.load(Ordering::Relaxed),
            self.tempo_bpm.load(Ordering::Relaxed) / 60.0,
            self.is_playing(),
            self.last_update_micros.load(Ordering::Relaxed),
            monotonic_micros(),
        )
    }

    /// Return whether transport beat blink should currently be lit.
    pub fn transport_beat_blink_active(&self) -> bool {
        const BEAT_FLASH_DUTY: f32 = 0.18;
        if !self.is_playing() {
            return false;
        }
        if self.has_host_beats_timeline() {
            return self.beat_phase() < BEAT_FLASH_DUTY;
        }
        // Fallback activity mode: keep the transport indicator lit while
        // playing when the host does not expose a beat timeline.
        true
    }
}

fn pack_dsp_snapshot(snapshot: GuiDspSnapshot) -> u64 {
    (u64::from(snapshot.effective_phase.to_bits()) << 32)
        | u64::from(snapshot.applied_phase_offset.to_bits())
}

fn unpack_dsp_snapshot(value: u64) -> GuiDspSnapshot {
    GuiDspSnapshot {
        effective_phase: f32::from_bits((value >> 32) as u32),
        applied_phase_offset: f32::from_bits(value as u32),
    }
}

pub(crate) fn gain_reduction_db_from_linear(gain: f32) -> f32 {
    if !gain.is_finite() || gain >= 1.0 {
        return 0.0;
    }
    (-20.0
        * gain
            .max(10.0_f32.powf(-GAIN_REDUCTION_METER_MAX_DB / 20.0))
            .log10())
    .clamp(0.0, GAIN_REDUCTION_METER_MAX_DB)
}

pub(crate) fn gain_reduction_meter_fraction(db: f32) -> f32 {
    (db / GAIN_REDUCTION_METER_MAX_DB).clamp(0.0, 1.0)
}

fn smooth_gain_reduction_db(current: f32, target: f32, elapsed_seconds: f32) -> f32 {
    let time = if target > current {
        GAIN_REDUCTION_ATTACK_SECONDS
    } else {
        GAIN_REDUCTION_RELEASE_SECONDS
    };
    let alpha = 1.0 - (-elapsed_seconds.max(0.0) / time).exp();
    (current + (target - current) * alpha).clamp(0.0, GAIN_REDUCTION_METER_MAX_DB)
}

#[allow(clippy::question_mark)]
#[cfg(all(target_os = "macos", feature = "radiant-gui"))]
impl<'a> toybox::clack_extensions::gui::PluginGuiImpl for PumpMainThread<'a> {
    toybox::radiant_clap_gui_callbacks!(
        gui = gui,
        preferred_size = crate::gui::preferred_window_size,
        show = |_plugin: &mut Self| Ok(())
    );
}

#[cfg(test)]
mod tests {
    use crate::dsp::db_to_linear;
    use std::sync::atomic::Ordering;

    use super::{
        extrapolate_phase, gain_reduction_db_from_linear, gain_reduction_meter_fraction,
        monotonic_micros, smooth_gain_reduction_db, GuiDspSnapshot, GuiStatus,
        GuiTransportTelemetry, GAIN_REDUCTION_METER_MAX_DB, GAIN_REDUCTION_STALE_MICROS,
    };

    #[test]
    fn db_to_linear_matches_unity_at_zero_db() {
        assert!((db_to_linear(0.0) - 1.0).abs() < 1.0e-6);
    }

    #[test]
    fn gain_reduction_mapping_is_decibel_based_and_clamped() {
        assert_eq!(gain_reduction_db_from_linear(1.0), 0.0);
        assert!((gain_reduction_db_from_linear(0.5) - 6.0206).abs() < 1.0e-3);
        assert_eq!(
            gain_reduction_db_from_linear(0.0),
            GAIN_REDUCTION_METER_MAX_DB
        );
        assert_eq!(gain_reduction_db_from_linear(f32::NAN), 0.0);
        assert_eq!(gain_reduction_meter_fraction(-12.0), 0.0);
        assert_eq!(
            gain_reduction_meter_fraction(GAIN_REDUCTION_METER_MAX_DB * 2.0),
            1.0
        );
    }

    #[test]
    fn gain_reduction_ballistics_attack_faster_than_release() {
        let attack = smooth_gain_reduction_db(0.0, 24.0, 0.025);
        let release = smooth_gain_reduction_db(24.0, 0.0, 0.025);
        assert!(
            attack > 12.0,
            "attack should quickly approach a stronger reduction"
        );
        assert!(
            release > 18.0,
            "release should decay more slowly than attack"
        );
    }

    #[test]
    fn meter_clears_for_inactive_stopped_and_stale_states() {
        let status = GuiStatus::default();
        status.transport_is_playing.store(true, Ordering::Relaxed);
        status.gain_reduction_linear.store(0.25, Ordering::Relaxed);
        status.gain_reduction_active.store(true, Ordering::Relaxed);
        status
            .gain_reduction_last_update_micros
            .store(1_000_000, Ordering::Relaxed);
        assert!((status.gain_reduction_db_at(1_010_000) - 12.0412).abs() < 1.0e-3);

        status.transport_is_playing.store(false, Ordering::Relaxed);
        assert_eq!(status.gain_reduction_db_at(1_020_000), 0.0);

        status.transport_is_playing.store(true, Ordering::Relaxed);
        status.gain_reduction_active.store(true, Ordering::Relaxed);
        assert_eq!(
            status.gain_reduction_db_at(1_000_000 + GAIN_REDUCTION_STALE_MICROS + 1),
            0.0
        );

        status
            .gain_reduction_last_update_micros
            .store(2_000_000, Ordering::Relaxed);
        status.gain_reduction_active.store(true, Ordering::Relaxed);
        status.mark_gain_reduction_inactive();
        assert_eq!(status.gain_reduction_db_at(2_010_000), 0.0);
    }

    #[test]
    fn stopped_meter_requests_one_clearing_ui_update() {
        let status = GuiStatus::default();
        status.gain_reduction_active.store(true, Ordering::Relaxed);
        status
            .gain_reduction_display_db
            .store(12.0, Ordering::Relaxed);
        status.transport_is_playing.store(false, Ordering::Relaxed);
        assert!(status.gain_reduction_needs_redraw());
        assert_eq!(status.gain_reduction_db_at(1_000_000), 0.0);
        assert!(!status.gain_reduction_needs_redraw());
    }

    #[test]
    fn inactive_clear_stays_pending_until_the_meter_is_read() {
        let status = GuiStatus::default();
        status.gain_reduction_active.store(true, Ordering::Relaxed);
        status
            .gain_reduction_display_db
            .store(12.0, Ordering::Relaxed);

        status.mark_gain_reduction_inactive();
        status.mark_gain_reduction_inactive();
        assert!(
            status.gain_reduction_needs_redraw(),
            "repeated inactive blocks must not consume the pending clear repaint"
        );

        assert_eq!(status.gain_reduction_db_at(1_000_000), 0.0);
        assert!(!status.gain_reduction_needs_redraw());

        status.mark_gain_reduction_inactive();
        assert!(
            !status.gain_reduction_needs_redraw(),
            "already-cleared inactivity must not request unbounded repaints"
        );
    }

    #[test]
    fn meter_uses_raw_stopped_state_when_phase_fallback_is_running() {
        let status = GuiStatus::default();
        status.update(
            0.25,
            0.25,
            GuiTransportTelemetry {
                is_playing: true,
                transport_is_playing: false,
                has_host_beats_timeline: false,
                beat_phase: 0.25,
                tempo_bpm: 120.0,
                beats_per_cycle: 1.0,
                timing_mode: crate::params::TIMING_MODE_SYNC,
                effective_cycle_rate_hz: 0.0,
            },
        );
        let last_audio_update = status
            .gain_reduction_last_update_micros
            .load(Ordering::Relaxed);

        assert!(status.is_playing(), "phase fallback should remain active");
        assert_eq!(
            status.gain_reduction_db_at(last_audio_update),
            0.0,
            "raw stopped transport must clear the meter without a beat timeline"
        );
    }

    #[test]
    fn transport_beat_blink_requires_playback_and_uses_timeline_when_available() {
        let status = GuiStatus::default();
        status.update(
            0.0,
            1.0,
            GuiTransportTelemetry {
                is_playing: true,
                transport_is_playing: true,
                has_host_beats_timeline: true,
                beat_phase: 0.05,
                tempo_bpm: 120.0,
                beats_per_cycle: 1.0,
                timing_mode: crate::params::TIMING_MODE_SYNC,
                effective_cycle_rate_hz: 0.0,
            },
        );
        assert!(status.transport_beat_blink_active());

        status.update(
            0.0,
            1.0,
            GuiTransportTelemetry {
                is_playing: true,
                transport_is_playing: true,
                has_host_beats_timeline: true,
                beat_phase: 0.5,
                tempo_bpm: 120.0,
                beats_per_cycle: 1.0,
                timing_mode: crate::params::TIMING_MODE_SYNC,
                effective_cycle_rate_hz: 0.0,
            },
        );
        assert!(!status.transport_beat_blink_active());

        status.update(
            0.0,
            1.0,
            GuiTransportTelemetry {
                is_playing: false,
                transport_is_playing: false,
                has_host_beats_timeline: true,
                beat_phase: 0.05,
                tempo_bpm: 120.0,
                beats_per_cycle: 1.0,
                timing_mode: crate::params::TIMING_MODE_SYNC,
                effective_cycle_rate_hz: 0.0,
            },
        );
        assert!(!status.transport_beat_blink_active());

        status.update(
            0.0,
            1.0,
            GuiTransportTelemetry {
                is_playing: true,
                transport_is_playing: true,
                has_host_beats_timeline: false,
                beat_phase: 0.05,
                tempo_bpm: 120.0,
                beats_per_cycle: 1.0,
                timing_mode: crate::params::TIMING_MODE_SYNC,
                effective_cycle_rate_hz: 0.0,
            },
        );
        assert!(status.transport_beat_blink_active());
    }

    #[test]
    fn extrapolate_phase_advances_when_playing() {
        let phase = extrapolate_phase(0.25, 2.0, true, 1_000_000, 1_250_000);
        assert!((phase - 0.75).abs() < 1.0e-6);
    }

    #[test]
    fn extrapolate_phase_holds_when_not_playing() {
        let phase = extrapolate_phase(0.25, 2.0, false, 1_000_000, 2_000_000);
        assert!((phase - 0.25).abs() < 1.0e-6);
    }

    #[test]
    fn captured_dsp_snapshot_remains_authoritative_after_replacement() {
        let status = GuiStatus::default();
        let captured_snapshot = GuiDspSnapshot {
            effective_phase: 0.2,
            applied_phase_offset: 0.3,
        };
        let replacement_snapshot = GuiDspSnapshot {
            effective_phase: 0.8,
            applied_phase_offset: 0.9,
        };

        status.publish_dsp_telemetry(crate::dsp::DspTelemetry {
            phase: captured_snapshot.effective_phase,
            applied_phase_offset: captured_snapshot.applied_phase_offset,
            gain: 1.0,
            reduction_gain: 1.0,
            input_active: false,
            bypassed: false,
        });
        let captured = status
            .dsp_snapshot()
            .expect("the first DSP block should be available");

        status.publish_dsp_telemetry(crate::dsp::DspTelemetry {
            phase: replacement_snapshot.effective_phase,
            applied_phase_offset: replacement_snapshot.applied_phase_offset,
            gain: 1.0,
            reduction_gain: 1.0,
            input_active: false,
            bypassed: false,
        });

        // Model a publication between projection reads without depending on
        // thread scheduling: the surface owns this Copy snapshot after capture.
        let projected_applied_phase_offset = captured.applied_phase_offset;
        let projected_effective_phase = status.phase_from_dsp_snapshot(Some(captured));

        assert_eq!(
            projected_applied_phase_offset,
            captured_snapshot.applied_phase_offset
        );
        assert_eq!(projected_effective_phase, captured_snapshot.effective_phase);
        assert_eq!(status.dsp_snapshot(), Some(replacement_snapshot));
    }

    #[test]
    fn gui_status_phase_holds_when_last_update_timestamp_is_stale_future_value() {
        let status = GuiStatus::default();
        status.update(
            0.42,
            1.0,
            GuiTransportTelemetry {
                is_playing: true,
                transport_is_playing: true,
                has_host_beats_timeline: true,
                beat_phase: 0.2,
                tempo_bpm: 120.0,
                beats_per_cycle: 1.0,
                timing_mode: crate::params::TIMING_MODE_SYNC,
                effective_cycle_rate_hz: 0.0,
            },
        );
        status.last_update_micros.store(
            monotonic_micros().saturating_add(1_000_000),
            Ordering::Relaxed,
        );
        let phase = status.phase();
        assert!(
            (phase - 0.42).abs() <= 1.0e-6,
            "future/stale timestamp should hold anchor phase instead of extrapolating"
        );
    }

    #[test]
    fn gui_status_beat_phase_holds_when_last_update_timestamp_is_stale_future_value() {
        let status = GuiStatus::default();
        status.update(
            0.0,
            1.0,
            GuiTransportTelemetry {
                is_playing: true,
                transport_is_playing: true,
                has_host_beats_timeline: true,
                beat_phase: 0.73,
                tempo_bpm: 123.0,
                beats_per_cycle: 1.0,
                timing_mode: crate::params::TIMING_MODE_SYNC,
                effective_cycle_rate_hz: 0.0,
            },
        );
        status.last_update_micros.store(
            monotonic_micros().saturating_add(1_000_000),
            Ordering::Relaxed,
        );
        let phase = status.beat_phase();
        assert!(
            (phase - 0.73).abs() <= 1.0e-6,
            "future/stale timestamp should hold anchor beat phase instead of extrapolating"
        );
    }
}
