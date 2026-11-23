use serde::{de::DeserializeOwned, Serialize};
use std::fmt::Debug;
use sticktalk_proto::*;

fn round_trip<T: Serialize + DeserializeOwned + PartialEq + Debug>(value: T) {
    let bytes = rmp_serde::to_vec_named(&value).unwrap();
    let decoded: T = rmp_serde::from_slice(&bytes).unwrap();
    assert_eq!(value, decoded);
    let json = serde_json::to_vec(&value).unwrap();
    assert_eq!(value, serde_json::from_slice::<T>(&json).unwrap());
}

fn user() -> UserSummary {
    UserSummary {
        id: Id(1),
        username: "alice".into(),
        display_name: Some("Alice 🦀".into()),
        avatar_id: None,
        status_text: Some("low memory, high spirits".into()),
    }
}

fn me() -> SelfUser {
    SelfUser {
        user: user(),
        desired_presence: PresenceStatus::Online,
    }
}

fn cursor() -> ResumeCursor {
    ResumeCursor {
        stream_id: "0123456789abcdef0123456789abcdef".into(),
        seq: 42,
    }
}

fn channel() -> Channel {
    Channel {
        summary: ChannelSummary {
            id: Id(3),
            kind: ChannelKind::Text,
            guild_id: Some(Id(2)),
            parent_id: None,
            name: Some("general".into()),
            position: 0,
            recipient_ids: vec![],
            owner_id: None,
        },
        topic: Some("Welcome".into()),
    }
}

fn guild() -> GuildDetail {
    GuildDetail {
        summary: GuildSummary {
            guild: Guild {
                id: Id(2),
                owner_id: Id(1),
                name: "Rust".into(),
                icon_id: None,
                member_count: 1,
            },
            my_role: MemberRole::Owner,
            my_permissions: Permissions::GUILD_OWNER,
        },
        channels: vec![channel().summary],
    }
}

fn member() -> Member {
    Member {
        guild_id: Id(2),
        user: user(),
        role: MemberRole::Owner,
        permissions: Permissions::GUILD_OWNER,
        joined_at: SNOWFLAKE_EPOCH_MS,
        role_ids: vec![],
        nickname: None,
    }
}

fn attachment() -> Attachment {
    Attachment {
        id: Id(6),
        filename: "crab.png".into(),
        content_type: "image/png".into(),
        byte_size: 1234,
        width: Some(512),
        height: Some(512),
        thumbnail: Some(Thumbnail {
            width: 256,
            height: 256,
            byte_size: 512,
            content_type: "image/webp".into(),
        }),
        meta: Default::default(),
    }
}

fn message() -> Message {
    Message {
        id: Id(5),
        channel_id: Id(3),
        author_id: Id(1),
        content: "Hi @bob 🦀".into(),
        created_at: SNOWFLAKE_EPOCH_MS + 1,
        edited_at: None,
        reply_to: Some(Id(4)),
        attachments: vec![attachment()],
        mention_ids: vec![Id(7)],
        reactions: vec![ReactionSummary {
            emoji: "👍🏽".into(),
            count: 2,
            me: true,
        }],
        extra: Default::default(),
    }
}

fn relationship() -> Relationship {
    Relationship {
        user: user(),
        kind: RelationshipKind::Friend,
        since: SNOWFLAKE_EPOCH_MS,
    }
}

fn read_state() -> ReadState {
    ReadState {
        channel_id: Id(3),
        last_read_id: Some(Id(4)),
        latest_message_id: Some(Id(5)),
        unread_count: 1,
        mention_count: 1,
    }
}

fn reaction() -> ReactionUpdate {
    ReactionUpdate {
        channel_id: Id(3),
        message_id: Id(5),
        user_id: Id(1),
        added: true,
        reaction: ReactionSummary {
            emoji: "👩‍💻".into(),
            count: 1,
            me: true,
        },
    }
}

fn invite() -> Invite {
    Invite {
        id: Id(9),
        code: "opaque-invite".into(),
        guild_id: Id(2),
        creator_id: Id(1),
        expires_at: SNOWFLAKE_EPOCH_MS + 60_000,
        max_uses: 10,
        uses: 0,
    }
}

