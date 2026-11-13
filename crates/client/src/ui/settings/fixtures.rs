//! Deterministic visual fixtures. Only invoked by the offscreen screenshot harness.
use super::*;
pub(crate) const SCREENS: &[&str] = &[
    "user-settings-account",
    "user-settings-password",
    "user-settings-sign-in",
    "sign-in-methods-unlinked",
    "sign-in-methods-google-only",
    "password-settings",
    "user-settings-account-status",
    "user-settings-account-edit",
    "user-settings-account-dirty",
    "user-settings-account-revealed",
    "user-settings-profile",
    "user-settings-profile-dirty",
    "user-settings-privacy",
    "user-settings-devices",
    "user-settings-notifications",
    "user-settings-voice",
    "user-settings-voice-processing",
    "user-settings-voice-input",
    "user-settings-appearance",
    "user-settings-accessibility",
    "user-settings-keybinds",
    "user-settings-advanced",
    "user-settings-nav-search",
    "guild-settings-overview",
    "guild-settings-dirty",
    "guild-settings-engagement",
    "guild-settings-members",
    "guild-settings-members-filter",
    "guild-settings-roles",
    "guild-settings-roles-drag",
    "guild-settings-role-display",
    "guild-settings-role-menu",
    "guild-settings-role-permissions",
    "guild-settings-role-members",
    "guild-settings-role-add-members",
    "guild-settings-role-disabled",
    "guild-settings-default-permissions",
    "guild-settings-invites",
    "guild-settings-invites-empty",
    "guild-settings-bans",
    "guild-settings-bans-empty",
    "guild-settings-emoji",
    "guild-settings-access",
    "guild-settings-audit",
    "guild-settings-delete",
    "guild-settings-delete-confirmed",
    "channel-settings-overview",
    "channel-settings-permissions",
    "channel-settings-private",
    "channel-settings-disabled",
    "notification-settings",
    "notification-settings-muted",
];
pub(crate) fn configure(app: &mut App, screen: &str) {
    if !SCREENS.contains(&screen) {
        return;
    }
    let gid = Id(10);
    let mut roles = vec![];
    for (i, name) in [
        "@everyone",
        "Members",
        "Artists",
        "Developers",
        "Event Hosts",
        "Moderators",
        "Administrators",
        "Team",
    ]
    .into_iter()
    .enumerate()
    {
        roles.push(Role {
            id: if i == 0 { gid } else { Id(100 + i as u64) },
            guild_id: gid,
            name: name.into(),
            color: if i == 0 { 0 } else { st::COLORS[i] },
            position: i as u16,
            hoist: i > 4,
            mentionable: i < 4,
            permissions: if i == 0 {
                Permissions::VIEW_CHANNEL
                    | Permissions::SEND_MESSAGES
                    | Permissions::READ_HISTORY
                    | Permissions::ADD_REACTIONS
                    | Permissions::CREATE_INVITES
                    | Permissions::CONNECT
                    | Permissions::SPEAK
            } else if i > 5 {
                Permissions::ADMINISTRATOR
            } else {
                Permissions::empty()
            },
        });
    }
    app.state.model.settings.roles = roles.into_iter().map(|r| (r.id, r)).collect();
    for m in app.state.model.members.values_mut() {
        m.role_ids = vec![gid, Id(101)];
        if m.user.id == Id(2) || m.user.id == Id(3) {
            m.role_ids.push(Id(103));
        }
        if m.user.id == Id(1) {
            m.role_ids.push(Id(107));
        }
    }
    if let Some(m) = app.state.model.members.get(&Id(1)) {
        app.state.model.settings.memberships.insert(gid, m.clone());
    }
    let guild = app.state.model.guilds[&gid].guild.clone();
    app.state.model.settings.guild_profiles.insert(
        gid,
        GuildPreview {
            guild,
            profile: GuildProfile {
                guild_id: gid,
                description: "A place to build, share, and spend time together.".into(),
                banner_color: 2,
                created_at: 1_745_006_400_000,
            },
            online_count: 5,
        },
    );
    let me = app.state.model.me.as_ref().unwrap().user.clone();
    app.state.model.settings.profiles.insert(
        me.id,
        UserProfile {
            user: me,
            banner_color: st::COLORS[0],
            pronouns: "they/them".into(),
            about_me: "Designer, weekend hiker, and a fan of small details.".into(),
            created_at: 1_745_006_400_000,
        },
    );
    app.state.model.settings.sessions = vec![
        DeviceSession {
            id: "session-current".into(),
            device_name: "Desktop PC".into(),
            platform: "Windows 11".into(),
            last_active: crate::mock::now(),
            place: "this device".into(),
            current: true,
        },
        DeviceSession {
            id: "session-laptop".into(),
            device_name: "River’s Laptop".into(),
            platform: "Windows 11".into(),
            last_active: crate::mock::now() - 3_600_000,
            place: "this device".into(),
            current: false,
        },
        DeviceSession {
            id: "session-browser".into(),
            device_name: "Web Browser".into(),
            platform: "Firefox".into(),
            last_active: crate::mock::now() - 86_400_000,
            place: "this device".into(),
            current: false,
        },
    ];
    app.settings = crate::settings_controller::State::default();
    app.settings_sync_profile();
    app.settings_sync_server();
    app.methods = Some(AuthMethodsResponse {
        has_password: true,
        google_linked: true,
    });
    app.settings.loaded_user = app.me();
    app.settings.loaded_guild = Some(gid);
    if matches!(
        screen,
        "sign-in-methods-unlinked" | "sign-in-methods-google-only" | "password-settings"
    ) {
        app.dialog = if screen == "password-settings" {
            Dialog::Password
        } else {
            Dialog::UserSettings
        };
        if screen == "password-settings" {
            app.settings_return = Some(Dialog::UserSettings);
        }
        app.user_tab = 0;
        app.methods = Some(AuthMethodsResponse {
            has_password: screen != "sign-in-methods-google-only",
            google_linked: screen != "sign-in-methods-unlinked",
        });
    } else if screen.starts_with("user-settings-") {
        app.dialog = Dialog::UserSettings;
        let page = match screen {
            "user-settings-profile" | "user-settings-profile-dirty" => UserPage::Profiles,
            "user-settings-privacy" => UserPage::Privacy,
            "user-settings-devices" => UserPage::Devices,
            "user-settings-notifications" => UserPage::Notifications,
            "user-settings-voice"
            | "user-settings-voice-processing"
            | "user-settings-voice-input" => UserPage::Voice,
            "user-settings-appearance" => UserPage::Appearance,
            "user-settings-accessibility" => UserPage::Accessibility,
            "user-settings-keybinds" => UserPage::Keybinds,
            "user-settings-advanced" => UserPage::Advanced,
            _ => UserPage::Account,
        };
        app.user_tab = page.index();
        if page == UserPage::Voice {
            app.calls.audio_devices = crate::audio_settings::fixture_devices();
            app.calls.call.settings.auto_high_quality = Some(true);
            if screen == "user-settings-voice-input" {
                app.calls.call.settings.push_to_talk = true;
                app.calls.call.settings.push_to_talk_key = Some(0x56);
            }
        }
        match screen {
            "user-settings-profile-dirty" => {
                app.settings.profile.about = "Building something new. Come say hello!".into();
                app.settings.profile.color = st::COLORS[2];
            }
            "user-settings-account-dirty" => {
                app.settings.edit_account = true;
                app.settings.edit_username = true;
                app.form.code = "river_notes".into();
            }
            "user-settings-account-edit" => {
                app.settings.edit_account = true;
                app.form.code = "river".into();
            }
            "user-settings-account-revealed" => app.settings.reveal = true,
            "user-settings-nav-search" => app.settings.nav_search = "pro".into(),
            _ => {}
        }
    } else if screen.starts_with("guild-settings-") {
        let tab = match screen {
            "guild-settings-engagement" => GuildTab::Engagement,
            "guild-settings-members" | "guild-settings-members-filter" => GuildTab::Members,
            "guild-settings-invites" | "guild-settings-invites-empty" => GuildTab::Invites,
            "guild-settings-bans" | "guild-settings-bans-empty" => GuildTab::Bans,
            "guild-settings-emoji" => GuildTab::Emoji,
            "guild-settings-access" => GuildTab::Access,
            "guild-settings-audit" => GuildTab::AuditLog,
            s if s.contains("role") || s == "guild-settings-default-permissions" => GuildTab::Roles,
            _ => GuildTab::Overview,
        };
        app.dialog = Dialog::GuildSettings(tab);
        if screen == "guild-settings-invites" {
            app.state.model.invites.insert(
                Id(5001),
                Invite {
                    id: Id(5001),
                    code: "STICK-7mQx-29ab".into(),
                    guild_id: gid,
                    creator_id: Id(1),
                    expires_at: crate::mock::now() + 86_400_000,
                    max_uses: 25,
                    uses: 3,
                },
            );
        }
        if screen == "guild-settings-bans" {
            let user = crate::mock::user(9, "taylor", "Taylor Brooks");
            app.state.model.users.insert(user.id, user.clone());
            app.state.model.bans.insert(
                user.id,
                Ban {
                    guild_id: gid,
                    user,
                    actor_id: Id(1),
                    reason: Some("Repeated harassment.".into()),
                    created_at: crate::mock::now() - 86_400_000,
                },
            );
        }

        if screen.starts_with("guild-settings-role-")
            || screen == "guild-settings-default-permissions"
        {
            let id = if screen == "guild-settings-default-permissions" {
                gid
            } else {
                Id(103)
            };
            let _ = app.settings_update(A::EditRole(id));
            app.settings.role_tab = Some(match screen {
                "guild-settings-role-permissions" | "guild-settings-default-permissions" => {
                    RoleTab::Permissions
                }
                "guild-settings-role-members" | "guild-settings-role-add-members" => {
                    RoleTab::Members
                }
                _ => RoleTab::Display,
            });
            app.settings.adding_members = screen == "guild-settings-role-add-members";
        }
        if screen == "guild-settings-role-disabled" {
            app.state.model.guilds.get_mut(&gid).unwrap().guild.owner_id = Id(6);
            if let Some(m) = app.state.model.settings.memberships.get_mut(&gid) {
                m.role_ids = vec![gid, Id(101)];
            }
        }
        if screen == "guild-settings-role-menu" {
            app.settings.role_menu = Some(Id(103));
        }
        if screen == "guild-settings-dirty" {
            app.settings.server.name = "The Common Room Studio".into();
            app.settings.server.banner = 5;
        }
        if screen == "guild-settings-roles-drag" {
            app.settings.dragging = Some(Id(103));
            app.settings.drag_over = Some(Id(105));
        }
        if screen == "guild-settings-members-filter" {
            app.settings.member_filter = Some(Id(103));
        }
        if screen.starts_with("guild-settings-delete") {
            app.settings.delete_open = true;
            if screen.ends_with("confirmed") {
                app.settings.delete_name = app.settings.server.name.clone();
            }
        }
    } else if screen.starts_with("channel-settings-") {
        app.dialog = Dialog::ChannelSettings(Id(12));
        app.settings.channel = Some(Id(12));
        app.settings.channel_name = "general".into();
        app.settings.channel_topic = "The place for everyday conversation.".into();
        app.state
            .model
            .topics
            .insert(Id(12), Some(app.settings.channel_topic.clone()));
        app.settings.channel_permissions = screen != "channel-settings-overview";
        app.settings.overwrites = vec![
            PermissionOverwrite {
                kind: OverwriteKind::Role,
                target_id: gid,
                allow: Permissions::empty(),
                deny: Permissions::VIEW_CHANNEL,
            },
            PermissionOverwrite {
                kind: OverwriteKind::Role,
                target_id: Id(103),
                allow: Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES,
                deny: Permissions::MANAGE_CHANNELS,
            },
        ];
        app.settings.overwrites_base = app.settings.overwrites.clone();
        app.settings.overwrite_target = Some((OverwriteKind::Role, Id(103)));
        if screen == "channel-settings-private" {
            app.settings.overwrites_base.clear();
        }
        if screen == "channel-settings-disabled" {
            app.state.model.guilds.get_mut(&gid).unwrap().guild.owner_id = Id(6);
            if let Some(m) = app.state.model.settings.memberships.get_mut(&gid) {
                m.role_ids = vec![gid];
            }
        }
    } else {
        app.dialog = Dialog::Notifications(gid);
        app.settings.notification = Some(app.state.model.settings.notification(gid));
        app.settings.mute_duration_ms = 3_600_000;
        if screen == "notification-settings-muted" {
            let s = app.settings.notification.as_mut().unwrap();
            s.muted = true;
            s.suppress_everyone = true;
            s.level = NotificationLevel::Mentions;
        }
    }
    app.settings.about_editor =
        widget::text_editor::Content::with_text(&app.settings.profile.about);
    app.settings.description_editor =
        widget::text_editor::Content::with_text(&app.settings.server.description);
}
