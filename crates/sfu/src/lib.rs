//! Standalone WebRTC SFU. See README.md for the authenticated embedding contract.
mod boundary;
mod control;
mod engine;
pub mod pacing;
pub mod quality;
#[cfg(feature = "media-tap")]
mod tap;
mod transport;
mod validation;

pub use control::{AgentStep, AGENT_HOLD_TIME, AGENT_SCRIPT_STEPS, AGENT_SCRIPT_TIME};
pub use engine::{rtc_config, Authorizer, Config, Error, Out, Sfu, LISTENER_VIDEO_BPS};
pub use sticktalk_proto as proto;
pub use transport::{encode_event, encode_signal, AgentAction, Command, ControlOut, SignalOut};
