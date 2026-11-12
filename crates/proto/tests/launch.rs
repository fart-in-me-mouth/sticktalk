use sticktalk_proto::*;

fn role() -> Role {
    Role {
        id: Id(20),
        guild_id: Id(10),
        name: "Mod".into(),
        color: 0x123456,
        position: 2,
        hoist: true,
        mentionable: true,
        permissions: Permissions::MANAGE_ROLES,
    }
}
fn settings() -> NotificationSettings {
    NotificationSettings {
        guild_id: Some(Id(10)),
        channel_id: None,
        level: NotificationLevel::Mentions,
        muted: true,
        mute_until: Some(123),
        suppress_everyone: true,
        hide_muted_channels: true,
    }
}
#[test]
fn all_launch_commands_round_trip_named_json_and_messagepack() {
    for command in [
        LaunchCommand::CreateRole {
            guild_id: Id(10),
            name: "Mod".into(),
        },
        LaunchCommand::UpdateRole { role: role() },
        LaunchCommand::DeleteRole {
            guild_id: Id(10),
            role_id: Id(20),
        },
        LaunchCommand::ReorderRoles {
            guild_id: Id(10),
            role_ids: vec![Id(10), Id(20)],
        },
        LaunchCommand::SetMemberRoles {
            guild_id: Id(10),
            user_id: Id(2),
            role_ids: vec![Id(20)],
        },
        LaunchCommand::SetOverwrites {
            channel_id: Id(11),
            overwrites: vec![PermissionOverwrite {
                kind: OverwriteKind::Member,
                target_id: Id(2),
                allow: Permissions::VIEW_CHANNEL,
                deny: Permissions::SEND_MESSAGES,
            }],
        },
        LaunchCommand::UpdateGuildProfile {
            guild_id: Id(10),
            description: "Server".into(),
            banner_color: 9,
        },
        LaunchCommand::UpdateUserProfile {
            banner_color: 0xffffff,
            pronouns: "they/them".into(),
            about_me: "Hi".into(),
        },
        LaunchCommand::SetNickname {
            guild_id: Id(10),
            user_id: Id(2),
            nickname: None,
        },
        LaunchCommand::PinMessage {
            channel_id: Id(11),
            message_id: Id(12),
            pinned: true,
        },
        LaunchCommand::SetNotifications(settings()),
        LaunchCommand::UpdateDevice {
            device_name: "Desktop".into(),
            platform: "Windows".into(),
        },
        LaunchCommand::LogoutSession {
            session_id: "opaque".into(),
        },
        LaunchCommand::LogoutAllOthers,
    ] {
        let value = ClientMsg::Command(CommandRequest {
            nonce: 1,
            command: Command::Launch(command),
        });
        let bytes = rmp_serde::to_vec_named(&value).unwrap();
        assert_eq!(rmp_serde::from_slice::<ClientMsg>(&bytes).unwrap(), value);
        let json = serde_json::to_value(&value).unwrap();
        assert!(json["data"]["command"]["data"].is_object());
        assert_eq!(serde_json::from_value::<ClientMsg>(json).unwrap(), value);
    }
}
#[test]
fn legacy_members_decode_with_additive_defaults_and_old_bits_unchanged() {
    let old = serde_json::json!({"guild_id":10,"user":{"id":2,"username":"old","display_name":null,"avatar_id":null,"status_text":null},"role":"member","permissions":287,"joined_at":0});
    let member: Member = serde_json::from_value(old).unwrap();
    assert!(member.role_ids.is_empty());
    assert!(member.nickname.is_none());
    assert_eq!(member.permissions, Permissions::MEMBER);
    assert_eq!(Permissions::GUILD_OWNER.bits(), 16383);
    assert_eq!(Permissions::MANAGE_ROLES.bits(), 1 << 14);
}
#[test]
fn launch_events_and_results_round_trip() {
    for event in [
        Event::RolesUpdated {
            guild_id: Id(10),
            roles: vec![role()],
        },
        Event::PinsUpdated {
            channel_id: Id(11),
            message_id: Id(12),
            pinned: true,
        },
        Event::NotificationSettingsUpdated(settings()),
        Event::SessionsUpdated,
    ] {
        let value = ServerMsg::Event(Box::new(EventEnvelope { seq: 1, event }));
        let bytes = rmp_serde::to_vec_named(&value).unwrap();
        assert_eq!(rmp_serde::from_slice::<ServerMsg>(&bytes).unwrap(), value);
    }
    for value in [
        CommandResult::Role(role()),
        CommandResult::NotificationSettings(settings()),
    ] {
        let bytes = rmp_serde::to_vec_named(&value).unwrap();
        assert_eq!(
            rmp_serde::from_slice::<CommandResult>(&bytes).unwrap(),
            value
        );
    }
}

#[test]
fn legacy_identify_has_no_new_required_wire_fields() {
    let value = serde_json::json!({"version":1,"token":"opaque","resume":null});
    let identify: Identify = serde_json::from_value(value.clone()).unwrap();
    assert!(!identify.launch_features);
    assert_eq!(serde_json::to_value(identify).unwrap(), value);
}
