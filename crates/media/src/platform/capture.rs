use super::runtime::Runtime;
use crate::{Result, Size, Source, SourceKind};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{sync_channel, Receiver},
    Arc,
};
use windows::{
    core::{factory, Interface},
    Foundation::TypedEventHandler,
    Graphics::{
        Capture::*,
        DirectX::{Direct3D11::IDirect3DDevice, DirectXPixelFormat},
    },
    Win32::{
        Foundation::{HWND, LPARAM, RECT},
        Graphics::{Direct3D11::ID3D11Texture2D, Dxgi::IDXGIDevice, Gdi::*},
        System::WinRT::{Direct3D11::*, Graphics::Capture::IGraphicsCaptureItemInterop},
        UI::WindowsAndMessaging::*,
    },
};

// Keep enumeration dimensions aligned with WGC's visible frame rather than
// GetWindowRect's invisible resize border. Stable Win32 ABI, no extra dependency.
#[link(name = "dwmapi")]
extern "system" {
    #[link_name = "DwmGetWindowAttribute"]
    fn dwm_attribute(
        hwnd: *mut std::ffi::c_void,
        attribute: u32,
        out: *mut std::ffi::c_void,
        bytes: u32,
    ) -> i32;
}
/// Lists active monitors, then visible, non-minimized top-level windows.
pub fn list_sources() -> Result<Vec<Source>> {
    let mut sources = Vec::<Source>::new();
    // Callbacks borrow this vector only for the synchronous enumeration call.
    unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(monitor),
            LPARAM(&mut sources as *mut _ as isize),
        )
        .ok()?;
        EnumWindows(Some(window), LPARAM(&mut sources as *mut _ as isize))?;
    }
    Ok(sources)
}
unsafe extern "system" fn monitor(
    handle: HMONITOR,
    _: windows::Win32::Graphics::Gdi::HDC,
    _: *mut RECT,
    data: LPARAM,
) -> windows::core::BOOL {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(handle, &mut info as *mut _ as *mut MONITORINFO).as_bool() {
        let rect = info.monitorInfo.rcMonitor;
        let end = info
            .szDevice
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(info.szDevice.len());
        let sources = &mut *(data.0 as *mut Vec<Source>);
        sources.push(Source {
            kind: SourceKind::Monitor,
            id: handle.0 as isize,
            name: String::from_utf16_lossy(&info.szDevice[..end]),
            size: Size {
                width: (rect.right - rect.left) as u32,
                height: (rect.bottom - rect.top) as u32,
            },
            primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        });
    }
    true.into()
}
unsafe extern "system" fn window(handle: HWND, data: LPARAM) -> windows::core::BOOL {
    if handle == GetShellWindow()
        || handle == GetDesktopWindow()
        || !IsWindowVisible(handle).as_bool()
        || IsIconic(handle).as_bool()
    {
        return true.into();
    }
    let len = GetWindowTextLengthW(handle);
    if len == 0 {
        return true.into();
    }
    let mut text = vec![0u16; (len + 1) as usize];
    let used = GetWindowTextW(handle, &mut text);
    let mut rect = RECT::default();
    if used > 0
        && dwm_attribute(
            handle.0,
            9,
            (&mut rect as *mut RECT).cast(),
            std::mem::size_of::<RECT>() as u32,
        ) >= 0
        && rect.right > rect.left
        && rect.bottom > rect.top
    {
        let sources = &mut *(data.0 as *mut Vec<Source>);
        sources.push(Source {
            kind: SourceKind::Window,
            id: handle.0 as isize,
            name: String::from_utf16_lossy(&text[..used as usize]),
            size: Size {
                width: (rect.right - rect.left) as u32,
                height: (rect.bottom - rect.top) as u32,
            },
            primary: false,
        });
    }
    true.into()
}

