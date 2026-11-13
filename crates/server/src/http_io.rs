//! Actual socket write-idle deadlines, including when Hyper is backpressured and
//! never polls a response body again. No timer exists on an idle keepalive socket.
use std::{
    future::Future,
    io,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

pub struct Listener(pub tokio::net::TcpListener);
impl axum::serve::Listener for Listener {
    type Io = WriteDeadline<tokio::net::TcpStream>;
    type Addr = std::net::SocketAddr;
    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        let (io, addr) = axum::serve::Listener::accept(&mut self.0).await;
        (WriteDeadline::new(io, Duration::from_secs(30)), addr)
    }
    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.0.local_addr()
    }
}
pub struct WriteDeadline<T> {
    io: T,
    idle: Duration,
    stalled: Option<Pin<Box<tokio::time::Sleep>>>,
}
impl<T> WriteDeadline<T> {
    fn new(io: T, idle: Duration) -> Self {
        Self {
            io,
            idle,
            stalled: None,
        }
    }
    fn check<R>(
        &mut self,
        cx: &mut Context<'_>,
        result: Poll<io::Result<R>>,
    ) -> Poll<io::Result<R>> {
        match result {
            Poll::Ready(result) => {
                self.stalled = None;
                Poll::Ready(result)
            }
            Poll::Pending => {
                let deadline = self
                    .stalled
                    .get_or_insert_with(|| Box::pin(tokio::time::sleep(self.idle)));
                if deadline.as_mut().poll(cx).is_ready() {
                    Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "HTTP socket write stalled",
                    )))
                } else {
                    Poll::Pending
                }
            }
        }
    }
}
impl<T: AsyncRead + Unpin> AsyncRead for WriteDeadline<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_read(cx, buf)
    }
}
impl<T: AsyncWrite + Unpin> AsyncWrite for WriteDeadline<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let result = Pin::new(&mut this.io).poll_write(cx, bytes);
        this.check(cx, result)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        // TcpStream::flush is a no-op, not write progress. In particular, a
        // flush between pending writes must not restart the stalled-write clock.
        Pin::new(&mut self.get_mut().io).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let result = Pin::new(&mut this.io).poll_shutdown(cx);
        this.check(cx, result)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    #[tokio::test]
    async fn s3_stalled_socket_writer_times_out_without_body_polling() {
        let (io, _unread) = tokio::io::duplex(1);
        let mut writer = WriteDeadline::new(io, Duration::from_millis(20));
        let result = tokio::time::timeout(Duration::from_secs(1), writer.write_all(b"blocked"))
            .await
            .unwrap();
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
        let (io, _unread) = tokio::io::duplex(1);
        let mut writer = WriteDeadline::new(io, Duration::from_millis(20));
        writer.write_all(b"x").await.unwrap();
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            futures_util::future::poll_fn(|cx| {
                let _ = Pin::new(&mut writer).poll_flush(cx);
                Pin::new(&mut writer).poll_write(cx, b"blocked")
            }),
        )
        .await
        .unwrap();
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
    }
}
