//! StickTalk for Windows: the server's own web app in a native window.
//!
//! The window navigates to the configured server origin and nothing else; see
//! `policy` for what may load, and `shell_page` for the one built-in page.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
// Other platforms build only the portable logic and its tests.
#![cfg_attr(not(windows), allow(dead_code))]

mod address;
#[cfg(windows)]
mod app;
mod appearance;
mod control;
mod health;
mod links;
mod menu;
mod policy;
mod ptt;
mod shell_page;
#[cfg(windows)]
mod webview;
mod window_state;

fn main() {
    let launch = links::launch(std::env::args());
    #[cfg(windows)]
    std::process::exit(app::run(launch));
    #[cfg(not(windows))]
    {
        let _ = launch;
        eprintln!("The StickTalk desktop app is for Windows. Open your server's address in a browser instead.");
        std::process::exit(1);
    }
}
