use super::*;
use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};

const PENDING_SAMPLE_RATE: u8 = 1 << 0;
const PENDING_STATE_RESTORE: u8 = 1 << 1;

#[derive(Debug, Copy, Clone)]
pub(super) struct PendingRuntimeChanges {
    pub(super) sample_rate: Option<f32>,
    pub(super) state_restored: bool,
}

/// Single-producer/multi-producer handoff into the audio-owned DSP runtime.
///
/// Setup and state callbacks only publish fixed-size atomic messages. The
/// realtime callback consumes them without taking ownership away from the
/// audio thread or waiting for another thread to release a lock.
pub(super) struct RuntimeHandoff {
    sample_rate_bits: AtomicU32,
    pending: AtomicU8,
}

impl RuntimeHandoff {
    pub(super) fn new() -> Self {
        Self {
            sample_rate_bits: AtomicU32::new(48_000.0_f32.to_bits()),
            pending: AtomicU8::new(0),
        }
    }

    pub(super) fn publish_sample_rate(&self, sample_rate: f64) {
        let sample_rate = if sample_rate.is_finite() {
            sample_rate.max(1.0) as f32
        } else {
            48_000.0
        };
        self.sample_rate_bits
            .store(sample_rate.to_bits(), Ordering::Release);
        self.pending
            .fetch_or(PENDING_SAMPLE_RATE, Ordering::Release);
    }

    pub(super) fn publish_state_restore(&self) {
        self.pending
            .fetch_or(PENDING_STATE_RESTORE, Ordering::Release);
    }

    pub(super) fn take_pending(&self) -> PendingRuntimeChanges {
        let pending = self.pending.swap(0, Ordering::AcqRel);
        PendingRuntimeChanges {
            sample_rate: (pending & PENDING_SAMPLE_RATE != 0)
                .then(|| f32::from_bits(self.sample_rate_bits.load(Ordering::Acquire)).max(1.0)),
            state_restored: pending & PENDING_STATE_RESTORE != 0,
        }
    }
}

/// Shared VST3 state used by processor, controller, and hosted GUI.
pub(super) struct PumpVst3Shared {
    pub(super) params: Arc<PumpParams>,
    pub(super) status: Arc<GuiStatus>,
    pub(super) automation_queue: Arc<AutomationQueue>,
}

impl PumpVst3Shared {
    pub(super) fn new() -> Self {
        Self {
            params: Arc::new(PumpParams::new()),
            status: Arc::new(GuiStatus::default()),
            automation_queue: Arc::new(AutomationQueue::default()),
        }
    }
}

#[derive(Copy, Clone)]
pub(super) enum SharedRole {
    Processor,
    Controller,
}

pub(super) struct SharedRegistryEntry {
    pub(super) shared: Weak<PumpVst3Shared>,
    pub(super) processor_claimed: bool,
    pub(super) controller_claimed: bool,
}

pub(super) fn shared_registry() -> &'static Mutex<Vec<SharedRegistryEntry>> {
    static REGISTRY: OnceLock<Mutex<Vec<SharedRegistryEntry>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Vec::new()))
}

pub(super) fn acquire_shared_for_role(role: SharedRole) -> Arc<PumpVst3Shared> {
    let mut registry = match shared_registry().lock() {
        Ok(registry) => registry,
        Err(_) => return Arc::new(PumpVst3Shared::new()),
    };

    registry.retain(|entry| entry.shared.upgrade().is_some());
    for entry in registry.iter_mut() {
        let Some(shared) = entry.shared.upgrade() else {
            continue;
        };
        match role {
            SharedRole::Processor if !entry.processor_claimed => {
                entry.processor_claimed = true;
                return shared;
            }
            SharedRole::Controller if !entry.controller_claimed => {
                entry.controller_claimed = true;
                return shared;
            }
            _ => {}
        }
    }

    let shared = Arc::new(PumpVst3Shared::new());
    registry.push(SharedRegistryEntry {
        shared: Arc::downgrade(&shared),
        processor_claimed: matches!(role, SharedRole::Processor),
        controller_claimed: matches!(role, SharedRole::Controller),
    });
    shared
}

/// Release one shared-state role claim when a VST3 component instance drops.
pub(super) fn release_shared_for_role(shared: &Arc<PumpVst3Shared>, role: SharedRole) {
    let mut registry = match shared_registry().lock() {
        Ok(registry) => registry,
        Err(_) => return,
    };

    registry.retain(|entry| entry.shared.upgrade().is_some());
    for entry in registry.iter_mut() {
        let Some(candidate) = entry.shared.upgrade() else {
            continue;
        };
        if !Arc::ptr_eq(&candidate, shared) {
            continue;
        }
        match role {
            SharedRole::Processor => entry.processor_claimed = false,
            SharedRole::Controller => entry.controller_claimed = false,
        }
    }
}

pub(super) struct PumpVst3Runtime {
    pub(super) engine: PumpEngine,
    pub(super) param_schedule: ParamEventSchedule,
    pub(super) last_curve_revision: u32,
    pub(super) sample_rate: f32,
}

impl PumpVst3Runtime {
    pub(super) fn new(params: &PumpParams) -> Self {
        let curve = params.curve_snapshot();
        Self {
            engine: PumpEngine::new(48_000.0, curve),
            param_schedule: ParamEventSchedule::with_capacity(16_384),
            last_curve_revision: params.curve_revision(),
            sample_rate: 48_000.0,
        }
    }

    pub(super) fn set_sample_rate(&mut self, sample_rate: f64, params: &PumpParams) {
        let clamped = sample_rate.max(1.0) as f32;
        if (self.sample_rate - clamped).abs() < 1.0e-6 {
            return;
        }

        self.sample_rate = clamped;
        self.engine = PumpEngine::new(self.sample_rate, params.curve_snapshot());
        self.last_curve_revision = params.curve_revision();
    }
}
