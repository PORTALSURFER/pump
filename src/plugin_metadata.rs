//! Shared plugin metadata and naming policy.
//!
//! Policy:
//! - Stable plugin IDs use reverse-DNS lowercase (for example `com.portalsurfer.pump`).
//! - Human-facing vendor names are display strings and may use uppercase branding.

/// Stable CLAP plugin identifier (reverse-DNS lowercase).
pub(crate) const PLUGIN_ID: &str = "com.portalsurfer.pump";
/// Human-facing plugin display name.
pub(crate) const PLUGIN_NAME: &str = "pump";
/// Human-facing vendor display name.
pub(crate) const VENDOR_NAME: &str = "PORTALSURFER";
/// Vendor/project homepage used in VST3 factory metadata.
#[cfg(feature = "vst3")]
pub(crate) const VENDOR_URL: &str = "https://github.com/uhx/pump";
/// Vendor support contact used in VST3 factory metadata.
#[cfg(feature = "vst3")]
pub(crate) const VENDOR_EMAIL: &str = "support@localhost";

#[cfg(test)]
mod tests {
    use super::{PLUGIN_ID, PLUGIN_NAME, VENDOR_NAME};

    #[test]
    fn plugin_id_stays_reverse_dns_lowercase() {
        assert_eq!(PLUGIN_ID, PLUGIN_ID.to_ascii_lowercase());
        assert!(PLUGIN_ID.split('.').count() >= 3);
    }

    #[test]
    fn plugin_name_remains_host_facing_identity() {
        assert_eq!(PLUGIN_NAME, "pump");
    }

    #[test]
    fn vendor_name_allows_uppercase_branding() {
        let has_alpha = VENDOR_NAME.chars().any(char::is_alphabetic);
        assert!(has_alpha);
        assert!(VENDOR_NAME
            .chars()
            .filter(|ch| ch.is_alphabetic())
            .all(|ch| ch.is_uppercase()));
    }
}
