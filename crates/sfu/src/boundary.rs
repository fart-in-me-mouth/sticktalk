//! Unwind containment. Never format an upstream panic payload (it can contain SDP).
use crate::Error;
use std::{
    cell::Cell,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::Once,
};

thread_local! { static IN_PEER: Cell<bool> = const { Cell::new(false) }; }
static HOOK: Once = Once::new();

pub(crate) fn run<T>(operation: impl FnOnce() -> Result<T, Error>) -> Result<T, Error> {
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if !IN_PEER.with(Cell::get) {
                previous(info);
            }
        }));
    });
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            IN_PEER.with(|flag| flag.set(self.0));
        }
    }
    let _restore = Restore(IN_PEER.with(|flag| flag.replace(true)));
    catch_unwind(AssertUnwindSafe(operation)).unwrap_or(Err(Error::PeerFailed))
}
