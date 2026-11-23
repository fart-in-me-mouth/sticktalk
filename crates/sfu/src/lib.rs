//! Standalone WebRTC SFU. See README.md for the authenticated embedding contract.
mod boundary;
mod control;
mod engine;
pub mod pacing;
pub mod quality;
mod transport;
mod validation;

pub use engine::{rtc_config, Authorizer, Config, Error, Out, Sfu};
pub use sticktalk_proto as proto;
pub use transport::{encode_event, encode_signal, Command, ControlOut, SignalOut};
