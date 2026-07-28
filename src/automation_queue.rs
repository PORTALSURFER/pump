//! Pump-owned transactional boundary for GUI automation events.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use toybox::clack_plugin::events::io::OutputEvents;
use toybox::clack_plugin::utils::ClapId;
use toybox::clap::automation::{
    AutomationConfig, AutomationDrainStats, AutomationDropPolicy, AutomationEnqueueStatus,
    AutomationEvent, AutomationQueue, AutomationQueueConfig,
};

use crate::params::clap_value_from_plain_value;

/// Bounded GUI automation queue with atomic three-event gesture admission.
///
/// Producers serialize through `gate`, allowing a complete begin/value/end edit
/// to preflight capacity before touching Toybox's per-event queue. Realtime
/// drains use `try_lock` and therefore never wait for a GUI producer.
pub struct PumpAutomationQueue {
    queue: AutomationQueue,
    gate: Mutex<()>,
    pending: AtomicUsize,
    gesture_end_reservation: AtomicUsize,
}

impl PumpAutomationQueue {
    /// Create a queue with the supplied bounded queue configuration.
    pub fn with_config(config: AutomationQueueConfig) -> Self {
        Self {
            queue: AutomationQueue::with_config(config),
            gate: Mutex::new(()),
            pending: AtomicUsize::new(0),
            gesture_end_reservation: AtomicUsize::new(0),
        }
    }

    /// Return the underlying bounded queue configuration.
    pub fn config(&self) -> AutomationQueueConfig {
        self.queue.config()
    }

    /// Enqueue one parameter value while keeping the tracked pending count exact.
    pub fn push_value(
        &self,
        config: &AutomationConfig,
        param_id: ClapId,
        value: f64,
    ) -> AutomationEnqueueStatus {
        if !config.is_enabled(param_id) {
            return AutomationEnqueueStatus::Disabled;
        }
        let Ok(_gate) = self.gate.lock() else {
            return AutomationEnqueueStatus::QueuePoisoned;
        };
        let status = self.queue.push_value(
            config,
            param_id,
            clap_value_from_plain_value(param_id, value),
        );
        if status == AutomationEnqueueStatus::Enqueued {
            self.record_enqueued(1);
        }
        status
    }

    /// Atomically admit one complete begin/value/end host edit.
    pub fn push_gesture_edit(
        &self,
        config: &AutomationConfig,
        param_id: ClapId,
        value: f64,
    ) -> bool {
        const GESTURE_EVENT_COUNT: usize = 3;

        if !config.is_enabled(param_id) {
            return false;
        }
        let Ok(_gate) = self.gate.lock() else {
            return false;
        };
        if !self.can_admit(GESTURE_EVENT_COUNT) {
            return false;
        }

        let statuses = [
            self.queue.push_gesture_begin(config, param_id),
            self.queue.push_value(
                config,
                param_id,
                clap_value_from_plain_value(param_id, value),
            ),
            self.queue.push_gesture_end(config, param_id),
        ];
        let complete = statuses
            .into_iter()
            .all(|status| status == AutomationEnqueueStatus::Enqueued);
        if complete {
            self.record_enqueued(GESTURE_EVENT_COUNT);
        }
        complete
    }

    /// Begin a continuous gesture while reserving space for its terminal event.
    pub fn push_gesture_begin(&self, config: &AutomationConfig, param_id: ClapId) -> bool {
        if !config.is_enabled(param_id) {
            return false;
        }
        let Ok(_gate) = self.gate.lock() else {
            return false;
        };
        if self.gesture_end_reservation.load(Ordering::Relaxed) != 0 {
            return false;
        }
        let queue_config = self.config();
        let pending = self.pending.load(Ordering::Relaxed);
        if queue_config.drop_policy == AutomationDropPolicy::DropNewest
            && pending.saturating_add(2) > queue_config.max_events
        {
            return false;
        }
        let status = self.queue.push_gesture_begin(config, param_id);
        if status != AutomationEnqueueStatus::Enqueued {
            return false;
        }
        self.record_enqueued(1);
        self.gesture_end_reservation.store(1, Ordering::Relaxed);
        true
    }

    /// Append one value to an active continuous gesture when capacity permits.
    pub fn push_gesture_value(
        &self,
        config: &AutomationConfig,
        param_id: ClapId,
        value: f64,
    ) -> bool {
        if !config.is_enabled(param_id) {
            return false;
        }
        let Ok(_gate) = self.gate.lock() else {
            return false;
        };
        if self.gesture_end_reservation.load(Ordering::Relaxed) == 0 {
            return false;
        }
        let queue_config = self.config();
        let pending = self.pending.load(Ordering::Relaxed);
        if queue_config.drop_policy == AutomationDropPolicy::DropNewest
            && pending.saturating_add(1) >= queue_config.max_events
        {
            return false;
        }
        let status = self.queue.push_value(
            config,
            param_id,
            clap_value_from_plain_value(param_id, value),
        );
        if status != AutomationEnqueueStatus::Enqueued {
            return false;
        }
        self.record_enqueued(1);
        true
    }