#[test]
fn every_command_round_trips() {
    let commands = vec![
        Command::UpdateProfile(ProfilePatch {
            username: Patch::Keep,
            display_name: Patch::Set("Alice".into()),
            avatar_id: Patch::Clear,
            status_text: Patch::Clear,
        }),
        Command::SetPresence {
            status: PresenceStatus::Invisible,
        },
        Command::SendFriendRequest {
            username: "bob".into(),
        },
        Command::AcceptFriendRequest { user_id: Id(7) },
        Command::DeclineFriendRequest { user_id: Id(7) },
        Command::CancelFriendRequest { user_id: Id(7) },
        Command::RemoveFriend { user_id: Id(7) },
        Command::BlockUser { user_id: Id(7) },
        Command::UnblockUser { user_id: Id(7) },
        Command::OpenDm { user_id: Id(7) },
        Command::CreateGroupDm {
            name: "Friends".into(),
            user_ids: vec![Id(7), Id(8)],
        },
        Command::AddGroupRecipient {
            channel_id: Id(3),
            user_id: Id(7),
        },
        Command::RemoveGroupRecipient {
            channel_id: Id(3),
            user_id: Id(7),
        },
        Command::LeaveGroupDm { channel_id: Id(3) },
        Command::TransferGroupOwnership {
            channel_id: Id(3),
            user_id: Id(7),
        },
        Command::CloseDm { channel_id: Id(3) },
        Command::CreateGuild {
            name: "Rust".into(),
            icon_id: Some(Id(6)),
        },
        Command::UpdateGuild {
            guild_id: Id(2),
            patch: GuildPatch {
                name: Patch::Set("Crabs".into()),
                icon_id: Patch::Clear,
            },
        },
        Command::DeleteGuild { guild_id: Id(2) },
        Command::JoinGuild {
            invite_code: "opaque-invite".into(),
        },
        Command::LeaveGuild { guild_id: Id(2) },
        Command::TransferGuildOwnership {
            guild_id: Id(2),
            user_id: Id(7),
        },
        Command::SetMemberRole {
            guild_id: Id(2),
            user_id: Id(7),
            role: MemberRole::Admin,
        },
        Command::KickMember {
            guild_id: Id(2),
            user_id: Id(7),
        },
        Command::BanMember {
            guild_id: Id(2),
            user_id: Id(7),
            reason: Some("spam".into()),
        },
        Command::UnbanMember {
            guild_id: Id(2),
            user_id: Id(7),
        },
        Command::CreateInvite {
            guild_id: Id(2),
            ttl_seconds: 3600,
            max_uses: 10,
        },
        Command::RevokeInvite {
            guild_id: Id(2),
            code: "opaque-invite".into(),
        },
        Command::CreateChannel {
            guild_id: Id(2),
            kind: ChannelKind::Text,
            name: "general".into(),
            parent_id: None,
            topic: None,
        },
        Command::UpdateChannel {
            channel_id: Id(3),
            patch: ChannelPatch {
                name: Patch::Keep,
                topic: Patch::Clear,
            },
        },
        Command::ReorderChannels {
            guild_id: Id(2),
            placements: vec![ChannelPlacement {
                channel_id: Id(3),
                parent_id: None,
                position: 0,
            }],
        },
        Command::DeleteChannel { channel_id: Id(3) },
        Command::SendMessage(MessageDraft {
            channel_id: Id(3),
            content: "hi".into(),
            reply_to: None,
            attachment_ids: vec![Id(6)],
            mention_ids: vec![Id(7)],
        }),
        Command::EditMessage(MessageEdit {
            channel_id: Id(3),
            message_id: Id(5),
            content: "edited".into(),
            mention_ids: vec![],
        }),
        Command::DeleteMessage {
            channel_id: Id(3),
            message_id: Id(5),
        },
        Command::AddReaction {
            channel_id: Id(3),
            message_id: Id(5),
            emoji: "🦀".into(),
        },
        Command::RemoveReaction {
            channel_id: Id(3),
            message_id: Id(5),
            emoji: "🦀".into(),
        },
        Command::Typing { channel_id: Id(3) },
        Command::MarkRead {
            channel_id: Id(3),
            message_id: Id(5),
        },
        Command::SubscribeGuild {
            guild_id: Some(Id(2)),
            presence_user_ids: vec![Id(7)],
        },
    ];
    assert_eq!(commands.len(), 40);
    for (index, command) in commands.into_iter().enumerate() {
        round_trip(ClientMsg::Command(CommandRequest {
            nonce: index as u64 + 1,
            command,
        }));
    }
}

