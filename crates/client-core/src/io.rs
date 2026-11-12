//! Explicit host services. Futures are Send on native, local on wasm.
use std::{future::Future, pin::Pin};
#[cfg(not(target_arch = "wasm32"))]
pub type Task<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
#[cfg(target_arch = "wasm32")]
pub type Task<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSend: Send {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send> MaybeSend for T {}
#[cfg(target_arch = "wasm32")]
pub trait MaybeSend {}
#[cfg(target_arch = "wasm32")]
impl<T> MaybeSend for T {}

pub trait Clock: Send + Sync {
    /// Monotonic milliseconds since an arbitrary host-chosen origin.
    fn monotonic_ms(&self) -> u64;
    fn unix_ms(&self) -> u64;
    fn sleep(&self, milliseconds: u64) -> Task<'static, ()>;
}
pub trait Spawner: Clone {
    /// Dropping the returned future must cancel the task (navigation/session teardown).
    fn spawn<T: MaybeSend + 'static>(
        &self,
        task: Task<'static, T>,
    ) -> Task<'static, Result<T, String>>;
}
pub trait Entropy {
    fn fill(&self, bytes: &mut [u8]) -> Result<(), String>;
}
pub fn nonce(entropy: &impl Entropy) -> u64 {
    let mut bytes = [0; 8];
    entropy
        .fill(&mut bytes)
        .expect("secure random source unavailable");
    (u64::from_le_bytes(bytes) & i64::MAX as u64).max(1)
}
/// Deterministic default for fixtures; interactive hosts inject their actual clock.
#[derive(Clone, Copy)]
pub struct FixedClock(pub u64);
impl Default for FixedClock {
    fn default() -> Self {
        Self(1_799_999_999_000)
    }
}
impl Clock for FixedClock {
    fn unix_ms(&self) -> u64 {
        self.0
    }
    fn monotonic_ms(&self) -> u64 {
        0
    }
    fn sleep(&self, _: u64) -> Task<'static, ()> {
        Box::pin(std::future::pending())
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSync: Sync {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Sync> MaybeSync for T {}
#[cfg(target_arch = "wasm32")]
pub trait MaybeSync {}
#[cfg(target_arch = "wasm32")]
impl<T> MaybeSync for T {}
