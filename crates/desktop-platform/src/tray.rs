use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct DesktopPreferences {
    /// Default closing behavior is minimize to tray; explicit Quit always exits.
    pub minimize_to_tray: bool,
    /// Never register HKCU Run until the user opts in.
    pub start_with_windows: bool,
}
impl DesktopPreferences {
    pub fn initial() -> Self {
        Self {
            minimize_to_tray: true,
            start_with_windows: false,
        }
    }
    pub fn close_action(self, tray_available: bool) -> CloseAction {
        if self.minimize_to_tray && tray_available {
            CloseAction::Hide
        } else {
            CloseAction::Exit
        }
    }
}
impl Default for DesktopPreferences {
    fn default() -> Self {
        Self::initial()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseAction {
    Hide,
    Exit,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayAction {
    Open,
    Hide,
    Mute,
    Deafen,
    Quit,
}
pub const MENU: [(u16, &str, TrayAction); 5] = [
    (1, "Show StickTalk", TrayAction::Open),
    (5, "Hide StickTalk", TrayAction::Hide),
    (2, "Mute", TrayAction::Mute),
    (3, "Deafen", TrayAction::Deafen),
    (4, "Quit", TrayAction::Quit),
];
pub fn menu_action(id: u16) -> Option<TrayAction> {
    MENU.iter().find(|m| m.0 == id).map(|m| m.2)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn closing_never_hides_without_a_tray() {
        let p = DesktopPreferences::initial();
        assert!(!p.start_with_windows);
        assert_eq!(p.close_action(true), CloseAction::Hide);
        assert_eq!(p.close_action(false), CloseAction::Exit);
        assert_eq!(menu_action(4), Some(TrayAction::Quit));
        assert_eq!(menu_action(5), Some(TrayAction::Hide));
        assert_eq!(menu_action(0), None);
    }
}
