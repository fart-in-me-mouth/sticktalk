#![cfg(windows)]
//! Exercise COM data/objects only. Never Show(), PlaySound, Shell_NotifyIcon,
//! SetOverlayIcon, startup registration, focus, popup menus, or visible windows.
use sticktalk_desktop_platform::{
    notification::MessageToast,
    windows::{register_shortcut_aumid, Apartment, Icon},
    APP_USER_MODEL_ID,
};
use windows::{
    core::{Interface, HSTRING},
    Data::Xml::Dom::XmlDocument,
    Win32::{
        Foundation::PROPERTYKEY,
        System::Com::{CoCreateInstance, IPersistFile, CLSCTX_INPROC_SERVER},
        UI::Shell::{
            IShellLinkW,
            PropertiesSystem::{IPropertyStore, SHGetPropertyStoreFromParsingName, GPS_DEFAULT},
            ShellLink,
        },
    },
};

#[test]
fn toast_xml_is_accepted_by_windows_without_showing_it() {
    let _apartment = Apartment::initialize().unwrap();
    let t = MessageToast {
        sender: "A < B & C".into(),
        server_channel: "Friends / #general".into(),
        preview: "Test notification".into(),
        avatar_uri: Some("file:///C:/cache/avatar.png".into()),
        channel_id: 1,
        message_id: 2,
    };
    let doc = XmlDocument::new().unwrap();
    doc.LoadXml(&HSTRING::from(t.xml().unwrap())).unwrap();
    for count in [1, 10, 99, 1000] {
        let icon = Icon::unread(count).unwrap();
        assert!(!icon.handle().is_invalid());
    }
}
#[test]
fn installer_shortcut_aumid_roundtrips_without_shell_ui() {
    let _apartment = Apartment::initialize().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("StickTalk.lnk");
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).unwrap();
        link.SetPath(&HSTRING::from(std::env::current_exe().unwrap().as_os_str()))
            .unwrap();
        let persist: IPersistFile = link.cast().unwrap();
        persist
            .Save(&HSTRING::from(path.as_os_str()), true)
            .unwrap();
    }
    register_shortcut_aumid(&path).unwrap();
    unsafe {
        let store: IPropertyStore =
            SHGetPropertyStoreFromParsingName(&HSTRING::from(path.as_os_str()), None, GPS_DEFAULT)
                .unwrap();
        let key = PROPERTYKEY {
            fmtid: windows::core::GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
            pid: 5,
        };
        assert_eq!(store.GetValue(&key).unwrap().to_string(), APP_USER_MODEL_ID);
    }
}

#[test]
fn badge_pixels_are_upright_and_use_spec_colors_in_memory_only() {
    use windows::Win32::{
        Graphics::Gdi::*,
        UI::WindowsAndMessaging::{DrawIconEx, DI_NORMAL},
    };
    let icon = Icon::unread(7).unwrap();
    let pixels = unsafe {
        let dc = CreateCompatibleDC(None);
        assert!(!dc.is_invalid());
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: 16,
                biHeight: -16,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let bitmap = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0).unwrap();
        let previous = SelectObject(dc, bitmap.into());
        std::ptr::write_bytes(bits.cast::<u8>(), 0, 16 * 16 * 4);
        let draw = DrawIconEx(dc, 0, 0, icon.handle(), 16, 16, 0, None, DI_NORMAL);
        GdiFlush().ok().unwrap();
        let pixels = std::slice::from_raw_parts(bits.cast::<u8>(), 16 * 16 * 4).to_vec();
        SelectObject(dc, previous);
        DeleteObject(bitmap.into()).ok().unwrap();
        DeleteDC(dc).ok().unwrap();
        draw.unwrap();
        pixels
    };
    let pixel = |x: usize, y: usize| &pixels[(y * 16 + x) * 4..(y * 16 + x) * 4 + 3];
    assert_eq!(
        pixel(6, 5),
        [0xed, 0xea, 0xe8],
        "top stroke must not be inverted"
    );
    assert_eq!(
        pixel(6, 9),
        [0x5c, 0x5f, 0xf2],
        "danger background below top stroke"
    );
}