pub(super) struct Capture {
    pub pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    item: GraphicsCaptureItem,
    device: IDirect3DDevice,
    token: i64,
    closed_token: i64,
    closed: Arc<AtomicBool>,
    pub signals: Receiver<()>,
    pub size: Size,
    _runtime_marker: std::marker::PhantomData<Runtime>,
}
impl Capture {
    pub fn new(
        device: &windows::Win32::Graphics::Direct3D11::ID3D11Device,
        source: &Source,
        cursor: bool,
    ) -> Result<Self> {
        if !GraphicsCaptureSession::IsSupported()? {
            return Err(crate::Error::Unsupported);
        }
        unsafe {
            let interop: IGraphicsCaptureItemInterop =
                factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
            let item: GraphicsCaptureItem = match source.kind {
                SourceKind::Monitor => interop.CreateForMonitor(HMONITOR(source.id as *mut _))?,
                SourceKind::Window => interop.CreateForWindow(HWND(source.id as *mut _))?,
            };
            let dxgi: IDXGIDevice = device.cast()?;
            let direct: IDirect3DDevice = CreateDirect3D11DeviceFromDXGIDevice(&dxgi)?.cast()?;
            let native = item.Size()?;
            let size = Size {
                width: native.Width as u32,
                height: native.Height as u32,
            }
            .validate()?;
            let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
                &direct,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                3,
                native,
            )?;
            let session = pool.CreateCaptureSession(&item)?;
            session.SetIsCursorCaptureEnabled(cursor)?;
            let (tx, signals) = sync_channel(1);
            let token = pool.FrameArrived(&TypedEventHandler::new(move |_, _| {
                let _ = tx.try_send(());
                Ok(())
            }))?;
            // Newer Windows exposes this throttle. Zero asks for every update;
            // older Windows returns E_NOINTERFACE and keeps its default pacing.
            let _ = session.SetMinUpdateInterval(windows::Foundation::TimeSpan { Duration: 0 });
            let closed = Arc::new(AtomicBool::new(false));
            let closed_callback = closed.clone();
            let closed_token = item.Closed(&TypedEventHandler::new(move |_, _| {
                closed_callback.store(true, Ordering::Release);
                Ok(())
            }))?;
            session.StartCapture()?;
            Ok(Self {
                pool,
                session,
                item,
                device: direct,
                token,
                closed_token,
                closed,
                signals,
                size,
                _runtime_marker: std::marker::PhantomData,
            })
        }
    }

    pub fn next(&mut self) -> Result<Option<(Direct3D11CaptureFrame, ID3D11Texture2D, u64)>> {
        if self.closed.load(Ordering::Acquire) {
            return Err(crate::Error::Stopped);
        }
        let mut latest = None;
        let mut count = 0;
        // WGC returns null rather than an error when no frame is available.
        for _ in 0..3 {
            let frame = match self.pool.TryGetNextFrame() {
                Ok(f) => f,
                Err(e)
                    if (e.code() == windows::core::HRESULT(0x80004003u32 as i32)
                        || e.code().0 == 0) =>
                {
                    break
                }
                Err(e) => return Err(e.into()),
            };
            count += 1;
            latest = Some(frame);
        }
        if let Some(frame) = latest {
            let native = frame.ContentSize()?;
            let size = Size {
                width: native.Width as u32,
                height: native.Height as u32,
            }
            .validate()?;
            if size != self.size {
                self.pool.Recreate(
                    &self.device,
                    DirectXPixelFormat::B8G8R8A8UIntNormalized,
                    3,
                    native,
                )?;
                self.size = size;
                // Old pool texture dimensions do not necessarily match ContentSize.
                return Ok(None);
            }
            let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
            let texture = unsafe { access.GetInterface::<ID3D11Texture2D>()? };
            Ok(Some((frame, texture, count)))
        } else {
            Ok(None)
        }
    }

    pub fn set_cursor(&self, enabled: bool) -> Result<()> {
        self.session.SetIsCursorCaptureEnabled(enabled)?;
        Ok(())
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.pool.RemoveFrameArrived(self.token);
        let _ = self.item.RemoveClosed(self.closed_token);
        let _ = self.session.Close();
        let _ = self.pool.Close();
        // Keep item alive until after session closure.
        let _ = &self.item;
    }
}
