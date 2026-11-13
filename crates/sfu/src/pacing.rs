//! One-shot RTC deadlines. Windows' coarse async/socket timers must not turn
//! str0m's microsecond "continue draining" deadlines into one packet per tick.
//! No spinning, global timer-resolution changes or idle interval. A Windows
//! clock owns one sleeping helper, joined on drop; cancellation disarms it.
use std::{io, time::Instant};

pub struct Clock {
    #[cfg(windows)]
    native: native::Clock,
}
impl Clock {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            #[cfg(windows)]
            native: native::Clock::new()?,
        })
    }
    pub async fn wait(&mut self, deadline: Instant) -> io::Result<()> {
        if deadline <= Instant::now() {
            return Ok(());
        }
        #[cfg(windows)]
        {
            self.native.wait(deadline).await
        }
        #[cfg(not(windows))]
        {
            tokio::time::sleep_until(deadline.into()).await;
            Ok(())
        }
    }
}

#[cfg(windows)]
pub use native::Timer;
#[cfg(windows)]
mod native {
    use super::*;
    use std::{
        future::Future,
        pin::Pin,
        sync::{Arc, Mutex},
        task::{Context, Poll, Waker},
        thread::{self, JoinHandle},
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0},
        System::Threading::*,
    };

    /// Also used with a Winsock readiness event by the blocking native client.
    pub struct Timer(usize);
    impl Timer {
        pub fn new() -> io::Result<Self> {
            let handle = unsafe {
                CreateWaitableTimerExW(
                    std::ptr::null(),
                    std::ptr::null(),
                    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                    TIMER_ALL_ACCESS,
                )
            };
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            Ok(Self(handle as usize))
        }
        pub fn raw_handle(&self) -> usize {
            self.0
        }
        pub fn arm(&self, deadline: Instant) -> io::Result<()> {
            let left = deadline.saturating_duration_since(Instant::now());
            let due = -((left.as_nanos() / 100).clamp(1, i64::MAX as u128) as i64);
            if unsafe {
                SetWaitableTimerEx(
                    self.0 as HANDLE,
                    &due,
                    0,
                    None,
                    std::ptr::null(),
                    std::ptr::null(),
                    0,
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
        pub fn cancel(&self) {
            unsafe {
                CancelWaitableTimer(self.0 as HANDLE);
            }
        }
    }
    impl Drop for Timer {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0 as HANDLE);
            }
        }
    }
    #[derive(Default)]
    struct State {
        armed: bool,
        failed: bool,
        waker: Option<Waker>,
    }
    pub(super) struct Clock {
        timer: Timer,
        stop: usize,
        state: Arc<Mutex<State>>,
        worker: Option<JoinHandle<()>>,
    }
    impl Clock {
        pub fn new() -> io::Result<Self> {
            let timer = Timer::new()?;
            let stop = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
            if stop.is_null() {
                return Err(io::Error::last_os_error());
            }
            let stop = stop as usize;
            let state = Arc::new(Mutex::new(State::default()));
            let shared = state.clone();
            let raw = timer.raw_handle();
            let worker = thread::Builder::new()
                .name("sticktalk-rtc-clock".into())
                .stack_size(128 * 1024)
                .spawn(move || {
                    let handles = [stop as HANDLE, raw as HANDLE];
                    loop {
                        let event =
                            unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE) };
                        if event != WAIT_OBJECT_0 + 1 {
                            if event != WAIT_OBJECT_0 {
                                let waker = {
                                    let mut state = shared.lock().unwrap();
                                    state.failed = true;
                                    state.waker.take()
                                };
                                if let Some(waker) = waker {
                                    waker.wake();
                                }
                            }
                            break;
                        }
                        let waker = {
                            let mut state = shared.lock().unwrap();
                            state.armed = false;
                            state.waker.take()
                        };
                        if let Some(waker) = waker {
                            waker.wake();
                        }
                    }
                });
            match worker {
                Ok(worker) => Ok(Self {
                    timer,
                    stop,
                    state,
                    worker: Some(worker),
                }),
                Err(error) => {
                    unsafe {
                        CloseHandle(stop as HANDLE);
                    }
                    Err(error)
                }
            }
        }
        pub fn wait(&mut self, deadline: Instant) -> Wait<'_> {
            Wait {
                clock: self,
                deadline,
            }
        }
    }
    impl Drop for Clock {
        fn drop(&mut self) {
            unsafe {
                SetEvent(self.stop as HANDLE);
            }
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
            unsafe {
                CloseHandle(self.stop as HANDLE);
            }
        }
    }
    pub(super) struct Wait<'a> {
        clock: &'a mut Clock,
        deadline: Instant,
    }
    impl Future for Wait<'_> {
        type Output = io::Result<()>;
        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            if Instant::now() >= self.deadline {
                return Poll::Ready(Ok(()));
            }
            let mut state = self.clock.state.lock().unwrap();
            if state.failed {
                return Poll::Ready(Err(io::Error::other("RTC deadline worker failed")));
            }
            state.waker = Some(cx.waker().clone());
            if !state.armed {
                if let Err(error) = self.clock.timer.arm(self.deadline) {
                    return Poll::Ready(Err(error));
                }
                state.armed = true;
            }
            Poll::Pending
        }
    }
    impl Drop for Wait<'_> {
        fn drop(&mut self) {
            let mut state = self.clock.state.lock().unwrap();
            self.clock.timer.cancel();
            state.armed = false;
            state.waker = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[tokio::test]
    async fn expired_deadlines_are_immediate_and_cancelled_waits_cannot_fire_the_next_wait() {
        let mut clock = Clock::new().unwrap();
        clock
            .wait(Instant::now() - Duration::from_secs(1))
            .await
            .unwrap();
        tokio::select! {
            result = clock.wait(Instant::now() + Duration::from_secs(10)) => panic!("unexpected timer: {result:?}"),
            _ = tokio::time::sleep(Duration::from_millis(1)) => {},
        }
        let deadline = Instant::now() + Duration::from_millis(2);
        clock.wait(deadline).await.unwrap();
        assert!(Instant::now() >= deadline);
        let before = Instant::now();
        drop(clock);
        assert!(before.elapsed() < Duration::from_secs(1));
    }
}
