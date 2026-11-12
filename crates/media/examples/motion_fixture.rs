//! GPU animated diagnostic stimulus, NOT application UI. Runs in a separate
//! process, so its CPU/RSS are excluded from sharing-process measurements.
#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::time::{Duration, Instant};
    use windows::{
        core::{w, Interface},
        Win32::{
            Foundation::*,
            Graphics::{
                Direct3D::*,
                Direct3D11::*,
                Dxgi::{Common::*, *},
            },
            System::LibraryLoader::GetModuleHandleW,
            UI::WindowsAndMessaging::*,
        },
    };
    unsafe extern "system" fn procedure(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if message == WM_DESTROY {
            PostQuitMessage(0);
            return LRESULT(0);
        }
        DefWindowProcW(window, message, wparam, lparam)
    }
    let seconds = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "180".into())
        .parse::<u64>()?;
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = w!("StickTalkMediaBenchMotion");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: instance.into(),
            lpszClassName: class,
            ..Default::default()
        };
        if RegisterClassW(&wc) == 0 {
            return Err(windows::core::Error::from_win32().into());
        }
        let width = GetSystemMetrics(SM_CXSCREEN);
        let height = GetSystemMetrics(SM_CYSCREEN);
        let window = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            class,
            w!("StickTalk GPU benchmark pattern (closes automatically)"),
            WS_POPUP | WS_VISIBLE,
            0,
            0,
            width,
            height,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        let desc = DXGI_SWAP_CHAIN_DESC {
            BufferDesc: DXGI_MODE_DESC {
                Width: width as u32,
                Height: height as u32,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                ..Default::default()
            },
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            OutputWindow: window,
            Windowed: true.into(),
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            ..Default::default()
        };
        let mut swap = None;
        let mut device = None;
        let mut context = None;
        D3D11CreateDeviceAndSwapChain(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&[D3D_FEATURE_LEVEL_11_0]),
            D3D11_SDK_VERSION,
            Some(&desc),
            Some(&mut swap),
            Some(&mut device),
            None,
            Some(&mut context),
        )?;
        let swap = swap.ok_or("null swapchain")?;
        let device = device.ok_or("null D3D device")?;
        let context = context.ok_or("null D3D context")?;
        let context1: ID3D11DeviceContext1 = context.cast()?;
        let texture: ID3D11Texture2D = swap.GetBuffer(0)?;
        let mut target = None;
        device.CreateRenderTargetView(&texture, None, Some(&mut target))?;
        let target = target.ok_or("null RTV")?;
        let start = Instant::now();
        let mut frame = 0i32;
        while start.elapsed() < Duration::from_secs(seconds) {
            let mut msg = MSG::default();
            let mut quit = false;
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    quit = true;
                    break;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            if quit {
                break;
            }
            let phase = frame as f32 * 0.017;
            context.ClearRenderTargetView(
                &target,
                &[phase.sin() * 0.4 + 0.5, phase.cos() * 0.4 + 0.5, 0.2, 1.0],
            );
            for row in 0..9 {
                for col in 0..16 {
                    let x = (col * width / 16 + frame * 7) % width;
                    let y = row * height / 9;
                    let rect = RECT {
                        left: x,
                        top: y,
                        right: (x + width / 32).min(width),
                        bottom: y + height / 18,
                    };
                    context1.ClearView(
                        &target,
                        &[
                            col as f32 / 16.0,
                            row as f32 / 9.0,
                            phase.sin() * 0.4 + 0.5,
                            1.0,
                        ],
                        Some(&[rect]),
                    );
                }
            }
            swap.Present(1, DXGI_PRESENT(0)).ok()?;
            frame += 1;
        }
        println!(
            "GPU fixture presented {frame} frames / {:.3}s = {:.3} fps",
            start.elapsed().as_secs_f64(),
            f64::from(frame) / start.elapsed().as_secs_f64()
        );
        DestroyWindow(window)?;
    }
    Ok(())
}
#[cfg(not(windows))]
fn main() {
    eprintln!("Windows only");
}