#[test]
fn every_event_round_trips() {
    let events = vec![
        Event::SelfUpdated(me()),
        Event::UserUpdated(user()),
        Event::RelationshipUpsert(relationship()),
        Event::RelationshipRemoved { user_id: Id(7) },
        Event::PresenceUpdated(Presence {
            user_id: Id(7),
            status: PresenceStatus::Dnd,
        }),
        Event::GuildUpsert(guild().summary),
        Event::GuildJoined {
            detail: guild(),
            read_states: vec![read_state()],
        },
        Event::GuildRemoved {
            guild_id: Id(2),
            reason: RemovalReason::Banned,
        },
        Event::MemberUpsert(member()),
        Event::MemberRemoved {
            guild_id: Id(2),
            user_id: Id(7),
            reason: RemovalReason::Kicked,
        },
        Event::ChannelUpsert(channel()),
        Event::ChannelRemoved {
            channel_id: Id(3),
            reason: RemovalReason::Deleted,
        },
        Event::ChannelsReordered {
            guild_id: Id(2),
            placements: vec![ChannelPlacement {
                channel_id: Id(3),
                parent_id: None,
                position: 0,
            }],
        },
        Event::MessageCreated {
            message: message(),
            author: user(),
        },
        Event::MessageUpdated(message()),
        Event::MessageDeleted {
            channel_id: Id(3),
            message_id: Id(5),
        },
        Event::ReactionUpdated(reaction()),
        Event::Typing {
            channel_id: Id(3),
            user_id: Id(7),
            expires_at: SNOWFLAKE_EPOCH_MS + 8_000,
        },
        Event::ReadStateUpdated(read_state()),
    ];
    let mut events = events;
    events.extend([
        Event::InviteUpsert(invite()),
        Event::InviteRemoved {
            guild_id: Id(2),
            invite_id: Id(9),
        },
        Event::BanUpsert(Ban {
            guild_id: Id(2),
            user: user(),
            actor_id: Id(7),
            reason: None,
            created_at: SNOWFLAKE_EPOCH_MS,
        }),
        Event::BanRemoved {
            guild_id: Id(2),
            user_id: Id(7),
        },
    ]);
    assert_eq!(events.len(), 23);
    for (index, event) in events.into_iter().enumerate() {
        round_trip(ServerMsg::Event(Box::new(EventEnvelope {
            seq: index as u64 + 1,
            event,
        })));
    }
}

