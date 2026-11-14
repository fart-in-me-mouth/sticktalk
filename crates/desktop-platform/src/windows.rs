//! Native effects are explicit calls, never constructors in examples/tests.
//! Call UI functions on the host's UI thread. Initialize COM/WinRT once there.
use crate::{
    notification::{MessageToast, NotificationContext, NotificationPolicy},
    sound::Sound,
    tray::{self, TrayAction},
    Error, Result, APP_USER_MODEL_ID,
};
use std::{marker::PhantomData, path::Path};
use windows::{
    core::{HSTRING, PCWSTR},
    Data::Xml::Dom::XmlDocument,
    Win32::{
        Foundation::{HWND, POINT},
        Media::Audio::{PlaySoundW, SND_MEMORY, SND_NODEFAULT, SND_SYNC},
        System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER},
        UI::{Shell::*, WindowsAndMessaging::*},
    },
    UI::{
        Notifications::{ToastNotification, ToastNotificationManager},
        Shell::FocusSessionManager,
    },
};

pub mod host;
pub mod instance;

/// COM apartment guard, deliberately !Send / !Sync.
pub struct Apartment(PhantomData<std::rc::Rc<()>>);
impl Apartment {
    pub fn initialize() -> Result<Self> {
        unsafe {
            windows::Win32::System::WinRT::RoInitialize(
                windows::Win32::System::WinRT::RO_INIT_SINGLETHREADED,
            )?;
        }
        Ok(Self(PhantomData))
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            windows::Win32::System::WinRT::RoUninitialize();
        }
    }
}

pub fn set_app_id() -> Result<()> {
    unsafe {
        SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(APP_USER_MODEL_ID))?;
    }
    Ok(())
}

/// Does not bypass DND. Windows owns final toast suppression. This shell query
/// additionally suppresses fullscreen/presentation/quiet-time distractions.
pub fn shell_allows_interruptions() -> bool {
    let shell =
        unsafe { SHQueryUserNotificationState() }.is_ok_and(|s| s == QUNS_ACCEPTS_NOTIFICATIONS);
    let focused = FocusSessionManager::GetDefault()
        .and_then(|m| m.IsFocusActive())
        .unwrap_or(false);
    shell && !focused
}

pub fn show_toast(
    toast: &MessageToast,
    policy: NotificationPolicy,
    context: NotificationContext,
) -> Result<bool> {
    if !policy.permits(context) {
        return Ok(false);
    }
    let doc = XmlDocument::new()?;
    doc.LoadXml(&HSTRING::from(toast.xml()?))?;
    let notification = ToastNotification::CreateToastNotification(&doc)?;
    // A quiet-time toast may enter Notification Center, but must not interrupt.
    notification.SetSuppressPopup(!shell_allows_interruptions())?;
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_USER_MODEL_ID))?
        .Show(&notification)?;
    Ok(true)
}

/// Synchronous short WAV playback: run on the host's bounded blocking executor.
/// PlaySound closes its device when this returns; no idle stream/thread exists.
/// `quiet_state_confirmed` must be false unless the host knows DND is off.
/// Windows exposes no reliable universal DND query for unpackaged custom sounds.
pub fn play_sound(sound: Sound, quiet_state_confirmed: bool) -> Result<bool> {
    if !quiet_state_confirmed || !shell_allows_interruptions() {
        return Ok(false);
    }
    let bytes = sound.wav();
    // SND_MEMORY takes a byte pointer despite the historical PCWSTR parameter.
    unsafe {
        PlaySoundW(
            PCWSTR(bytes.as_ptr().cast()),
            None,
            SND_MEMORY | SND_SYNC | SND_NODEFAULT,
        )
        .ok()?;
    }
    Ok(true)
}

