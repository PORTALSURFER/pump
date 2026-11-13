//! Bounded publication of high-resolution captured curves to the audio thread.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;

use crate::curve::{sample_editable_curve, EditableCurve};

/// Captures retain sixteen times the legacy playback table resolution.
pub(crate) const PRECISE_CURVE_LEN: usize = 16384;

struct Page {
    writer: Mutex<()>,
    sequence: AtomicU64,
    enabled: AtomicBool,
    samples: Box<[AtomicU32]>,
}

impl Default for Page {
    fn default() -> Self {
        Self {
            writer: Mutex::new(()),
            sequence: AtomicU64::new(0),
            enabled: AtomicBool::new(false),
            samples: (0..PRECISE_CURVE_LEN)
                .map(|_| AtomicU32::new(1.0_f32.to_bits()))
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        }
    }
}

/// Per-sound snapshots, written off the audio thread and read without locks.
#[derive(Default)]
pub(crate) struct SharedPreciseCurves {
    pages: [Page; 2],
}

impl SharedPreciseCurves {
    /// Publish a normalized editable curve on a non-audio thread.
    pub(crate) fn publish(&self, side: usize, curve: &EditableCurve) {
        let Some(page) = self.pages.get(side) else {
            return;
        };
        let _guard = page
            .writer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        page.sequence.fetch_add(1, Ordering::AcqRel);
        if curve.high_resolution {
            for (index, sample) in page.samples.iter().enumerate() {
                let phase = index as f32 / (PRECISE_CURVE_LEN - 1) as f32;
                sample.store(
                    sample_editable_curve(curve, phase).to_bits(),
                    Ordering::Relaxed,
                );
            }
        }
        page.enabled.store(curve.high_resolution, Ordering::Relaxed);
        page.sequence.fetch_add(1, Ordering::Release);
    }

    /// Copy a coherent snapshot into preallocated audio-owned storage.
    /// `None` means a writer overlapped the copy; the caller must retry later.
    pub(crate) fn try_read(&self, side: usize, output: &mut [f32]) -> Option<bool> {
        if output.len() != PRECISE_CURVE_LEN {
            return None;
        }
        let page = self.pages.get(side)?;
        let before = page.sequence.load(Ordering::Acquire);
        if before & 1 != 0 {
            return None;
        }
        let enabled = page.enabled.load(Ordering::Relaxed);
        if enabled {
            for (destination, source) in output.iter_mut().zip(page.samples.iter()) {
                *destination = f32::from_bits(source.load(Ordering::Relaxed));
            }
        }
        let after = page.sequence.load(Ordering::Acquire);
        (before == after).then_some(enabled)
    }
}

/// Interpolate the high-resolution cyclic table at a normalized phase.
pub(crate) fn sample_precise_curve(samples: &[f32], phase: f32) -> f32 {
    debug_assert_eq!(samples.len(), PRECISE_CURVE_LEN);
    let scaled = phase.rem_euclid(1.0) * (PRECISE_CURVE_LEN - 1) as f32;
    let left = scaled.floor() as usize;
    let right = (left + 1).min(PRECISE_CURVE_LEN - 1);
    let fraction = scaled - left as f32;
    (samples[left] + (samples[right] - samples[left]) * fraction).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curve::{CurveNode, CurveSegment};

    #[test]
    fn captured_curve_is_published_without_audio_thread_allocations() {
        let shared = SharedPreciseCurves::default();
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 1.0 },
                CurveNode { x: 0.2, y: 0.0 },
                CurveNode { x: 1.0, y: 1.0 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }; 2],
            high_resolution: true,
            ..EditableCurve::default()
        }
        .normalized();
        shared.publish(1, &curve);
        let mut output = vec![0.0; PRECISE_CURVE_LEN];
        assert_eq!(
            crate::test_alloc::assert_no_alloc(|| shared.try_read(1, &mut output)),
            Some(true)
        );
        assert!(
            (sample_precise_curve(&output, 0.2) - sample_editable_curve(&curve, 0.2)).abs() < 0.001
        );
        assert_eq!(shared.try_read(0, &mut output), Some(false));
        shared.publish(1, &EditableCurve::default());
        assert_eq!(shared.try_read(1, &mut output), Some(false));
    }

    #[test]
    fn concurrent_publication_never_returns_a_torn_curve() {
        use std::sync::atomic::AtomicBool;
        let shared = SharedPreciseCurves::default();
        let finished = AtomicBool::new(false);
        let curve = |value: f32| {
            EditableCurve {
                nodes: vec![
                    CurveNode { x: 0.0, y: value },
                    CurveNode { x: 1.0, y: value },
                ],
                segments: vec![CurveSegment { tension: 0.0 }],
                high_resolution: true,
                ..EditableCurve::default()
            }
            .normalized()
        };
        let a = curve(0.2);
        let b = curve(0.8);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                for index in 0..20 {
                    shared.publish(0, if index % 2 == 0 { &a } else { &b });
                }
                finished.store(true, Ordering::Release);
            });
            let mut output = vec![0.0; PRECISE_CURVE_LEN];
            while !finished.load(Ordering::Acquire) {
                if shared.try_read(0, &mut output) == Some(true) {
                    assert!(output.iter().all(|sample| *sample == output[0]));
                }
            }
        });
    }
}
