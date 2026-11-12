/// Original 24 kHz mono 16-bit PCM tones. No audio engine or persistent device.
#[derive(Clone, Copy, Debug)]
pub enum Sound {
    Message,
    Mention,
    VoiceJoin,
    VoiceLeave,
    Mute,
    Unmute,
}
impl Sound {
    pub fn wav(self) -> &'static [u8] {
        match self {
            Self::Message => include_bytes!("../assets/message.wav"),
            Self::Mention => include_bytes!("../assets/mention.wav"),
            Self::VoiceJoin => include_bytes!("../assets/voice-join.wav"),
            Self::VoiceLeave => include_bytes!("../assets/voice-leave.wav"),
            Self::Mute => include_bytes!("../assets/mute.wav"),
            Self::Unmute => include_bytes!("../assets/unmute.wav"),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tiny_valid_pcm_assets() {
        for s in [
            Sound::Message,
            Sound::Mention,
            Sound::VoiceJoin,
            Sound::VoiceLeave,
            Sound::Mute,
            Sound::Unmute,
        ] {
            let b = s.wav();
            assert_eq!(&b[..4], b"RIFF");
            assert_eq!(&b[8..12], b"WAVE");
            assert!(b.len() < 16000);
        }
    }
}