#[test]
fn handshake_reconnect_and_all_result_shapes_round_trip() {
    round_trip(ClientMsg::Identify(Identify {
        text_features: true,
        launch_features: true,
        version: PROTOCOL_VERSION,
        token: "secret".into(),
        resume: Some(cursor()),
    }));
    round_trip(ClientMsg::Heartbeat(Heartbeat {
        nonce: 12,
        last_seq: 42,
    }));
    let voice = VoicePayload {
        nonce: 13,
        version: 0,
        kind: "reserved".into(),
        payload: vec![0, 127, 255],
    };
    round_trip(ClientMsg::Voice(voice.clone()));
    round_trip(ServerMsg::Voice(voice));
    round_trip(ServerMsg::Hello(Hello {
        version: PROTOCOL_VERSION,
        heartbeat_interval_ms: HEARTBEAT_INTERVAL_MS,
        heartbeat_timeout_ms: HEARTBEAT_TIMEOUT_MS,
        max_frame_bytes: MAX_WS_FRAME_BYTES,
        server_time: SNOWFLAKE_EPOCH_MS,
    }));
    round_trip(ServerMsg::Ready(Box::new(Ready {
        roles: vec![],
        memberships: vec![],
        notification_settings: vec![],
        cursor: cursor(),
        me: me(),
        guilds: vec![guild().summary],
        channels: vec![channel().summary],
        relationships: vec![relationship()],
        users: vec![user()],
        read_states: vec![read_state()],
        voice_states: vec![],
        presences: vec![Presence {
            user_id: Id(7),
            status: PresenceStatus::Offline,
        }],
    })));
    round_trip(ServerMsg::Resumed(Resumed {
        cursor: cursor(),
        replayed: 5,
    }));
    round_trip(ServerMsg::ResyncRequired(ResyncRequired {
        reason: ResyncReason::CursorTooOld,
    }));
    round_trip(ServerMsg::HeartbeatAck(HeartbeatAck {
        nonce: 12,
        server_time: SNOWFLAKE_EPOCH_MS,
    }));
    for result in [
        CommandResult::Ack,
        CommandResult::Profile(me()),
        CommandResult::Presence(Presence {
            user_id: Id(1),
            status: PresenceStatus::Invisible,
        }),
        CommandResult::Relationship(relationship()),
        CommandResult::Channel(channel()),
        CommandResult::Guild(guild()),
        CommandResult::Member(member()),
        CommandResult::Invite(invite()),
        CommandResult::Message(message()),
        CommandResult::Reaction(reaction()),
        CommandResult::ReadState(read_state()),
        CommandResult::Subscription { presences: vec![] },
    ] {
        round_trip(ServerMsg::Response(Box::new(CommandResponse {
            nonce: 1,
            cursor: cursor(),
            replayed: false,
            outcome: CommandOutcome::Ok(result),
        })));
    }
    let error = ApiError {
        code: ErrorCode::RateLimited,
        message: "slow down".into(),
        field: None,
        retry_after_ms: Some(1000),
    };
    round_trip(ServerMsg::Error(error.clone()));
    round_trip(ServerMsg::Response(Box::new(CommandResponse {
        nonce: 2,
        cursor: cursor(),
        replayed: false,
        outcome: CommandOutcome::Err(error),
    })));
}

#[test]
fn rest_models_and_numeric_masks_round_trip() {
    round_trip(HealthResponse {
        version: "0.1.0".into(),
        protocol_version: PROTOCOL_VERSION,
    });
    round_trip(RegisterRequest {
        username: "alice".into(),
        password: "not-a-real-password".into(),
        display_name: None,
    });
    round_trip(LoginRequest {
        username: "alice".into(),
        password: "not-a-real-password".into(),
    });
    round_trip(AuthResponse {
        token: "secret".into(),
        expires_at: SNOWFLAKE_EPOCH_MS,
        user: me(),
    });
    round_trip(PageQuery {
        around: None,
        around_timestamp: None,
        before: Some(Id(u64::MAX >> 1)),
        after: None,
        limit: Some(50),
    });
    round_trip(Page {
        items: vec![message()],
        users: vec![user()],
        next_before: Some(Id(5)),
        next_after: None,
        has_more: true,
        cursor: cursor(),
    });
    round_trip(Snapshot {
        data: guild(),
        cursor: cursor(),
    });
    round_trip(UploadMetadata {
        purpose: UploadPurpose::Message,
        channel_id: Some(Id(3)),
    });
    round_trip(UploadResponse {
        attachment: attachment(),
        expires_at: SNOWFLAKE_EPOCH_MS,
    });
    round_trip(Ban {
        guild_id: Id(2),
        user: user(),
        actor_id: Id(7),
        reason: None,
        created_at: SNOWFLAKE_EPOCH_MS,
    });
    let mask = Permissions::MEMBER | Permissions::from_bits_retain(1 << 60);
    round_trip(mask);
    assert!(serde_json::to_value(mask).unwrap().is_u64());
    assert_eq!(Permissions::MEMBER.bits(), 16671);
    assert_eq!(Permissions::ADMIN.bits(), 24575);
    assert_eq!(Permissions::GUILD_OWNER.bits(), 32767);
}