/// Owned icon; DestroyIcon is always paired with successful CreateIcon.
pub struct Icon(HICON);
impl Icon {
    pub fn handle(&self) -> HICON {
        self.0
    }
    /// Load the host's branded .ico; no window or shell icon is displayed.
    pub fn load_ico(path: &Path) -> Result<Self> {
        if !path.is_absolute() {
            return Err(Error::Invalid("icon file must be absolute"));
        }
        let handle = unsafe {
            LoadImageW(
                None,
                &HSTRING::from(path.as_os_str()),
                IMAGE_ICON,
                0,
                0,
                LR_LOADFROMFILE | LR_DEFAULTSIZE,
            )?
        };
        Ok(Self(HICON(handle.0)))
    }
    pub fn brand() -> Result<Self> {
        let mut rgba = [0u8; 16 * 16 * 4];
        let mut mask = [255u8; 16 * 2];
        for y in 2..14 {
            for x in 1..15 {
                if !(3..13).contains(&y) && !(3..13).contains(&x) {
                    continue;
                }
                let pixel = if (y == 6 || y == 9) && (4..12).contains(&x) {
                    [0x0f, 0x17, 0x08, 255]
                } else {
                    [0x97, 0xdc, 0x3d, 255]
                };
                rgba[(y * 16 + x) * 4..(y * 16 + x + 1) * 4].copy_from_slice(&pixel);
                mask[y * 2 + x / 8] &= !(0x80 >> (x % 8));
            }
        }
        Ok(Self(unsafe {
            CreateIcon(None, 16, 16, 1, 32, mask.as_ptr(), rgba.as_ptr())?
        }))
    }
    pub fn unread(count: u32) -> Result<Self> {
        let mut rgba = [0u8; 16 * 16 * 4];
        let mut mask = [255u8; 16 * 2];
        for y in 0..16 {
            for x in 0..16 {
                if (x as i32 - 7).pow(2) + (y as i32 - 7).pow(2) <= 57 {
                    let i = (y * 16 + x) * 4;
                    rgba[i..i + 4].copy_from_slice(&[0x5c, 0x5f, 0xf2, 255]);
                    mask[y * 2 + x / 8] &= !(0x80 >> (x % 8));
                }
            }
        }
        let text = if count > 99 {
            "99".to_string()
        } else {
            count.to_string()
        };
        const DIGITS: [[u8; 5]; 10] = [
            [7, 5, 5, 5, 7],
            [2, 6, 2, 2, 7],
            [7, 1, 7, 4, 7],
            [7, 1, 7, 1, 7],
            [5, 5, 7, 1, 1],
            [7, 4, 7, 1, 7],
            [7, 4, 7, 5, 7],
            [7, 1, 1, 1, 1],
            [7, 5, 7, 5, 7],
            [7, 5, 7, 1, 7],
        ];
        let start = if text.len() == 1 { 6 } else { 4 };
        for (n, d) in text.bytes().enumerate() {
            for (y, row) in DIGITS[(d - b'0') as usize].iter().enumerate() {
                for x in 0..3 {
                    if row & (4 >> x) != 0 {
                        let i = ((y + 5) * 16 + start + n * 4 + x) * 4;
                        rgba[i..i + 4].copy_from_slice(&[0xed, 0xea, 0xe8, 255]);
                    }
                }
            }
        }
        Ok(Self(unsafe {
            CreateIcon(None, 16, 16, 1, 32, mask.as_ptr(), rgba.as_ptr())?
        }))
    }
}
impl Drop for Icon {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyIcon(self.0);
        }
    }
}

pub fn set_unread_badge(hwnd: HWND, count: u32) -> Result<()> {
    let taskbar: ITaskbarList3 =
        unsafe { CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER)? };
    unsafe {
        taskbar.HrInit()?;
    }
    let icon = if count == 0 {
        None
    } else {
        Some(Icon::unread(count)?)
    };
    let description = HSTRING::from(format!("{count} unread messages"));
    unsafe {
        taskbar.SetOverlayIcon(
            hwnd,
            icon.as_ref().map_or(HICON::default(), |i| i.0),
            &description,
        )?;
    }
    Ok(())
}
pub fn flash_mention(hwnd: HWND, do_not_disturb: bool) {
    if do_not_disturb || !shell_allows_interruptions() {
        return;
    }
    let info = FLASHWINFO {
        cbSize: size_of::<FLASHWINFO>() as u32,
        hwnd,
        dwFlags: FLASHW_TRAY,
        uCount: 3,
        dwTimeout: 0,
    };
    unsafe {
        let _ = FlashWindowEx(&info);
    }
}
/// Only call after an explicit user activation (tray, second launch or deep link).
pub fn focus_window(hwnd: HWND) {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            placement::show_saved(hwnd);
        }
        let _ = SetForegroundWindow(hwnd);
    }
}
pub fn show_window_inactive(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
}
pub fn hide_window(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
}

