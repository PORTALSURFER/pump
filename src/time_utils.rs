//! Shared monotonic timing helpers.
//!
//! These helpers keep GUI and audio-thread timing math aligned without copying
//! epoch/overflow handling in multiple modules.

use std::sync::OnceLock;
use std::time::Instant;

/// Return monotonic microseconds since process-local runtime epoch.
pub(crate) fn monotonic_micros() -> u64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    let epoch = EPOCH.get_or_init(Instant::now);
    epoch.elapsed().as_micros().min(u64::MAX as u128) as u64
}

#[cfg(test)]
mod tests {
    use super::monotonic_micros;

    #[test]
    fn monotonic_micros_is_non_decreasing() {
        let first = monotonic_micros();
        let second = monotonic_micros();
        assert!(second >= first);
    }
}