    /// End a continuous gesture, consuming its terminal-event reservation.
    pub fn push_gesture_end(&self, config: &AutomationConfig, param_id: ClapId) -> bool {
        if !config.is_enabled(param_id) {
            return false;
        }
        let Ok(_gate) = self.gate.lock() else {
            return false;
        };
        if self.gesture_end_reservation.load(Ordering::Relaxed) == 0 {
            return false;
        }
        let status = self.queue.push_gesture_end(config, param_id);
        if status != AutomationEnqueueStatus::Enqueued {
            return false;
        }
        self.record_enqueued(1);
        self.gesture_end_reservation.store(0, Ordering::Relaxed);
        true
    }

    /// Drain pending events without ever waiting for a GUI producer.
    pub fn drain_to_output(
        &self,
        output: &mut OutputEvents<'_>,
        scratch: &mut Vec<AutomationEvent>,
    ) -> AutomationDrainStats {
        let Ok(_gate) = self.gate.try_lock() else {
            return AutomationDrainStats {
                locked: true,
                ..AutomationDrainStats::default()
            };
        };
        let stats = self.queue.drain_to_output(output, scratch);
        if !stats.locked {
            let pending = self.pending.load(Ordering::Relaxed);
            self.pending
                .store(pending.saturating_sub(stats.attempted), Ordering::Relaxed);
        }
        stats
    }

    fn can_admit(&self, count: usize) -> bool {
        let config = self.config();
        if config.max_events < count {
            return false;
        }
        if config.drop_policy == AutomationDropPolicy::DropNewest {
            self.pending.load(Ordering::Relaxed) <= config.max_events - count
        } else {
            true
        }
    }

    fn record_enqueued(&self, count: usize) {
        let pending = self.pending.load(Ordering::Relaxed);
        self.pending.store(
            pending.saturating_add(count).min(self.config().max_events),
            Ordering::Relaxed,
        );
    }

    #[cfg(test)]
    fn pending_count(&self) -> usize {
        self.pending.load(Ordering::Relaxed)
    }
}

impl Default for PumpAutomationQueue {
    fn default() -> Self {
        Self::with_config(AutomationQueueConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use toybox::clack_plugin::events::io::EventBuffer;

    #[test]
    fn gesture_admission_is_all_or_nothing_when_capacity_is_short() {
        let queue = PumpAutomationQueue::with_config(AutomationQueueConfig::new(
            3,
            AutomationDropPolicy::DropNewest,
        ));
        let config = AutomationConfig::default();
        let existing = ClapId::new(1);
        let bypass = ClapId::new(10);
        assert_eq!(
            queue.push_value(&config, existing, 0.25),
            AutomationEnqueueStatus::Enqueued
        );

        assert!(!queue.push_gesture_edit(&config, bypass, 1.0));
        assert_eq!(queue.pending_count(), 1);

        let mut buffer = EventBuffer::new();
        let mut output = buffer.as_output();
        let mut scratch = Vec::with_capacity(3);
        let stats = queue.drain_to_output(&mut output, &mut scratch);
        assert_eq!(stats.attempted, 1);
        assert_eq!(buffer.len(), 1);
        assert_eq!(queue.pending_count(), 0);
    }

    #[test]
    fn realtime_drain_reports_locked_instead_of_waiting() {
        let queue = PumpAutomationQueue::default();
        let _gate = queue.gate.lock().expect("producer gate");
        let mut buffer = EventBuffer::new();
        let mut output = buffer.as_output();
        let mut scratch = Vec::new();

        let stats = queue.drain_to_output(&mut output, &mut scratch);

        assert!(stats.locked);
        assert_eq!(stats.attempted, 0);
    }

    #[test]
    fn continuous_gesture_preserves_begin_values_end_and_reserves_terminal_space() {
        let queue = PumpAutomationQueue::with_config(AutomationQueueConfig::new(
            4,
            AutomationDropPolicy::DropNewest,
        ));
        let config = AutomationConfig::default();
        let param = ClapId::new(1);

        assert!(queue.push_gesture_begin(&config, param));
        assert!(queue.push_gesture_value(&config, param, 0.25));
        assert!(queue.push_gesture_value(&config, param, 0.5));
        assert!(!queue.push_gesture_value(&config, param, 0.75));
        assert!(queue.push_gesture_end(&config, param));

        let mut buffer = EventBuffer::new();
        let mut output = buffer.as_output();
        let mut scratch = Vec::new();
        let stats = queue.drain_to_output(&mut output, &mut scratch);
        assert_eq!(stats.attempted, 4);
        assert_eq!(buffer.len(), 4);
        assert_eq!(queue.pending_count(), 0);
    }
}
