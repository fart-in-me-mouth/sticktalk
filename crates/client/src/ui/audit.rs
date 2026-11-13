//! Adversarial desktop fixtures. Only called by the mock/headless harness.
use super::*;
use crate::{features::Picker, main_window::Popover};

pub(crate) const SCREENS: &[&str] = &[
    "audit-long-names",
    "audit-long-message",
    "audit-many-servers",
    "audit-many-members",
    "audit-many-reactions",
    "audit-quick-empty",
    "audit-quick-long",
    "audit-search-empty",
    "audit-profile-long",
    "audit-login-error",
    "audit-register-error",
    "audit-login-busy",
    "audit-connecting",
    "audit-empty-dms",
    "audit-emoji-empty",
    "audit-gif-empty",
    "audit-gif-error",
    "audit-image-error",
    "audit-invite-copied",
    "audit-new-messages",
    "audit-server-name-long",
    "audit-user-name-long",
];

pub(crate) fn configure(app: &mut App, screen: &str) {
    use crate::main_window::{mm, Message as Main};
    let long = "WWWW Wide names with a very long descriptive suffix";
    match screen {
        "audit-long-names" | "audit-quick-long" | "audit-profile-long" => {
            for user in app.state.model.users.values_mut() {
                user.display_name = Some(long.into());
                user.status_text =
                    Some("Reviewing the release notes and the decisions from yesterday".into());
            }
            for member in app.state.model.members.values_mut() {
                member.user.display_name = Some(long.into());
            }
            for guild in app.state.model.guilds.values_mut() {
                guild.guild.name = long.into();
            }
            for channel in app.state.model.channels.values_mut() {
                if channel.guild_id.is_some() {
                    channel.name = Some("wwww-wide-channel-name-that-needs-to-fit".into());
                }
            }
            if screen == "audit-quick-long" {
                app.main.popover = Some(Popover::Quick);
            } else if screen == "audit-profile-long" {
                app.main.popover = Some(Popover::Profile(Id(2)));
                app.main.position = iced::Point::new(app.width - t::S8, app.height - t::S8);
            }
        }
        "audit-long-message" => {
            let message = app
                .state
                .model
                .messages
                .get_mut(&Id(12))
                .unwrap()
                .last_entry()
                .unwrap()
                .into_mut();
            message.content = format!("A long unbroken link must not push the message pane wider: https://example.com/{}\n{}", "release".repeat(30), "The next paragraph should wrap naturally without hiding the composer. ".repeat(20));
        }
        "audit-many-servers" => {
            let guild = app.state.model.guilds[&Id(10)].clone();
            for i in 0..128 {
                let mut guild = guild.clone();
                guild.guild.id = Id(50_000 + i);
                guild.guild.name = format!("Server {}", i + 1);
                app.state.model.guilds.insert(guild.guild.id, guild);
            }
        }
        "audit-many-members" => {
            let member = app.state.model.members[&Id(2)].clone();
            for i in 0..1000 {
                let mut member = member.clone();
                member.user.id = Id(60_000 + i);
                member.user.display_name = Some(format!("Member {i:04}"));
                app.state
                    .model
                    .users
                    .insert(member.user.id, member.user.clone());
                app.state
                    .model
                    .presences
                    .insert(member.user.id, PresenceStatus::Online);
                app.state.model.members.insert(member.user.id, member);
            }
        }
        "audit-many-reactions" => {
            let reaction = app
                .state
                .model
                .messages
                .get(&Id(12))
                .unwrap()
                .values()
                .find_map(|m| m.reactions.first())
                .cloned();
            if let Some(reaction) = reaction {
                let message = app
                    .state
                    .model
                    .messages
                    .get_mut(&Id(12))
                    .unwrap()
                    .last_entry()
                    .unwrap()
                    .into_mut();
                message.reactions = [
                    "😀", "🎉", "👍", "🔥", "❤️", "👀", "🌱", "✨", "👋", "🐱", "🎨", "☕", "💯",
                    "🚀", "🙌", "💚", "🤔", "✅", "🎮", "🌟",
                ]
                .into_iter()
                .map(|emoji| {
                    let mut r = reaction.clone();
                    r.emoji = emoji.into();
                    r.count = 999;
                    r
                })
                .collect();
            }
        }
        "audit-quick-empty" => {
            app.main.quick = "no-such-channel-zzzzz".into();
            app.main.popover = Some(Popover::Quick);
        }
        "audit-search-empty" => app.main.search = "no-such-message-zzzzz".into(),
        "audit-login-error" | "audit-register-error" | "audit-login-busy" => {
            app.state.session = None;
            app.register = screen == "audit-register-error";
            app.busy = screen == "audit-login-busy";
            if !app.busy {
                app.error = "Could not sign in. Check your username and password.".into();
            }
        }
        "audit-connecting" => app.state.model.me = None,
        "audit-empty-dms" => {
            app.state.guild = None;
            app.state.screen = Screen::Friends(FriendTab::All);
            app.state.model.channels.retain(|_, c| c.guild_id.is_some());
            app.state.model.relationships.clear();
        }
        "audit-emoji-empty" => {
            app.features.picker = Picker::Emoji;
            app.features.emoji.query = "no-such-emoji-zzzzz".into();
        }
        "audit-gif-empty" | "audit-gif-error" => {
            app.features.picker = Picker::Gif;
            app.features.gifs.query = "no-such-gif-zzzzz".into();
            if screen == "audit-gif-error" {
                app.features.gifs.error = "Could not load GIFs. Try again.".into();
            }
        }
        "audit-image-error" => {
            crate::features::fixture(app, "image-viewer");
            app.features.viewer.loading = false;
            app.features.viewer.error = "This image is no longer available.".into();
        }
        "audit-invite-copied" => {
            let _ = app.update(Msg::Open(Dialog::Invite));
            app.form.code = "STICK-7mQx-29ab".into();
            app.notice = "Invite copied.".into();
        }
        "audit-new-messages" => {
            if let Some(read) = app.state.model.reads.get_mut(&Id(12)) {
                read.last_read_id = Some(Id(2275));
                read.unread_count = 4;
            }
            app.state.at_present = false;
            app.scroll_y = 0.;
        }
        "audit-server-name-long" => {
            crate::ui::settings::configure(app, "guild-settings-overview");
            app.state.model.guilds.get_mut(&Id(10)).unwrap().guild.name = long.into();
            app.settings.server.name = long.into();
        }
        "audit-user-name-long" => {
            crate::ui::settings::configure(app, "user-settings-account");
            if let Some(me) = app.state.model.me.as_mut() {
                me.user.display_name = Some(long.into());
            }
        }
        _ => {}
    }
    // Exercise the same reducer used by keyboard navigation without executing tasks.
    if screen == "audit-quick-long" {
        let _ = app.update(mm(Main::Quick(String::new())));
    }
}
