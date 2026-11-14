//! Browser host adapters. No Tokio, OS window, filesystem or worker threads.
use crate::{
    http::{Body, ByteStream, HttpRequest, HttpResponse, HttpTransport},
    io::{Clock, Entropy, MaybeSend, Spawner, Task},
    protocol::{ConnectError, Frame, Incoming, Outgoing, Transport},
    upload::UploadSource,
};
use futures_channel::{mpsc, oneshot};
use futures_util::{
    future::{AbortHandle, Abortable},
    Stream, StreamExt,
};
use std::{
    cell::{Cell, RefCell},
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};
use wasm_bindgen::{closure::Closure, JsCast};
use wasm_bindgen_futures::JsFuture;
use web_sys::{CloseEvent, Event, MessageEvent, WebSocket};
fn window() -> web_sys::Window {
    web_sys::window().expect("browser window")
}
#[derive(Clone, Copy)]
pub struct Browser;
struct Timer {
    id: i32,
    _callback: Closure<dyn FnMut()>,
}
impl Drop for Timer {
    fn drop(&mut self) {
        window().clear_timeout_with_handle(self.id);
    }
}
fn timer(ms: u64, callback: impl FnMut() + 'static) -> Result<Timer, String> {
    let callback = Closure::wrap(Box::new(callback) as Box<dyn FnMut()>);
    let id = window()
        .set_timeout_with_callback_and_timeout_and_arguments_0(
            callback.as_ref().unchecked_ref(),
            ms.min(i32::MAX as u64) as i32,
        )
        .map_err(|_| "Could not schedule timer".to_string())?;
    Ok(Timer {
        id,
        _callback: callback,
    })
}
impl Clock for Browser {
    fn monotonic_ms(&self) -> u64 {
        window().performance().expect("performance clock").now() as u64
    }
    fn unix_ms(&self) -> u64 {
        js_sys::Date::now() as u64
    }
    fn sleep(&self, ms: u64) -> Task<'static, ()> {
        Box::pin(async move {
            let (tx, rx) = oneshot::channel();
            let mut tx = Some(tx);
            let _timer = timer(ms, move || {
                if let Some(tx) = tx.take() {
                    let _ = tx.send(());
                }
            })
            .expect("browser timer");
            let _ = rx.await;
        })
    }
}
impl Entropy for Browser {
    fn fill(&self, bytes: &mut [u8]) -> Result<(), String> {
        window()
            .crypto()
            .map_err(|_| "Secure random source unavailable".to_string())?
            .get_random_values_with_u8_array(bytes)
            .map_err(|_| "Secure random source unavailable".to_string())?;
        Ok(())
    }
}
struct CancelTask(AbortHandle);
impl Drop for CancelTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}
impl Spawner for Browser {
    fn spawn<T: MaybeSend + 'static>(
        &self,
        task: Task<'static, T>,
    ) -> Task<'static, Result<T, String>> {
        let (abort, registration) = AbortHandle::new_pair();
        let (tx, rx) = oneshot::channel();
        wasm_bindgen_futures::spawn_local(async move {
            let result = Abortable::new(task, registration)
                .await
                .map_err(|_| "Network task cancelled".into());
            let _ = tx.send(result);
        });
        let guard = CancelTask(abort);
        Box::pin(async move {
            let _guard = guard;
            rx.await.map_err(|_| "Network task cancelled".to_string())?
        })
    }
}
struct Socket {
    ws: WebSocket,
    _open: Closure<dyn FnMut(Event)>,
    _error: Closure<dyn FnMut(Event)>,
    _message: Closure<dyn FnMut(MessageEvent)>,
    _close: Closure<dyn FnMut(CloseEvent)>,
}
impl Drop for Socket {
    fn drop(&mut self) {
        self.ws.set_onopen(None);
        self.ws.set_onerror(None);
        self.ws.set_onmessage(None);
        self.ws.set_onclose(None);
        let _ = self.ws.close();
    }
}
struct Receive {
    rx: mpsc::Receiver<Result<Frame, String>>,
    queued: Rc<Cell<usize>>,
    _socket: Rc<Socket>,
}
impl Stream for Receive {
    type Item = Result<Frame, String>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let result = Pin::new(&mut self.rx).poll_next(cx);
        if let Poll::Ready(Some(Ok(Frame::Binary(bytes)))) = &result {
            self.queued
                .set(self.queued.get().saturating_sub(bytes.len()));
        }
        result
    }
}
impl Transport for Browser {
    fn connect(&self, url: String) -> Task<'_, Result<(Outgoing, Incoming), ConnectError>> {
        Box::pin(async move {
            let ws = WebSocket::new_with_str(&url, "sticktalk.v1")
                .map_err(|_| "Connection failed".to_string())?;
            ws.set_binary_type(web_sys::BinaryType::Arraybuffer);
            let (tx, rx) = mpsc::channel(128);
            let tx = Rc::new(RefCell::new(tx));
            let queued = Rc::new(Cell::new(0usize));
            let (opened, ready) = oneshot::channel();
            let opened = Rc::new(RefCell::new(Some(opened)));
            let open = {
                let opened = opened.clone();
                Closure::wrap(Box::new(move |_: Event| {
                    if let Some(tx) = opened.borrow_mut().take() {
                        let _ = tx.send(Ok(()));
                    }
                }) as Box<dyn FnMut(Event)>)
            };
            let error = {
                let opened = opened.clone();
                let tx = tx.clone();
                Closure::wrap(Box::new(move |_: Event| {
                    if let Some(tx) = opened.borrow_mut().take() {
                        let _ = tx.send(Err("Connection failed".to_string()));
                    }
                    let _ = tx.borrow_mut().try_send(Err("Socket error".into()));
                }) as Box<dyn FnMut(Event)>)
            };
            let message = {
                let tx = tx.clone();
                let queued = queued.clone();
                let ws = ws.clone();
                Closure::wrap(Box::new(move |event: MessageEvent| {
                    let frame = if let Ok(buffer) = event.data().dyn_into::<js_sys::ArrayBuffer>() {
                        let count = buffer.byte_length() as usize;
                        if count > sticktalk_proto::MAX_WS_FRAME_BYTES as usize
                            || queued.get() + count > 2 * 1024 * 1024
                        {
                            let _ = tx.borrow_mut().try_send(Ok(Frame::Close(Some(1009))));
                            tx.borrow_mut().close_channel();
                            let _ = ws.close();
                            return;
                        }
                        queued.set(queued.get() + count);
                        Frame::Binary(js_sys::Uint8Array::new(&buffer).to_vec())
                    } else {
                        Frame::Text
                    };
                    let sent = tx.borrow_mut().try_send(Ok(frame)).is_ok();
                    if !sent {
                        let _ = tx.borrow_mut().try_send(Ok(Frame::Close(Some(1009))));
                        tx.borrow_mut().close_channel();
                        let _ = ws.close();
                    }
                }) as Box<dyn FnMut(MessageEvent)>)
            };
            let close = {
                let tx = tx.clone();
                let opened = opened.clone();
                Closure::wrap(Box::new(move |event: CloseEvent| {
                    if let Some(tx) = opened.borrow_mut().take() {
                        let _ = tx.send(Err("Connection closed".to_string()));
                    }
                    let mut tx = tx.borrow_mut();
                    let _ = tx.try_send(Ok(Frame::Close(Some(event.code()))));
                    tx.close_channel();
                }) as Box<dyn FnMut(CloseEvent)>)
            };
            ws.set_onopen(Some(open.as_ref().unchecked_ref()));
            ws.set_onerror(Some(error.as_ref().unchecked_ref()));
            ws.set_onmessage(Some(message.as_ref().unchecked_ref()));
            ws.set_onclose(Some(close.as_ref().unchecked_ref()));
            let socket = Rc::new(Socket {
                ws,
                _open: open,
                _error: error,
                _message: message,
                _close: close,
            });
            ready.await.map_err(|_| "Connection failed".to_string())??;
            if socket.ws.protocol() != "sticktalk.v1" {
                return Err(ConnectError::Stop(
                    "Server did not negotiate sticktalk.v1".into(),
                ));
            }
            let write = futures_util::sink::unfold(socket.clone(), |socket, frame| async move {
                if socket.ws.buffered_amount() > 2 * 1024 * 1024 {
                    return Err("Socket send queue is full".into());
                }
                match frame {
                    Frame::Binary(bytes) => {
                        if socket.ws.buffered_amount() as usize + bytes.len() > 2 * 1024 * 1024 {
                            return Err("Socket send queue is full".into());
                        }
                        socket
                            .ws
                            .send_with_u8_array(&bytes)
                            .map_err(|_| "Socket send failed".to_string())?;
                    }
                    Frame::Close(_) => {
                        let _ = socket.ws.close();
                    }
                    _ => {}
                }
                Ok(socket)
            });
            let read = Receive {
                rx,
                queued,
                _socket: socket,
            };
            Ok((Box::pin(write) as Outgoing, Box::pin(read) as Incoming))
        })
    }
}
/// Fetch policy. CSRF is supplied by the web session bootstrap, never persisted here.
#[derive(Clone, Default)]
pub struct Fetch {
    pub csrf_token: Option<String>,
}
struct AbortFetch(web_sys::AbortController);
impl Drop for AbortFetch {
    fn drop(&mut self) {
        self.0.abort();
    }
}
impl HttpTransport for Fetch {
    fn execute(&self, request: HttpRequest) -> Task<'_, Result<HttpResponse, String>> {
        Box::pin(async move {
            let safe_read = request.method == "GET" && matches!(&request.body, Body::Empty);
            let mut retry = crate::http::ReadRetry::new(safe_read);
            let controller = web_sys::AbortController::new()
                .map_err(|_| "Could not start request".to_string())?;
            let abort = controller.clone();
            let _timeout = timer(request.timeout_ms, move || abort.abort())?;
            let _abort = AbortFetch(controller.clone());
            let init = web_sys::RequestInit::new();
            init.set_method(request.method);
            init.set_redirect(web_sys::RequestRedirect::Error);
            init.set_credentials(web_sys::RequestCredentials::SameOrigin);
            init.set_signal(Some(&controller.signal()));
            let bytes = match request.body {
                Body::Empty => None,
                Body::Bytes(bytes) => Some(bytes),
                Body::Stream { mut chunks, length } => {
                    // Fetch requires a concrete upload body across Safari/Chrome. Bound it to
                    // one protocol chunk, never the full file, and hash via 64 KiB slices.
                    if length > 16 * 1024 * 1024 {
                        return Err("Upload chunk exceeds limit".into());
                    }
                    let mut bytes = Vec::with_capacity(length as usize);
                    while let Some(chunk) = chunks.next().await {
                        let chunk = chunk?;
                        if chunk.len() > (length as usize).saturating_sub(bytes.len()) {
                            return Err("Upload source exceeds declared length".into());
                        }
                        bytes.extend_from_slice(&chunk);
                    }
                    Some(bytes)
                }
            };
            if let Some(bytes) = bytes {
                let body = js_sys::Uint8Array::from(bytes.as_slice());
                init.set_body(&body);
            }
            let req = web_sys::Request::new_with_str_and_init(&request.url, &init)
                .map_err(|_| "Invalid request".to_string())?;
            for (key, value) in request.headers {
                if !key.eq_ignore_ascii_case("content-length") {
                    req.headers()
                        .set(&key, &value)
                        .map_err(|_| "Invalid request header".to_string())?;
                }
            }
            if let Some(token) = &self.csrf_token {
                if request.method != "GET" {
                    req.headers()
                        .set("X-CSRF-Token", token)
                        .map_err(|_| "Invalid CSRF token".to_string())?;
                }
            }
            loop {
                let response = JsFuture::from(window().fetch_with_request(&req))
                    .await
                    .map_err(|_| request.network_error.to_string())?
                    .dyn_into::<web_sys::Response>()
                    .map_err(|_| "Invalid response".to_string())?;
                let status = response.status();
                if response
                    .headers()
                    .get("Content-Length")
                    .ok()
                    .flatten()
                    .and_then(|v| v.parse::<u64>().ok())
                    .is_some_and(|v| v > request.max_response as u64)
                {
                    return Err("Server response exceeds limit".into());
                }
                let mut bytes = Vec::new();
                if let Some(body) = response.body() {
                    let reader = body
                        .get_reader()
                        .dyn_into::<web_sys::ReadableStreamDefaultReader>()
                        .map_err(|_| "Could not read response".to_string())?;
                    loop {
                        let result = JsFuture::from(reader.read())
                            .await
                            .map_err(|_| "Network interrupted".to_string())?;
                        if js_sys::Reflect::get(&result, &"done".into())
                            .ok()
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false)
                        {
                            break;
                        }
                        let value = js_sys::Reflect::get(&result, &"value".into())
                            .map_err(|_| "Invalid response chunk".to_string())?;
                        let chunk = js_sys::Uint8Array::new(&value);
                        if chunk.length() as usize
                            > request.max_response.saturating_sub(bytes.len())
                        {
                            let _ = reader.cancel();
                            return Err("Server response exceeds limit".into());
                        }
                        bytes.extend_from_slice(&chunk.to_vec());
                    }
                    reader.release_lock();
                }
                let response = HttpResponse { status, bytes };
                if let Some(delay) = retry.delay(&response) {
                    Browser.sleep(delay).await;
                    continue;
                }
                return Ok(response);
            }
        })
    }
}
pub struct BlobSource {
    pub blob: web_sys::Blob,
    pub filename: String,
}
impl UploadSource for BlobSource {
    fn filename(&self) -> String {
        self.filename.clone()
    }
    fn byte_size(&self) -> Task<'_, Result<u64, String>> {
        Box::pin(async { Ok(self.blob.size() as u64) })
    }
    fn chunks(&self, offset: u64, count: u64) -> Task<'_, Result<ByteStream, String>> {
        let blob = self.blob.clone();
        Box::pin(async move {
            let stream = futures_util::stream::try_unfold(
                (blob, offset, count),
                |(blob, offset, count)| async move {
                    if count == 0 {
                        return Ok(None);
                    }
                    let take = count.min(65536);
                    let slice = blob
                        .slice_with_f64_and_f64(offset as f64, (offset + take) as f64)
                        .map_err(|_| "Could not read file".to_string())?;
                    let buffer = JsFuture::from(slice.array_buffer())
                        .await
                        .map_err(|_| "Could not read file".to_string())?;
                    let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
                    if bytes.len() != take as usize {
                        return Err("File changed during upload".into());
                    }
                    Ok(Some((bytes, (blob, offset + take, count - take))))
                },
            );
            Ok(Box::pin(stream) as ByteStream)
        })
    }
}
