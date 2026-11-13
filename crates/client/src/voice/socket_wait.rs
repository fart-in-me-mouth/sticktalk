//! Active-call-only Winsock readiness + one high-resolution deadline timer.
//! SO_RCVTIMEO rounds sub-millisecond pacer deadlines to scheduler ticks.
use std::{io, net::UdpSocket, os::windows::io::AsRawSocket, time::Instant};
use sticktalk_sfu::pacing::Timer;
use windows_sys::Win32::{
    Foundation::{HANDLE, WAIT_OBJECT_0},
    Networking::WinSock::*,
    System::Threading::{WaitForMultipleObjects, INFINITE},
};

pub(super) struct SocketWait<'a> {
    socket: &'a UdpSocket,
    event: WSAEVENT,
    timer: Timer,
}
impl<'a> SocketWait<'a> {
    pub fn new(socket: &'a UdpSocket) -> io::Result<Self> {
        let timer = Timer::new()?;
        let event = unsafe { WSACreateEvent() };
        if event == 0 {
            return Err(io::Error::last_os_error());
        }
        if unsafe {
            WSAEventSelect(
                socket.as_raw_socket() as SOCKET,
                event,
                (FD_READ | FD_CLOSE) as i32,
            )
        } != 0
        {
            let error = io::Error::from_raw_os_error(unsafe { WSAGetLastError() });
            unsafe {
                WSACloseEvent(event);
            }
            return Err(error);
        }
        Ok(Self {
            socket,
            event,
            timer,
        })
    }
    pub fn until(&self, deadline: Instant) -> io::Result<()> {
        if deadline <= Instant::now() {
            return Ok(());
        }
        self.timer.arm(deadline)?;
        let handles = [self.event as HANDLE, self.timer.raw_handle() as HANDLE];
        let result = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE) };
        self.timer.cancel();
        if result == WAIT_OBJECT_0 {
            let mut events = WSANETWORKEVENTS::default();
            if unsafe {
                WSAEnumNetworkEvents(
                    self.socket.as_raw_socket() as SOCKET,
                    self.event,
                    &mut events,
                )
            } != 0
            {
                return Err(io::Error::from_raw_os_error(unsafe { WSAGetLastError() }));
            }
        } else if result != WAIT_OBJECT_0 + 1 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}
impl Drop for SocketWait<'_> {
    fn drop(&mut self) {
        unsafe {
            WSAEventSelect(self.socket.as_raw_socket() as SOCKET, 0, 0);
            WSACloseEvent(self.event);
        }
        let _ = self.socket.set_nonblocking(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn readiness_wakes_without_a_poll_timer_and_deadlines_remain_bounded() {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let wait = SocketWait::new(&socket).unwrap();
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        sender.send_to(&[42], socket.local_addr().unwrap()).unwrap();
        wait.until(Instant::now() + Duration::from_secs(1)).unwrap();
        assert_eq!(socket.recv_from(&mut [0; 8]).unwrap().0, 1);
        // Clear a possible level-triggered readiness notification before timing.
        wait.until(Instant::now() + Duration::from_millis(1))
            .unwrap();
        let deadline = Instant::now() + Duration::from_millis(2);
        wait.until(deadline).unwrap();
        assert!(Instant::now() >= deadline);
    }
}
