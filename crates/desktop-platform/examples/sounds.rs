use sticktalk_desktop_platform::sound::Sound;
fn main() {
    for s in [
        Sound::Message,
        Sound::Mention,
        Sound::VoiceJoin,
        Sound::VoiceLeave,
        Sound::Mute,
        Sound::Unmute,
    ] {
        println!("Dry run: {s:?}: {} WAV bytes (not played)", s.wav().len());
    }
    // Real: bounded spawn_blocking(move || windows::play_sound(sound, dnd_known_off));
}
