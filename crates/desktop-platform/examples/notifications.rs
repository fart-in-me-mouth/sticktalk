use sticktalk_desktop_platform::notification::*;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let context = NotificationContext {
        direct_message: true,
        ..Default::default()
    };
    let toast = MessageToast {
        sender: "Ada".into(),
        server_channel: "Friends / #general".into(),
        preview: "Ready to join?".into(),
        avatar_uri: Some("file:///C:/cache/ada.png".into()),
        channel_id: 1,
        message_id: 2,
    };
    println!(
        "Dry run: permitted={}\n{}",
        NotificationPolicy::default().permits(context),
        toast.xml()?
    );
    // Real integration, on COM-initialized UI thread only:
    // windows::show_toast(&toast, policy, context)?;
    // windows::set_unread_badge(hwnd, 12)?; windows::flash_mention(hwnd, false);
    Ok(())
}