/// Host owns HWND and handles callback_message in its event loop. The icon handle
/// must remain valid while this guard lives. Recreate on Explorer TaskbarCreated.
pub struct TrayIcon {
    data: NOTIFYICONDATAW,
    _icon: Icon,
}
impl TrayIcon {
    pub fn add(hwnd: HWND, callback_message: u32, icon: Icon) -> Result<Self> {
        if callback_message < WM_APP {
            return Err(Error::Invalid("tray callback must be WM_APP or higher"));
        }
        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: callback_message,
            hIcon: icon.0,
            ..Default::default()
        };
        for (out, c) in data.szTip.iter_mut().zip("StickTalk".encode_utf16()) {
            *out = c;
        }
        unsafe {
            Shell_NotifyIconW(NIM_ADD, &data).ok()?;
        }
        Ok(Self { data, _icon: icon })
    }
    /// Invoke only on a real user context-menu event; no polling/event thread.
    pub fn menu(&self, muted: bool, deafened: bool) -> Result<Option<TrayAction>> {
        let menu = unsafe { CreatePopupMenu()? };
        let result = (|| -> Result<_> {
            for (id, label, action) in tray::MENU {
                let checked = matches!(action, TrayAction::Mute) && muted
                    || matches!(action, TrayAction::Deafen) && deafened;
                unsafe {
                    AppendMenuW(
                        menu,
                        MF_STRING | if checked { MF_CHECKED } else { MF_UNCHECKED },
                        id as usize,
                        &HSTRING::from(label),
                    )?;
                }
            }
            let mut point = POINT::default();
            unsafe {
                GetCursorPos(&mut point)?;
                let _ = SetForegroundWindow(self.data.hWnd);
                let id = TrackPopupMenu(
                    menu,
                    TPM_RETURNCMD | TPM_RIGHTBUTTON,
                    point.x,
                    point.y,
                    None,
                    self.data.hWnd,
                    None,
                )
                .0 as u16;
                let _ = PostMessageW(
                    Some(self.data.hWnd),
                    WM_NULL,
                    Default::default(),
                    Default::default(),
                );
                Ok(tray::menu_action(id))
            }
        })();
        unsafe {
            let _ = DestroyMenu(menu);
        }
        result
    }
    /// Legacy Shell_NotifyIcon callback lParam, delivered by the host's WndProc.
    /// Only an actual user click opens a menu/restores the window.
    pub fn on_callback(
        &self,
        event: u32,
        muted: bool,
        deafened: bool,
    ) -> Result<Option<TrayAction>> {
        match event {
            WM_LBUTTONDBLCLK => Ok(Some(TrayAction::Open)),
            WM_RBUTTONUP | WM_CONTEXTMENU => self.menu(muted, deafened),
            _ => Ok(None),
        }
    }
    /// Call on Explorer's registered TaskbarCreated message, not on a timer.
    pub fn restore_after_explorer_restart(&self) -> Result<()> {
        unsafe {
            Shell_NotifyIconW(NIM_ADD, &self.data).ok()?;
        }
        Ok(())
    }
}
impl Drop for TrayIcon {
    fn drop(&mut self) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &self.data);
        }
    }
}

pub fn set_start_with_windows(exe: &Path, enabled: bool) -> Result<()> {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run")?;
    if enabled {
        if !exe.is_absolute() || exe.to_string_lossy().contains('"') {
            return Err(Error::Invalid("startup executable path"));
        }
        key.set_value("StickTalk", &format!("\"{}\" --background", exe.display()))?;
    } else if let Err(e) = key.delete_value("StickTalk") {
        if e.kind() != std::io::ErrorKind::NotFound {
            return Err(e.into());
        }
    }
    Ok(())
}

/// NSIS calls the maintenance helper on the shortcut it has just created.
pub fn register_shortcut_aumid(path: &Path) -> Result<()> {
    use windows::Win32::{
        Foundation::PROPERTYKEY,
        System::{
            Com::StructuredStorage::{
                PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
            },
            Variant::VT_LPWSTR,
        },
        UI::Shell::PropertiesSystem::{
            IPropertyStore, SHGetPropertyStoreFromParsingName, GPS_READWRITE,
        },
    };
    let path = HSTRING::from(path.as_os_str());
    // PROPVARIANT owns/frees VT_LPWSTR with CoTaskMemFree. Never put a Rust Vec
    // allocation here: windows-rs implements Drop even though it is a C struct.
    let value = unsafe { SHStrDupW(&HSTRING::from(APP_USER_MODEL_ID))? };
    let prop = PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: std::mem::ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_LPWSTR,
                Anonymous: PROPVARIANT_0_0_0 { pwszVal: value },
                ..Default::default()
            }),
        },
    };
    let key = PROPERTYKEY {
        fmtid: windows::core::GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
        pid: 5,
    };
    unsafe {
        let store: IPropertyStore = SHGetPropertyStoreFromParsingName(&path, None, GPS_READWRITE)?;
        // SetValue copies the string; PROPVARIANT releases the original on drop.
        store.SetValue(&key, &prop)?;
        store.Commit()?;
    }
    Ok(())
}

/// Wait on a process handle (zero CPU); no polling. Used by the restart helper.
pub fn wait_for_exit(pid: u32) -> Result<()> {
    use windows::Win32::{
        Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, WAIT_OBJECT_0},
        System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE},
    };
    unsafe {
        let handle = match OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            Ok(h) => h,
            Err(e) if e.code() == ERROR_INVALID_PARAMETER.to_hresult() => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        let result = WaitForSingleObject(handle, 120_000);
        let _ = CloseHandle(handle);
        if result != WAIT_OBJECT_0 {
            return Err(Error::Invalid("parent did not exit within two minutes"));
        }
    }
    Ok(())
}
pub mod placement;
pub mod update_install;
