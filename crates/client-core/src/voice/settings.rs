use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use sticktalk_proto::Id;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoiseSuppression {
    Off,
    Standard,
    #[default]
    High,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UserAudio {
    pub volume: u16,
    pub muted: bool,
}
impl Default for UserAudio {
    fn default() -> Self {
        Self {
            volume: 100,
            muted: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceSettings {
    pub input_device_id: Option<String>,
    pub output_device_id: Option<String>,
    pub input_volume: u16,
    pub output_volume: u16,
    pub sensitivity_auto: bool,
    pub sensitivity_dbfs: f32,
    pub noise_suppression: NoiseSuppression,
    pub echo_cancellation: bool,
    pub agc: bool,
    pub push_to_talk: bool,
    /// Win32 virtual key; None disables registration. No OS-specific object in core.
    pub push_to_talk_key: Option<u32>,
    pub push_to_talk_release_delay_ms: u16,
    pub studio_mode: bool,
    pub users: BTreeMap<Id, UserAudio>,
    pub share_fps: u32,
    /// 0 means Source.
    pub share_height: u32,
    pub share_include_audio: bool,
}
impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            input_device_id: None,
            output_device_id: None,
            input_volume: 100,
            output_volume: 100,
            sensitivity_auto: true,
            sensitivity_dbfs: -50.,
            noise_suppression: NoiseSuppression::High,
            echo_cancellation: true,
            agc: true,
            push_to_talk: false,
            push_to_talk_key: None,
            push_to_talk_release_delay_ms: 80,
            studio_mode: false,
            users: BTreeMap::new(),
            share_fps: 60,
            share_height: 1080,
            share_include_audio: false,
        }
    }
}
impl VoiceSettings {
    pub fn validate(&self) -> Result<(), String> {
        if self.input_volume > 200
            || self.output_volume > 200
            || self.push_to_talk_release_delay_ms > 1000
            || !self.sensitivity_dbfs.is_finite()
            || !(-96.0..=0.).contains(&self.sensitivity_dbfs)
            || self.users.len() > 1000
            || self.users.values().any(|u| u.volume > 200)
            || ![15, 30, 60].contains(&self.share_fps)
            || ![0, 480, 720, 1080, 1440].contains(&self.share_height)
            || self
                .push_to_talk_key
                .is_some_and(|k| !(1..=254).contains(&k))
            || [&self.input_device_id, &self.output_device_id]
                .into_iter()
                .flatten()
                .any(|s| s.len() > 4096 || s.contains('\0'))
        {
            return Err("Invalid Voice & Video settings".into());
        }
        Ok(())
    }
    pub fn user(&self, user: Id) -> UserAudio {
        self.users.get(&user).cloned().unwrap_or_default()
    }
    pub fn set_user(&mut self, user: Id, audio: UserAudio) -> Result<(), String> {
        if audio.volume > 200 || (self.users.len() >= 1000 && !self.users.contains_key(&user)) {
            return Err("User volume limit".into());
        }
        self.users.insert(user, audio);
        Ok(())
    }
}