#[test]
fn named_wire_shape_and_patch_defaults_are_stable() {
    let value = ClientMsg::Heartbeat(Heartbeat {
        nonce: 1,
        last_seq: 2,
    });
    assert_eq!(
        serde_json::to_value(&value).unwrap(),
        serde_json::json!({
            "type": "heartbeat", "data": { "nonce": 1, "last_seq": 2 }
        })
    );
    // Fixed MessagePack fixture prevents an accidental switch to positional struct encoding.
    let expected = b"\x82\xa4type\xa9heartbeat\xa4data\x82\xa5nonce\x01\xa8last_seq\x02";
    assert_eq!(rmp_serde::to_vec_named(&value).unwrap(), expected);
    assert_eq!(rmp_serde::from_slice::<ClientMsg>(expected).unwrap(), value);
    assert_eq!(
        serde_json::from_str::<ProfilePatch>("{}").unwrap(),
        ProfilePatch::default()
    );
    assert_eq!(
        serde_json::to_value(Patch::<String>::Clear).unwrap(),
        serde_json::json!({"op": "clear"})
    );
    // Unknown fields are forward-compatible; unknown message variants are not silently ignored.
    let future =
        serde_json::json!({"type":"heartbeat","data":{"nonce":1,"last_seq":2,"future":true}});
    assert_eq!(serde_json::from_value::<ClientMsg>(future).unwrap(), value);
    assert!(serde_json::from_str::<ClientMsg>(r#"{"type":"future","data":{}}"#).is_err());
}

#[test]
fn maximum_bootstrap_metadata_fits_the_frame_budget() {
    let key = |offset: u64| Id(i64::MAX as u64 - 100_000 + offset);
    let mut profile = user();
    profile.id = key(1);
    profile.username = "u".repeat(32);
    profile.display_name = Some("🦀".repeat(16)); // 64 UTF-8 bytes
    profile.status_text = Some("s".repeat(128));
    profile.avatar_id = Some(key(2));
    let mut ready = Ready {
        roles: vec![],
        memberships: vec![],
        notification_settings: vec![],
        cursor: cursor(),
        me: SelfUser {
            user: profile.clone(),
            desired_presence: PresenceStatus::Dnd,
        },
        guilds: vec![],
        channels: vec![],
        relationships: vec![],
        users: vec![],
        read_states: vec![],
        presences: vec![],
        voice_states: vec![],
    };
    for g in 0..MAX_GUILDS_PER_USER {
        let mut detail = guild();
        detail.summary.guild.id = key(100 + g as u64);
        detail.summary.guild.owner_id = profile.id;
        detail.summary.guild.name = "g".repeat(64);
        detail.summary.guild.icon_id = Some(key(2));
        detail.summary.guild.member_count = MAX_GUILD_MEMBERS as u32;
        ready.guilds.push(detail.summary);
        for c in 0..MAX_CHANNELS_PER_GUILD {
            let mut summary = channel().summary;
            summary.id = key(1000 + (g * MAX_CHANNELS_PER_GUILD + c) as u64);
            summary.guild_id = Some(key(100 + g as u64));
            summary.name = Some("c".repeat(64));
            summary.position = c as u16;
            ready.channels.push(summary);
        }
    }
    for d in 0..MAX_OPEN_DMS_PER_USER {
        ready.channels.push(ChannelSummary {
            id: key(2000 + d as u64),
            kind: ChannelKind::GroupDm,
            guild_id: None,
            parent_id: None,
            name: Some("d".repeat(64)),
            position: 0,
            recipient_ids: (0..MAX_GROUP_PARTICIPANTS)
                .map(|i| key(3000 + i as u64))
                .collect(),
            owner_id: Some(profile.id),
        });
        let mut peer = profile.clone();
        peer.id = key(4000 + d as u64);
        ready.users.push(peer); // Conservative: groups don't require all these sidebar profiles.
    }
    for r in 0..MAX_RELATIONSHIPS_PER_USER {
        let mut peer = profile.clone();
        peer.id = key(5000 + r as u64);
        ready.relationships.push(Relationship {
            user: peer,
            kind: RelationshipKind::Friend,
            since: SNOWFLAKE_EPOCH_MS,
        });
    }
    for channel in &ready.channels {
        ready.read_states.push(ReadState {
            channel_id: channel.id,
            last_read_id: Some(key(6000)),
            latest_message_id: Some(key(6001)),
            unread_count: u32::MAX,
            mention_count: u32::MAX,
        });
    }
    // Upper bound on base presence: self + friends + distinct other group participants.
    let presence_count =
        1 + MAX_RELATIONSHIPS_PER_USER + MAX_OPEN_DMS_PER_USER * (MAX_GROUP_PARTICIPANTS - 1);
    for p in 0..presence_count {
        ready.presences.push(Presence {
            user_id: key(7000 + p as u64),
            status: PresenceStatus::Offline,
        });
    }
    for g in &ready.guilds {
        let guild_id = g.guild.id;
        let mut role_ids = vec![];
        for n in 0..MAX_ROLES_PER_GUILD {
            let id = if n == 0 {
                guild_id
            } else {
                key(9000 + ready.roles.len() as u64)
            };
            ready.roles.push(Role {
                id,
                guild_id,
                name: "r".repeat(100),
                color: 0xffffff,
                position: n as u16,
                hoist: true,
                mentionable: true,
                permissions: Permissions::all() - Permissions::OWNER,
            });
            role_ids.push(id);
        }
        ready.memberships.push(Member {
            guild_id,
            user: profile.clone(),
            role: MemberRole::Owner,
            permissions: Permissions::all(),
            joined_at: SNOWFLAKE_EPOCH_MS,
            role_ids,
            nickname: Some("🦀".repeat(32)),
        });
        ready.notification_settings.push(NotificationSettings {
            guild_id: Some(guild_id),
            channel_id: None,
            level: NotificationLevel::Mentions,
            muted: true,
            mute_until: Some(i64::MAX as u64),
            suppress_everyone: true,
            hide_muted_channels: true,
        });
    }
    for ch in &ready.channels {
        ready.notification_settings.push(NotificationSettings {
            guild_id: ch.guild_id,
            channel_id: Some(ch.id),
            level: NotificationLevel::Mentions,
            muted: true,
            mute_until: Some(i64::MAX as u64),
            suppress_everyone: true,
            hide_muted_channels: true,
        });
    }
    let wire = ServerMsg::Ready(Box::new(ready));
    let bytes = rmp_serde::to_vec_named(&wire).unwrap();
    println!("maximum launch Ready bytes: {}", bytes.len());
    assert!(
        bytes.len() <= MAX_WS_FRAME_BYTES as usize,
        "Ready was {} bytes",
        bytes.len()
    );
    round_trip(wire);
    round_trip(ServerMsg::Response(Box::new(CommandResponse {
        nonce: 123,
        cursor: cursor(),
        replayed: true,
        outcome: CommandOutcome::Ok(CommandResult::Message(message())),
    })));
}

#[test]
fn google_auth_contracts_are_additive_named_maps() {
    round_trip(MetaResponse {
        version: "0.1.0".into(),
        protocol_version: PROTOCOL_VERSION,
        google_client_id: None,
        gifs: GifCapability::Disabled,
    });
    round_trip(MetaResponse {
        version: "0.1.0".into(),
        protocol_version: PROTOCOL_VERSION,
        google_client_id: Some("test-client".into()),
        gifs: GifCapability::Disabled,
    });
    round_trip(GoogleAuthRequest {
        id_token: "signed.id.token".into(),
    });
    round_trip(GoogleLinkRequest {
        id_token: "signed.id.token".into(),
    });
    round_trip(GoogleUnlinkRequest {});
    round_trip(GoogleCompleteRequest {
        ticket: "opaque-ticket".into(),
        username: "alice".into(),
    });
    round_trip(AuthMethodsResponse {
        has_password: false,
        google_linked: true,
    });
    round_trip(SetPasswordRequest {
        current_password: None,
        new_password: "long new password".into(),
    });
    round_trip(SetPasswordRequest {
        current_password: Some("old password".into()),
        new_password: "long new password".into(),
    });
    let signup = GoogleAuthResponse::SignupRequired(GoogleSignupRequired {
        ticket: "opaque-ticket".into(),
        expires_at: 1234,
    });
    assert_eq!(
        serde_json::to_value(&signup).unwrap(),
        serde_json::json!({"type":"signup_required","data":{"ticket":"opaque-ticket","expires_at":1234}})
    );
    round_trip(signup);
    let session = GoogleAuthResponse::Session(AuthResponse {
        token: "session-token".into(),
        expires_at: 5678,
        user: me(),
    });
    assert_eq!(serde_json::to_value(&session).unwrap()["type"], "session");
    round_trip(session);
    for (code, wire) in [
        (ErrorCode::FeatureDisabled, "feature_disabled"),
        (ErrorCode::InvalidSignupTicket, "invalid_signup_ticket"),
        (ErrorCode::IdentityAlreadyLinked, "identity_already_linked"),
        (ErrorCode::LastLoginMethod, "last_login_method"),
    ] {
        assert_eq!(serde_json::to_value(code).unwrap(), wire);
        round_trip(code);
    }
}

#[test]
fn gif_discovery_and_favorites_contracts_are_additive() {
    let legacy: MetaResponse = serde_json::from_value(serde_json::json!({
        "version":"0.1.0", "protocol_version":1, "google_client_id":null
    }))
    .unwrap();
    assert_eq!(legacy.gifs, GifCapability::Disabled);
    assert_eq!(serde_json::to_value(&legacy).unwrap()["gifs"], false);
    assert!(serde_json::from_value::<GifCapability>(serde_json::json!(true)).is_err());
    round_trip(GifCapability::Disabled);
    round_trip(GifCapability::Enabled(GifConfig::Proxy {
        provider: GifProviderName::Klipy,
    }));
    round_trip(GifCapability::Enabled(GifConfig::Direct {
        provider: GifProviderName::Klipy,
        public_client_key: "public-key".into(),
    }));
    let media = GifMedia {
        url: "https://static.klipy.com/tiny.webm".into(),
        width: 150,
        height: 100,
        content_type: "video/webm".into(),
    };
    let item = GifItem {
        id: "klipy:123".into(),
        title: "A friendly wave".into(),
        preview: media.clone(),
        send: media.clone(),
    };
    round_trip(item.clone());
    round_trip(GifPage {
        items: vec![item],
        next_cursor: Some("opaque+/=".into()),
    });
    round_trip(GifPage {
        items: vec![],
        next_cursor: None,
    });
    round_trip(GifQuery {
        q: Some("café & happy".into()),
        cursor: Some("opaque+/=".into()),
    });
    round_trip(GifQuery::default());
    round_trip(GifItemsQuery {
        ids: "klipy:123,klipy:456".into(),
    });
    round_trip(GifCategoriesResponse {
        items: vec![GifCategory {
            title: "Happy".into(),
            query: "happy".into(),
            preview: media,
        }],
    });
    round_trip(GifFavoritesResponse {
        ids: vec!["klipy:123".into()],
    });
}

#[test]
fn old_page_query_remains_decodable() {
    let query: PageQuery =
        serde_json::from_str(r#"{"before":null,"after":null,"limit":50}"#).unwrap();
    assert_eq!(query.around, None);
    assert_eq!(query.around_timestamp, None);
    round_trip(PageQuery {
        around: Some(Id(42)),
        around_timestamp: None,
        before: None,
        after: None,
        limit: Some(50),
    });
    round_trip(PageQuery {
        around: None,
        around_timestamp: Some(SNOWFLAKE_EPOCH_MS),
        before: None,
        after: None,
        limit: Some(1),
    });
}
