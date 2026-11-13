//! Main-thread host interface implementations for Pump.
//!
//! This module keeps host callback surfaces separate from crate-level plugin
//! registration/wiring, reducing review surface when host-facing behavior
//! changes.

use super::*;

impl<'a> PluginMainThread<'a, PumpShared> for PumpMainThread<'a> {}

impl PluginAudioPortsImpl for PumpMainThread<'_> {
    fn count(&mut self, is_input: bool) -> u32 {
        if is_input {
            2
        } else {
            1
        }
    }

    fn get(&mut self, index: u32, is_input: bool, writer: &mut AudioPortInfoWriter) {
        if let Some(info) = audio_port_info(index, is_input) {
            writer.set(&info);
        }
    }
}

fn audio_port_info(index: u32, is_input: bool) -> Option<AudioPortInfo<'static>> {
    match (is_input, index) {
        (true, 0) | (false, 0) => Some(AudioPortInfo {
            id: ClapId::new(0),
            name: b"main",
            channel_count: 2,
            flags: AudioPortFlags::IS_MAIN,
            port_type: Some(AudioPortType::STEREO),
            in_place_pair: None,
        }),
        (true, 1) => Some(AudioPortInfo {
            id: ClapId::new(1),
            name: b"Sidechain",
            channel_count: 2,
            flags: AudioPortFlags::empty(),
            port_type: Some(AudioPortType::STEREO),
            in_place_pair: None,
        }),
        _ => None,
    }
}

impl PluginMainThreadParams for PumpMainThread<'_> {
    fn count(&mut self) -> u32 {
        param_count()
    }

    fn get_info(&mut self, param_index: u32, info: &mut ParamInfoWriter) {
        write_param_info(param_index, info);
    }

    fn get_value(&mut self, param_id: ClapId) -> Option<f64> {
        get_param_value(self.shared.params.as_ref(), param_id)
            .map(|value| clap_value_from_plain_value(param_id, value))
    }

    fn value_to_text(
        &mut self,
        param_id: ClapId,
        value: f64,
        writer: &mut ParamDisplayWriter,
    ) -> std::fmt::Result {
        value_to_text(
            self.shared.params.as_ref(),
            param_id,
            plain_value_from_clap_value(param_id, value),
            writer,
        )
    }

    fn text_to_value(&mut self, param_id: ClapId, text: &std::ffi::CStr) -> Option<f64> {
        text_to_value(param_id, text).map(|value| clap_value_from_plain_value(param_id, value))
    }

    fn flush(
        &mut self,
        input_parameter_changes: &InputEvents,
        output_parameter_changes: &mut OutputEvents,
    ) {
        apply_param_events(input_parameter_changes, |param_id, value| {
            apply_clap_param_event(self.shared.params.as_ref(), param_id, value as f32)
        });

        let _stats = self
            .shared
            .automation_queue
            .drain_to_output(output_parameter_changes, &mut self.automation_drain);
    }
}

impl PluginStateImpl for PumpMainThread<'_> {
    fn save(&mut self, output: &mut OutputStream) -> Result<(), PluginError> {
        let payload = encode_state_payload(self.shared.params.as_ref());
        write_versioned_payload(output, STATE_MAGIC, STATE_VERSION, &payload)?;
        Ok(())
    }

    fn load(&mut self, input: &mut InputStream) -> Result<(), PluginError> {
        let payload = read_versioned_payload(input, STATE_MAGIC, &[STATE_VERSION])?;
        decode_state_payload(self.shared.params.as_ref(), &payload.payload)
            .map_err(PluginError::Message)
    }
}

#[cfg(test)]
mod audio_port_tests {
    use super::*;

    #[test]
    fn clap_sidechain_is_distinct_stereo_input_without_output_pair() {
        let main = audio_port_info(0, true).unwrap();
        let sidechain = audio_port_info(1, true).unwrap();
        assert_ne!(main.id, sidechain.id);
        assert!(main.flags.contains(AudioPortFlags::IS_MAIN));
        assert!(!sidechain.flags.contains(AudioPortFlags::IS_MAIN));
        assert_eq!(sidechain.channel_count, 2);
        assert_eq!(sidechain.port_type, Some(AudioPortType::STEREO));
        assert!(sidechain.in_place_pair.is_none());
        assert!(audio_port_info(1, false).is_none());
        assert!(audio_port_info(2, true).is_none());
    }
}
