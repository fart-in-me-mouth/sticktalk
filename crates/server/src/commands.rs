use crate::{
    db::*,
    error::{require, Error, Result},
    state::{hash, now, App, Core},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use sqlx::{Row, SqliteConnection};
use std::collections::HashMap;
use sticktalk_proto::*;
type Deliveries = Vec<(Vec<Id>, Event)>;
fn event(out: &mut Deliveries, users: Vec<Id>, e: Event) {
    if let Some((previous_users, previous_event)) = out.last_mut() {
        if *previous_event == e {
            previous_users.extend(users);
            previous_users.sort();
            previous_users.dedup();
            return;
        }
    }
    out.push((users, e));
}
async fn relationship_event(
    c: &mut SqliteConnection,
    out: &mut Deliveries,
    u: Id,
    v: Id,
) -> Result<()> {
    let e = match relationship(c, u, v).await? {
        Some(r) => Event::RelationshipUpsert(r),
        None => Event::RelationshipRemoved { user_id: v },
    };
    event(out, vec![u], e);
    Ok(())
}
async fn relationship_capacity(c: &mut SqliteConnection, u: Id, v: Id) -> Result<()> {
    let n: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM relationships WHERE user_id=? AND other_id<>?")
            .bind(id(u)?)
            .bind(id(v)?)
            .fetch_one(c)
            .await?;
    require(n < 100, ErrorCode::LimitExceeded)
}
async fn put_relationship(
    c: &mut SqliteConnection,
    u: Id,
    v: Id,
    kind: RelationshipKind,
) -> Result<()> {
    sqlx::query("INSERT INTO relationships(user_id,other_id,kind,since) VALUES(?,?,?,?) ON CONFLICT(user_id,other_id) DO UPDATE SET kind=excluded.kind,since=excluded.since").bind(id(u)?).bind(id(v)?).bind(enum_str(kind)).bind(now() as i64).execute(c).await?;
    Ok(())
}
async fn guild_events(c: &mut SqliteConnection, out: &mut Deliveries, g: Id) -> Result<()> {
    for u in guild_users(c, g).await? {
        event(out, vec![u], Event::GuildUpsert(guild(c, u, g).await?));
    }
    Ok(())
}
async fn member_event(c: &mut SqliteConnection, out: &mut Deliveries, g: Id, u: Id) -> Result<()> {
    event(
        out,
        guild_users(c, g).await?,
        Event::MemberUpsert(member(c, g, u).await?),
    );
    Ok(())
}
async fn invite_users(c: &mut SqliteConnection, i: &Invite) -> Result<Vec<Id>> {
    let mut users = vec![];
    for u in guild_users(c, i.guild_id).await? {
        if u == i.creator_id
            || crate::permissions::evaluate(c, u, i.guild_id, None)
                .await?
                .contains(Permissions::MANAGE_GUILD)
        {
            users.push(u);
        }
    }
    users.sort();
    users.dedup();
    Ok(users)
}
pub(crate) async fn expire_invites(c: &mut SqliteConnection, core: &mut Core) -> Result<()> {
    let ids=sqlx::query_scalar::<_,i64>("SELECT id FROM invites WHERE inactive_notified=0 AND (expires_at<=? OR uses>=max_uses) LIMIT 100")
        .bind(now() as i64).fetch_all(&mut *c).await?;
    for iid in ids {
        let i = invite(c, Id(iid as u64)).await?;
        let users = invite_users(c, &i).await?;
        sqlx::query("UPDATE invites SET inactive_notified=1 WHERE id=?")
            .bind(iid)
            .execute(&mut *c)
            .await?;
        core.emit(
            &users,
            Event::InviteRemoved {
                guild_id: i.guild_id,
                invite_id: i.id,
            },
        )?;
    }
    sqlx::query("DELETE FROM invites WHERE id IN(SELECT id FROM invites WHERE inactive_notified=1 AND expires_at<? LIMIT 100)").bind(now().saturating_sub(86_400_000) as i64).execute(c).await?;
    Ok(())
}
async fn admins(c: &mut SqliteConnection, g: Id) -> Result<Vec<Id>> {
    let mut out = vec![];
    for u in guild_users(c, g).await? {
        if crate::permissions::evaluate(c, u, g, None)
            .await?
            .contains(Permissions::BAN_MEMBERS)
        {
            out.push(u);
        }
    }
    Ok(out)
}
async fn channel_event(c: &mut SqliteConnection, out: &mut Deliveries, ch: Id) -> Result<Channel> {
    let ch = channel(c, ch).await?;
    let users = if ch.summary.guild_id.is_none() {
        sqlx::query_scalar::<_, i64>(
            "SELECT user_id FROM dm_members WHERE channel_id=? AND visible=1 ORDER BY user_id",
        )
        .bind(id(ch.summary.id)?)
        .fetch_all(&mut *c)
        .await?
        .into_iter()
        .map(|v| Id(v as u64))
        .collect()
    } else {
        audience(c, ch.summary.id).await?
    };
    event(out, users, Event::ChannelUpsert(ch.clone()));
    Ok(ch)
}
pub(crate) async fn reads_event(
    c: &mut SqliteConnection,
    out: &mut Deliveries,
    ch: Id,
) -> Result<()> {
    recount(c, ch).await?;
    let readers = audience(c, ch).await?;
    for u in crate::threads::readers(c, ch, readers).await? {
        let visible = if channel(c, ch).await?.summary.guild_id.is_none() {
            sqlx::query_scalar::<_, i64>(
                "SELECT visible FROM dm_members WHERE channel_id=? AND user_id=?",
            )
            .bind(id(ch)?)
            .bind(id(u)?)
            .fetch_one(&mut *c)
            .await?
                != 0
        } else {
            true
        };
        if visible {
            event(out, vec![u], Event::ReadStateUpdated(read(c, u, ch).await?));
        }
    }
    Ok(())
}
pub(crate) async fn joined(
    c: &mut SqliteConnection,
    out: &mut Deliveries,
    g: Id,
    u: Id,
) -> Result<()> {
    let detail = detail(c, u, g).await?;
    let mut reads = vec![];
    for ch in &detail.channels {
        if matches!(
            ch.kind,
            ChannelKind::Text | ChannelKind::Voice | ChannelKind::Announcement
        ) {
            init_read(c, u, ch.id, true).await?;
            reads.push(read(c, u, ch.id).await?);
        }
    }
    event(
        out,
        vec![u],
        Event::GuildJoined {
            detail,
            read_states: reads,
        },
    );
    event(
        out,
        vec![u],
        Event::RolesUpdated {
            guild_id: g,
            roles: crate::permissions::roles(c, g).await?,
        },
    );
    guild_events(c, out, g).await?;
    member_event(c, out, g, u).await
}
/// A channel may be nested in any other channel of its guild.
async fn parent(c: &mut SqliteConnection, g: Id, p: Option<Id>) -> Result<()> {
    if let Some(p) = p {
        let ch = channel(c, p)
            .await
            .map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
        require(ch.summary.guild_id == Some(g), ErrorCode::InvalidRequest)?;
        // Threads and forums hold threads (posts), not channels.
        require(
            !matches!(ch.summary.kind, ChannelKind::Thread | ChannelKind::Forum),
            ErrorCode::InvalidRequest,
        )?;
    }
    Ok(())
}
async fn append_position(c: &mut SqliteConnection, g: Id, p: Option<Id>) -> Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT COUNT(*) FROM channels WHERE guild_id=? AND parent_id IS ? AND kind<>'thread'",
    )
    .bind(id(g)?)
    .bind(opt(p)?)
    .fetch_one(c)
    .await?)
}
async fn compact(c: &mut SqliteConnection, out: &mut Deliveries, g: Id) -> Result<()> {
    let viewer = Id(
        sqlx::query_scalar::<_, i64>("SELECT owner_id FROM guilds WHERE id=?")
            .bind(id(g)?)
            .fetch_one(&mut *c)
            .await? as u64,
    );
    let all = channels(c, viewer, Some(g)).await?;
    let mut positions: HashMap<Option<Id>, u16> = HashMap::new();
    for ch in all {
        let p = positions.entry(ch.parent_id).or_default();
        sqlx::query("UPDATE channels SET position=? WHERE id=?")
            .bind(i64::from(*p))
            .bind(id(ch.id)?)
            .execute(&mut *c)
            .await?;
        *p += 1;
    }
    let placements = channels(c, viewer, Some(g))
        .await?
        .into_iter()
        .map(|ch| ChannelPlacement {
            channel_id: ch.id,
            parent_id: ch.parent_id,
            position: ch.position,
        })
        .collect();
    event(
        out,
        guild_users(c, g).await?,
        Event::ChannelsReordered {
            guild_id: g,
            placements,
        },
    );
    Ok(())
}
async fn attach(
    c: &mut SqliteConnection,
    u: Id,
    a: Id,
    p: UploadPurpose,
    ch: Option<Id>,
) -> Result<Attachment> {
    let r=sqlx::query("SELECT * FROM uploads WHERE id=? AND owner_id=? AND purpose=? AND channel_id IS ? AND expires_at>?").bind(id(a)?).bind(id(u)?).bind(enum_str(p)).bind(opt(ch)?).bind(now() as i64).fetch_optional(&mut *c).await?.ok_or_else(||Error::new(ErrorCode::InvalidAttachment))?;
    sqlx::query("UPDATE uploads SET expires_at=NULL WHERE id=?")
        .bind(id(a)?)
        .execute(c)
        .await?;
    Ok(attachment_row(&r))
}
fn patch_text(
    old: Option<String>,
    p: Patch<String>,
    max: usize,
    nullable: bool,
) -> Result<Option<String>> {
    match p {
        Patch::Keep => Ok(old),
        Patch::Set(s) => Ok(Some(name(&s, max)?)),
        Patch::Clear => {
            require(nullable, ErrorCode::InvalidRequest)?;
            Ok(None)
        }
    }
}
async fn validate_message(
    c: &mut SqliteConnection,
    u: Id,
    ch: Id,
    content: &str,
    mentions: &[Id],
    attachments: usize,
) -> Result<Channel> {
    let ch = text(c, u, ch, Permissions::SEND_MESSAGES).await?;
    if ch.summary.kind == ChannelKind::Direct {
        let peer = *ch
            .summary
            .recipient_ids
            .iter()
            .find(|v| **v != u)
            .ok_or_else(|| Error::new(ErrorCode::Internal))?;
        require(!blocked(c, u, peer).await?, ErrorCode::Blocked)?;
    }
    require(
        content.chars().count() <= 4000 && (!content.trim().is_empty() || attachments > 0),
        ErrorCode::InvalidRequest,
    )?;
    if attachments > 0 {
        access(c, u, ch.summary.id, Permissions::ATTACH_FILES).await?;
    }
    unique(mentions, 100)?;
    for v in mentions {
        text(c, *v, ch.summary.id, Permissions::VIEW_CHANNEL)
            .await
            .map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
    }
    Ok(ch)
}
// V1 distinguishes a visible cross-channel target (invalid_request) from an
// inaccessible/missing target (not_found), without making message IDs an oracle.
async fn command_message(
    c: &mut SqliteConnection,
    u: Id,
    ch: Id,
    m: Id,
    deleted: bool,
) -> Result<Message> {
    text(c, u, ch, Permissions::VIEW_CHANNEL).await?;
    match message(c, u, ch, m, deleted).await {
        Err(e) if e.0.code == ErrorCode::NotFound => {
            let actual: Option<i64> =
                sqlx::query_scalar("SELECT channel_id FROM messages WHERE id=?")
                    .bind(id(m)?)
                    .fetch_optional(&mut *c)
                    .await?;
            if let Some(actual) = actual.filter(|other| *other != ch.0 as i64) {
                if text(c, u, Id(actual as u64), Permissions::READ_HISTORY)
                    .await
                    .is_ok()
                {
                    return Err(Error::new(ErrorCode::InvalidRequest));
                }
            }
            Err(e)
        }
        result => result,
    }
}
async fn remove_member(
    c: &mut SqliteConnection,
    out: &mut Deliveries,
    g: Id,
    u: Id,
    reason: RemovalReason,
) -> Result<()> {
    sqlx::query("DELETE FROM voice_connect_denials WHERE user_id=? AND channel_id IN (SELECT id FROM channels WHERE guild_id=?)").bind(id(u)?).bind(id(g)?).execute(&mut *c).await?;
    sqlx::query("DELETE FROM guild_members WHERE guild_id=? AND user_id=?")
        .bind(id(g)?)
        .bind(id(u)?)
        .execute(&mut *c)
        .await?;
    sqlx::query("DELETE FROM read_states WHERE user_id=? AND channel_id IN (SELECT id FROM channels WHERE guild_id=?)").bind(id(u)?).bind(id(g)?).execute(&mut *c).await?;
    event(
        out,
        vec![u],
        Event::GuildRemoved {
            guild_id: g,
            reason,
        },
    );
    event(
        out,
        guild_users(c, g).await?,
        Event::MemberRemoved {
            guild_id: g,
            user_id: u,
            reason,
        },
    );
    guild_events(c, out, g).await
}
pub(crate) async fn broadcast_presence(
    c: &mut SqliteConnection,
    core: &mut Core,
    u: Id,
) -> Result<()> {
    let mut viewers: Vec<_> = core.streams.values().map(|s| s.user).collect();
    viewers.sort();
    viewers.dedup();
    for viewer in viewers {
        let base = base_presence(c, viewer).await?;
        let interested = core
            .streams
            .values()
            .any(|s| s.user == viewer && s.interests.contains(&u));
        if (viewer == u || base.contains(&u) || interested)
            && (viewer == u || !blocked(c, viewer, u).await?)
        {
            let p = presence(c, core, viewer, u).await?;
            if viewer == u || base.contains(&u) {
                core.emit(&[viewer], Event::PresenceUpdated(p))?;
            } else {
                core.emit_filtered(&[viewer], Event::PresenceUpdated(p), |s| {
                    s.interests.contains(&u)
                })?;
            }
        }
    }
    Ok(())
}
pub(crate) async fn execute(
    app: &App,
    core: &mut Core,
    key: &[u8],
    u: Id,
    req: CommandRequest,
) -> Result<CommandResponse> {
    let durable = !matches!(
        req.command,
        Command::Typing { .. }
            | Command::SubscribeGuild { .. }
            | Command::Launch(LaunchCommand::LogoutSession { .. } | LaunchCommand::LogoutAllOthers)
    );
    let mut c = app.0.pool.acquire().await?;
    let valid: Option<i64> =
        sqlx::query_scalar("SELECT expires_at FROM sessions WHERE token_hash=?")
            .bind(key)
            .fetch_optional(&mut *c)
            .await?;
    require(
        valid.is_some_and(|v| v > now() as i64),
        ErrorCode::Unauthenticated,
    )?;
    sqlx::query("UPDATE sessions SET last_used_at=? WHERE token_hash=? AND last_used_at<?")
        .bind(now() as i64)
        .bind(key)
        .bind(now().saturating_sub(60_000) as i64)
        .execute(&mut *c)
        .await?;
    let cursor = core.cursor(key, u);
    // Sending a launch command also proves support for its event catalog.
    if matches!(req.command, Command::Launch(_)) {
        if let Some(s) = core.streams.get_mut(key) {
            if !s.launch_features {
                s.clear_replay();
            }
            s.launch_features = true;
        }
    }
    if matches!(req.command, Command::Content(_)) {
        if let Some(s) = core.streams.get_mut(key) {
            s.content_features = true;
        }
    }
    if matches!(req.command, Command::Threads(_)) {
        if let Some(s) = core.streams.get_mut(key) {
            s.thread_features = true;
        }
    }
    if matches!(req.command, Command::Expression(_)) {
        if let Some(s) = core.streams.get_mut(key) {
            s.expression_features = true;
        }
    }
    let mut replayed = false;
    let outcome: Result<CommandResult>=async {
        require(req.nonce>0&&req.nonce<=i64::MAX as u64,ErrorCode::InvalidRequest)?;
        let request_hash=hash(&rmp_serde::to_vec_named(&req.command).map_err(|_|Error::new(ErrorCode::Internal))?);
        if durable {
            if let Some(r)=sqlx::query("SELECT request_hash,result_msgpack FROM command_receipts WHERE session_hash=? AND nonce=? AND expires_at>?").bind(key).bind(req.nonce as i64).bind(now() as i64).fetch_optional(&mut *c).await? {
                require(r.get::<Vec<u8>,_>("request_hash")==request_hash,ErrorCode::Conflict)?;replayed=true;
                let mut result:CommandResult=rmp_serde::from_slice(&r.get::<Vec<u8>,_>("result_msgpack")).map_err(|_|Error::new(ErrorCode::Internal))?;
                result_access(&mut c,u,&result).await?;
                match &mut result {
                    CommandResult::Channel(ch)=>*ch=access(&mut c,u,ch.summary.id,Permissions::VIEW_CHANNEL).await?,
                    CommandResult::Guild(g)=>*g=detail(&mut c,u,g.summary.guild.id).await?,
                    CommandResult::Member(m)=>*m=member(&mut c,m.guild_id,m.user.id).await?,
                    _=>{}
                }
                return Ok(result);
            }
            core.rate(format!("cmd:{}",u.0),120.,20.)?;
            if matches!(req.command,Command::SendMessage(_)) {core.rate(format!("send:{}",u.0),60.,10.)?;}
            let n:i64=sqlx::query_scalar("SELECT COUNT(*) FROM command_receipts WHERE session_hash=? AND expires_at>?").bind(key).bind(now() as i64).fetch_one(&mut *c).await?;require(n<256,ErrorCode::LimitExceeded)?;
        }
        let mut moderation_hook = Box::pin(crate::moderation::precheck(app, core, &mut c, u, &req.command)).await?;
        let visibility_users = visibility_users(&mut c, u, &req.command).await?;
        let mut previous_base = Vec::new();
        for viewer in visibility_users { previous_base.push((viewer, base_presence(&mut c, viewer).await?)); }
        let previous_interests = core.streams.get(key).map(|s| s.interests.clone()).unwrap_or_default();
        let subscription_changed = matches!(req.command, Command::SubscribeGuild { .. });
        let acl_guild = crate::launch::acl_guild(&mut c,&req.command).await?;
        let mut before=vec![];
        if let Some(g)=acl_guild {for v in guild_users(&mut c,g).await? {before.push((v,channels(&mut c,v,Some(g)).await?));}}
        let mut tx=c.begin().await?;let mut out=vec![];
        let revoke_sessions=matches!(req.command,Command::Launch(LaunchCommand::LogoutSession{..}|LaunchCommand::LogoutAllOthers));
        let presence_changed=matches!(req.command,Command::SetPresence{..}|Command::Privacy(PrivacyCommand::SetStatus{..}));
        let needs_file_gc=matches!(req.command,Command::DeleteGuild{..}|Command::DeleteChannel{..}|Command::LeaveGroupDm{..});
        let block_pair=if let Command::BlockUser{user_id}=req.command {Some(user_id)}else{None};
        let voice_acl_changed=acl_guild.is_some() || revoke_sessions || crate::community::voice_acl(&req.command) || matches!(req.command,Command::SetVoicePermission{..}|Command::SetMemberRole{..}|Command::TransferGuildOwnership{..}|Command::KickMember{..}|Command::BanMember{..}|Command::LeaveGuild{..}|Command::DeleteGuild{..}|Command::DeleteChannel{..});
        let voice_snapshot=matches!(req.command,Command::JoinGuild{..});
        let role_target=if let Command::SetMemberRole{user_id,..}=req.command {Some(user_id)}else{None};
        let deleted_channel=if let Command::DeleteChannel {channel_id}=req.command {Some(channel_id)} else {None};
        let call_acl_changed=crate::calls::acl_changed(&req.command);
        let mut revoked_visibility=vec![];
        let mut result=Box::pin(mutate(&mut tx,core,key,u,req.command,&mut out)).await?;
        crate::moderation::record(&mut tx, u, &mut moderation_hook, &result).await?;
        if let CommandResult::Channel(ch)=&mut result {*ch=access(&mut tx,u,ch.summary.id,Permissions::VIEW_CHANNEL).await?;}
        if let Some(g)=acl_guild {
            for (v,old) in &before {
                let after=channels(&mut tx,*v,Some(g)).await?;
                if old.iter().any(|ch|Some(ch.id)!=deleted_channel && !after.iter().any(|a|a.id==ch.id)) {revoked_visibility.push(*v);}
                for ch in old.iter().filter(|ch|!after.iter().any(|a|a.id==ch.id)) {out.push((vec![*v],Event::ChannelRemoved {channel_id:ch.id,reason:RemovalReason::Left}));}
                for ch in &after {
                    let newly_visible=!old.iter().any(|o|o.id==ch.id);
                    let mut full=channel(&mut tx,ch.id).await?;full.summary=ch.clone();
                    out.push((vec![*v],Event::ChannelUpsert(full)));
                    if newly_visible && matches!(ch.kind,ChannelKind::Text|ChannelKind::Voice|ChannelKind::Announcement) {init_read(&mut tx,*v,ch.id,true).await?;out.push((vec![*v],Event::ReadStateUpdated(read(&mut tx,*v,ch.id).await?)));}
                }
            }
        }
        if durable {let result_bytes=rmp_serde::to_vec_named(&result).map_err(|_|Error::new(ErrorCode::Internal))?;sqlx::query("INSERT INTO command_receipts(session_hash,nonce,request_hash,result_msgpack,created_at,expires_at) VALUES(?,?,?,?,?,?) ON CONFLICT(session_hash,nonce) DO UPDATE SET request_hash=excluded.request_hash,result_msgpack=excluded.result_msgpack,created_at=excluded.created_at,expires_at=excluded.expires_at") .bind(key).bind(req.nonce as i64).bind(request_hash).bind(result_bytes).bind(now() as i64).bind((now()+600_000) as i64).execute(&mut *tx).await?;}
        crate::identity::settle(&mut tx, core, &mut out).await?;
        let changed=!out.is_empty();
        let orphaned=gc_uploads(&mut tx).await?;
        core.persist_id(&mut tx).await?;
        tx.commit().await?;
        // Discard queued pre-revocation packets as well as replay. The writer's
        // biased close branch drops its pending queue; reconnect obtains removals/Ready.
        for s in core.streams.values_mut().filter(|s|revoked_visibility.contains(&s.user)) {
            if let Some(connection)=s.connection.take() {let _=connection.close.send(Some(4003));}
        }
        if acl_guild.is_some() {core.invalidate(&before.iter().map(|(u,_)|*u).collect::<Vec<_>>());}
        if revoke_sessions {
        let mut revoked=vec![];
        for (k,s) in &core.streams {
            if s.user==u && sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM sessions WHERE token_hash=?").bind(k).fetch_one(&mut *c).await?==0 {revoked.push(k.clone());}
        }
        for k in revoked {if let Some(s)=core.streams.remove(&k) {if let Some(c)=s.connection {let _=c.close.send(Some(4001));}}}

        }
        if voice_acl_changed { crate::voice::reconcile(app, core, &mut c).await?; }
        if let Some(v)=block_pair {core.invalidate(&[u,v]);}
        if let Some(v)=role_target {core.invalidate(&[v]);}
        if call_acl_changed && !voice_acl_changed { crate::voice::reconcile(app, core, &mut c).await?; }
        if needs_file_gc || !orphaned.is_empty() {app.0.tier.wake.notify_one();let app=app.clone();tokio::spawn(async move{let _=app.collect_orphan_files().await;});}
        // Access revocation invalidates old body-bearing replay before admitting removal events.
        for (users,e) in &out {if matches!(e,Event::GuildRemoved{..}|Event::ChannelRemoved{..}|Event::RelationshipRemoved{..}|Event::MessageDeleted{..}) {core.invalidate(users);}}
        let mut reaction_alerted=false;
        for (users,e) in out {
            if let Event::MessageCreated { ref message, .. } = e { crate::push::message(app, &mut c, &users, message).await; }
            if let Event::MessageCreated{message,..}=&e {crate::mentions::notify(app,core,&mut c,message).await?;}
            crate::content::observe(app, &e);
            crate::threads::observe(app, &e);
            // One alert per message, and per reaction (its event is per viewer).
            let alert=match &e {Event::MessageCreated{..}=>Some(e.clone()),Event::ReactionUpdated(_) if !reaction_alerted=>{reaction_alerted=true;Some(e.clone())},_=>None};
            crate::community::observe(core, &mut c, &e).await;
            let deliveries=crate::launch::filter_event(&mut c,users,e).await?;
            for (users,e) in deliveries {
            if let Event::UserUpdated(ref profile) = e {
                let mut streams = Vec::new();
                for s in core.streams.values().filter(|s| users.contains(&s.user)) {
                    if profile_projection(&mut c, s.user, profile.id, s.guild).await? { streams.push(s.id.clone()); }
                }
                core.emit_filtered(&users, e, |s| streams.contains(&s.id))?;
            } else { core.emit(&users,e)?; }
        }
            if let Some(e)=alert { crate::privacy::observe(app, core, &mut c, &e).await; }
        }
        if let Some(v) = block_pair.filter(|_| changed) {
            for (viewer, target) in [(u, v), (v, u)] {
                let was_base = previous_base.iter().any(|(who, peers)| *who == viewer && peers.contains(&target));
                core.emit_filtered(&[viewer], Event::PresenceUpdated(Presence { user_id: target, status: PresenceStatus::Offline }), |s| !was_base && s.interests.contains(&target))?;
            }
        }
        if voice_snapshot { crate::voice::snapshot(core, &mut c, key, u).await?; }
        sync_base_presence(&mut c, core, previous_base).await?;
        if subscription_changed {
            let base = base_presence(&mut c, u).await?;
            let stream = core.streams.get(key).ok_or_else(|| Error::new(ErrorCode::Internal))?;
            let stream_id = stream.id.clone();
            let ended: Vec<_> = previous_interests.into_iter().filter(|v| !stream.interests.contains(v) && !base.contains(v)).collect();
            for user_id in ended { core.emit_filtered(&[u], Event::PresenceUpdated(Presence { user_id, status: PresenceStatus::Offline }), |s| s.id == stream_id)?; }
        }
        // Presence is synthesized against the committed projection, not supplied by clients.
        if presence_changed && changed { broadcast_presence(&mut c,core,u).await?; }
        Box::pin(crate::moderation::after(app, core, &mut c, u, moderation_hook)).await?;
        Ok(result)
    }.await;
    let outcome = match outcome {
        Ok(v) => CommandOutcome::Ok(v),
        Err(e) => CommandOutcome::Err(e.0),
    };
    Ok(CommandResponse {
        nonce: req.nonce,
        cursor: core.streams.get(key).map(|s| s.cursor()).unwrap_or(cursor),
        replayed,
        outcome,
    })
}
use sqlx::Connection as _;
async fn visibility_users(c: &mut SqliteConnection, u: Id, cmd: &Command) -> Result<Vec<Id>> {
    use Command::*;
    let mut users = match cmd {
        AcceptFriendRequest { user_id }
        | RemoveFriend { user_id }
        | BlockUser { user_id }
        | UnblockUser { user_id }
        | OpenDm { user_id } => vec![u, *user_id],
        CreateGroupDm { user_ids, .. } => {
            let mut v = user_ids.clone();
            v.push(u);
            v
        }
        AddGroupRecipient {
            channel_id,
            user_id,
        }
        | RemoveGroupRecipient {
            channel_id,
            user_id,
        } => {
            let ch = access(c, u, *channel_id, Permissions::VIEW_CHANNEL).await?;
            let mut v = ch.summary.recipient_ids;
            v.push(*user_id);
            v
        }
        LeaveGroupDm { channel_id } | CloseDm { channel_id } => {
            access(c, u, *channel_id, Permissions::VIEW_CHANNEL)
                .await?
                .summary
                .recipient_ids
        }
        SendMessage(d) => {
            access(c, u, d.channel_id, Permissions::VIEW_CHANNEL)
                .await?
                .summary
                .recipient_ids
        }
        _ => vec![],
    };
    users.sort();
    users.dedup();
    users.retain(|v| id(*v).is_ok());
    Ok(users)
}
async fn sync_base_presence(
    c: &mut SqliteConnection,
    core: &mut Core,
    before: Vec<(Id, Vec<Id>)>,
) -> Result<()> {
    for (viewer, old) in before {
        let current = base_presence(c, viewer).await?;
        for target in current.iter().filter(|v| !old.contains(v)) {
            let p = presence(c, core, viewer, *target).await?;
            core.emit(&[viewer], Event::PresenceUpdated(p))?;
        }
        for target in old.iter().filter(|v| !current.contains(v)) {
            let blocked = blocked(c, viewer, *target).await?;
            core.emit_filtered(
                &[viewer],
                Event::PresenceUpdated(Presence {
                    user_id: *target,
                    status: PresenceStatus::Offline,
                }),
                |s| blocked || !s.interests.contains(target),
            )?;
        }
    }
    Ok(())
}
pub(crate) async fn profile_projection(
    c: &mut SqliteConnection,
    viewer: Id,
    target: Id,
    selected: Option<Id>,
) -> Result<bool> {
    let visible: i64 = sqlx::query_scalar("SELECT (?=?) OR EXISTS(SELECT 1 FROM relationships WHERE user_id=? AND other_id=?) OR EXISTS(SELECT 1 FROM dm_members a JOIN dm_members b ON a.channel_id=b.channel_id WHERE a.user_id=? AND a.visible=1 AND b.user_id=?) OR EXISTS(SELECT 1 FROM guild_members a JOIN guild_members b ON a.guild_id=b.guild_id WHERE a.guild_id=? AND a.user_id=? AND b.user_id=?)")
        .bind(id(viewer)?).bind(id(target)?).bind(id(viewer)?).bind(id(target)?).bind(id(viewer)?).bind(id(target)?).bind(opt(selected)?).bind(id(viewer)?).bind(id(target)?).fetch_one(c).await?;
    Ok(visible != 0)
}
async fn result_access(c: &mut SqliteConnection, u: Id, r: &CommandResult) -> Result<()> {
    match r {
        CommandResult::Role(r) => {
            role(c, u, r.guild_id).await?;
            require(
                crate::permissions::roles(c, r.guild_id)
                    .await?
                    .iter()
                    .any(|v| v.id == r.id),
                ErrorCode::NotFound,
            )?;
        }
        CommandResult::NotificationSettings(s) => {
            if let Some(ch) = s.channel_id {
                access(c, u, ch, Permissions::VIEW_CHANNEL).await?;
            } else if let Some(g) = s.guild_id {
                role(c, u, g).await?;
            }
        }
        CommandResult::Channel(ch) => {
            access(c, u, ch.summary.id, Permissions::VIEW_CHANNEL).await?;
        }
        CommandResult::Message(m) => {
            text(c, u, m.channel_id, Permissions::READ_HISTORY).await?;
            message(c, u, m.channel_id, m.id, false).await?;
        }
        CommandResult::Guild(g) => {
            role(c, u, g.summary.guild.id).await?;
            for ch in &g.channels {
                access(c, u, ch.id, Permissions::VIEW_CHANNEL).await?;
            }
        }
        CommandResult::Member(m) => {
            role(c, u, m.guild_id).await?;
            role(c, m.user.id, m.guild_id).await?;
        }
        CommandResult::Invite(i) => {
            role(c, u, i.guild_id).await?;
            let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM invites WHERE id=?")
                .bind(id(i.id)?)
                .fetch_one(&mut *c)
                .await?;
            require(exists != 0, ErrorCode::NotFound)?;
            require(
                i.creator_id == u
                    || crate::permissions::evaluate(c, u, i.guild_id, None)
                        .await?
                        .contains(Permissions::MANAGE_GUILD),
                ErrorCode::NotFound,
            )?;
        }
        CommandResult::ReadState(r) => {
            text(c, u, r.channel_id, Permissions::VIEW_CHANNEL).await?;
        }
        CommandResult::Reaction(r) => {
            text(c, u, r.channel_id, Permissions::READ_HISTORY).await?;
            message(c, u, r.channel_id, r.message_id, false).await?;
        }
        CommandResult::Relationship(r) => {
            profile_access(c, u, r.user.id).await?;
        }
        _ => {}
    }
    Ok(())
}
pub(crate) async fn mutate(
    c: &mut SqliteConnection,
    core: &mut Core,
    key: &[u8],
    u: Id,
    cmd: Command,
    out: &mut Deliveries,
) -> Result<CommandResult> {
    use Command::*;
    let adding_reaction = matches!(cmd, Command::AddReaction { .. });
    match cmd {
        // Boxed: each feature's future is large, and holding them all inline in
        // this one overflowed a 2 MB stack in debug builds.
        Launch(cmd) => Box::pin(crate::launch::mutate(c, core, key, u, cmd, out)).await,
        Content(cmd) => Box::pin(crate::content::mutate(c, core, key, u, cmd, out)).await,
        Threads(cmd) => Box::pin(crate::threads::mutate(c, core, key, u, cmd, out)).await,
        Expression(cmd) => Box::pin(crate::expression::mutate(c, core, key, u, cmd, out)).await,
        Privacy(cmd) => Box::pin(crate::privacy::mutate(c, core, key, u, cmd, out)).await,
        Community(cmd) => Box::pin(crate::community::mutate(c, core, key, u, cmd, out)).await,
        Identity(cmd) => Box::pin(crate::identity::mutate(c, core, u, cmd, out)).await,
        UpdateProfile(p) => {
            let old = me(c, u).await?;
            let mut user = old.user.clone();
            user.username = match p.username {
                Patch::Keep => user.username,
                Patch::Set(s) => username(&s)?,
                Patch::Clear => return Err(Error::new(ErrorCode::InvalidRequest)),
            };
            let exists: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE username=? AND id<>?")
                    .bind(&user.username)
                    .bind(id(u)?)
                    .fetch_one(&mut *c)
                    .await?;
            require(
                exists == 0 && !crate::account::reserved(c, &user.username).await?,
                ErrorCode::UsernameTaken,
            )?;
            user.display_name = patch_text(user.display_name, p.display_name, 64, true)?;
            user.status_text = patch_text(user.status_text, p.status_text, 128, true)?;
            user.avatar_id = match p.avatar_id {
                Patch::Keep => user.avatar_id,
                Patch::Clear => None,
                Patch::Set(a) => {
                    attach(c, u, a, UploadPurpose::Avatar, None).await?;
                    Some(a)
                }
            };
            sqlx::query(
                "UPDATE users SET username=?,display_name=?,status_text=?,avatar_id=? WHERE id=?",
            )
            .bind(&user.username)
            .bind(&user.display_name)
            .bind(&user.status_text)
            .bind(opt(user.avatar_id)?)
            .bind(id(u)?)
            .execute(&mut *c)
            .await?;
            let new = SelfUser {
                user: user.clone(),
                desired_presence: old.desired_presence,
            };
            if new != old {
                event(out, vec![u], Event::SelfUpdated(new.clone()));
                let mut viewers = vec![];
                for viewer in core.streams.values().map(|s| s.user) {
                    if profile_access(c, viewer, u).await.is_ok() {
                        viewers.push(viewer);
                    }
                }
                viewers.sort();
                viewers.dedup();
                event(out, viewers, Event::UserUpdated(user));
                for g in sqlx::query_scalar::<_, i64>(
                    "SELECT guild_id FROM guild_members WHERE user_id=?",
                )
                .bind(id(u)?)
                .fetch_all(&mut *c)
                .await?
                {
                    member_event(c, out, Id(g as u64), u).await?;
                }
            }
            Ok(CommandResult::Profile(new))
        }
        SetPresence { status } => {
            require(status != PresenceStatus::Offline, ErrorCode::InvalidRequest)?;
            let old = me(c, u).await?;
            if old.desired_presence != status {
                sqlx::query("UPDATE users SET desired_presence=?,presence_until=NULL WHERE id=?")
                    .bind(enum_str(status))
                    .bind(id(u)?)
                    .execute(&mut *c)
                    .await?;
                event(out, vec![u], Event::SelfUpdated(me(c, u).await?));
            }
            Ok(CommandResult::Presence(Presence { user_id: u, status }))
        }
        SendFriendRequest { username: s } => {
            let s = username(&s)?;
            let v = sqlx::query_scalar::<_, i64>("SELECT id FROM users WHERE username=?")
                .bind(s)
                .fetch_optional(&mut *c)
                .await?
                .map(|v| Id(v as u64))
                .ok_or_else(|| Error::new(ErrorCode::NotFound))?;
            require(v != u, ErrorCode::InvalidRequest)?;
            require(!blocked(c, u, v).await?, ErrorCode::Forbidden)?;
            if let Some(r) = relationship(c, u, v).await? {
                return Err(Error::new(if r.kind == RelationshipKind::IncomingRequest {
                    ErrorCode::Conflict
                } else {
                    ErrorCode::AlreadyExists
                }));
            }
            relationship_capacity(c, u, v).await?;
            relationship_capacity(c, v, u).await?;
            put_relationship(c, u, v, RelationshipKind::OutgoingRequest).await?;
            put_relationship(c, v, u, RelationshipKind::IncomingRequest).await?;
            relationship_event(c, out, u, v).await?;
            relationship_event(c, out, v, u).await?;
            Ok(CommandResult::Relationship(
                relationship(c, u, v)
                    .await?
                    .ok_or_else(|| Error::new(ErrorCode::Internal))?,
            ))
        }
        AcceptFriendRequest { user_id: v } => {
            require(v != u, ErrorCode::InvalidRequest)?;
            require(!blocked(c, u, v).await?, ErrorCode::Forbidden)?;
            require(
                relationship(c, u, v)
                    .await?
                    .is_some_and(|r| r.kind == RelationshipKind::IncomingRequest),
                ErrorCode::NotFound,
            )?;
            put_relationship(c, u, v, RelationshipKind::Friend).await?;
            put_relationship(c, v, u, RelationshipKind::Friend).await?;
            relationship_event(c, out, u, v).await?;
            relationship_event(c, out, v, u).await?;
            Ok(CommandResult::Relationship(
                relationship(c, u, v)
                    .await?
                    .ok_or_else(|| Error::new(ErrorCode::Internal))?,
            ))
        }
        DeclineFriendRequest { user_id: v }
        | CancelFriendRequest { user_id: v }
        | RemoveFriend { user_id: v } => {
            require(v != u, ErrorCode::InvalidRequest)?;
            let expected = match cmd {
                DeclineFriendRequest { .. } => RelationshipKind::IncomingRequest,
                CancelFriendRequest { .. } => RelationshipKind::OutgoingRequest,
                _ => RelationshipKind::Friend,
            };
            if let Some(r) = relationship(c, u, v).await? {
                require(r.kind == expected, ErrorCode::Conflict)?;
                sqlx::query("DELETE FROM relationships WHERE (user_id=? AND other_id=?) OR (user_id=? AND other_id=?)").bind(id(u)?).bind(id(v)?).bind(id(v)?).bind(id(u)?).execute(&mut *c).await?;
                relationship_event(c, out, u, v).await?;
                relationship_event(c, out, v, u).await?;
            }
            Ok(CommandResult::Ack)
        }
        BlockUser { user_id: v } => {
            require(v != u, ErrorCode::InvalidRequest)?;
            user(c, v).await?;
            if relationship(c, u, v)
                .await?
                .is_some_and(|r| r.kind == RelationshipKind::Blocked)
            {
                return Ok(CommandResult::Relationship(
                    relationship(c, u, v)
                        .await?
                        .ok_or_else(|| Error::new(ErrorCode::Internal))?,
                ));
            }
            relationship_capacity(c, u, v).await?;
            let peer = relationship(c, v, u)
                .await?
                .is_some_and(|r| r.kind != RelationshipKind::Blocked);
            sqlx::query("DELETE FROM relationships WHERE kind<>'blocked' AND ((user_id=? AND other_id=?) OR (user_id=? AND other_id=?))").bind(id(u)?).bind(id(v)?).bind(id(v)?).bind(id(u)?).execute(&mut *c).await?;
            put_relationship(c, u, v, RelationshipKind::Blocked).await?;
            relationship_event(c, out, u, v).await?;
            if peer {
                relationship_event(c, out, v, u).await?;
            }
            Ok(CommandResult::Relationship(
                relationship(c, u, v)
                    .await?
                    .ok_or_else(|| Error::new(ErrorCode::Internal))?,
            ))
        }
        UnblockUser { user_id: v } => {
            require(v != u, ErrorCode::InvalidRequest)?;
            if relationship(c, u, v)
                .await?
                .is_some_and(|r| r.kind == RelationshipKind::Blocked)
            {
                sqlx::query(
                    "DELETE FROM relationships WHERE user_id=? AND other_id=? AND kind='blocked'",
                )
                .bind(id(u)?)
                .bind(id(v)?)
                .execute(&mut *c)
                .await?;
                relationship_event(c, out, u, v).await?;
            }
            Ok(CommandResult::Ack)
        }
        OpenDm { user_id: v } => {
            require(v != u, ErrorCode::InvalidRequest)?;
            user(c, v).await?;
            require(!blocked(c, u, v).await?, ErrorCode::Blocked)?;
            let low = u.min(v);
            let high = u.max(v);
            let existing: Option<i64> = sqlx::query_scalar(
                "SELECT channel_id FROM dm_pairs WHERE low_user_id=? AND high_user_id=?",
            )
            .bind(id(low)?)
            .bind(id(high)?)
            .fetch_optional(&mut *c)
            .await?;
            // A first contact from a non-friend may arrive as a message request.
            let mut request = false;
            let ch = if let Some(ch) = existing {
                crate::privacy::requests::opened_by(c, Id(ch as u64), u, out).await?;
                Id(ch as u64)
            } else {
                request = crate::privacy::requests::opening(c, u, v).await?;
                let ch = core.id().await?;
                sqlx::query(
                    "INSERT INTO channels(id,kind,position,created_at) VALUES(?,'direct',0,?)",
                )
                .bind(id(ch)?)
                .bind(now() as i64)
                .execute(&mut *c)
                .await?;
                sqlx::query("INSERT INTO dm_pairs VALUES(?,?,?)")
                    .bind(id(low)?)
                    .bind(id(high)?)
                    .bind(id(ch)?)
                    .execute(&mut *c)
                    .await?;
                for user in [u, v] {
                    sqlx::query("INSERT INTO dm_members VALUES(?,?,?,0)")
                        .bind(id(ch)?)
                        .bind(id(user)?)
                        .bind(now() as i64)
                        .execute(&mut *c)
                        .await?;
                    init_read(c, user, ch, false).await?;
                }
                if request {
                    crate::privacy::requests::record(c, ch, u, v, out).await?;
                }
                ch
            };
            let opened = if existing.is_none() && !request {
                vec![u, v]
            } else {
                vec![u]
            };
            for user in &opened {
                dm_quota(c, *user, Some(ch)).await?;
            }
            let changed: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM dm_members WHERE channel_id=? AND visible=0 AND (? OR user_id=?)",
            )
            .bind(id(ch)?)
            .bind(existing.is_none() && !request).bind(id(u)?)
            .fetch_one(&mut *c)
            .await?;
            sqlx::query("UPDATE dm_members SET visible=1 WHERE channel_id=? AND (? OR user_id=?)")
                .bind(id(ch)?)
                .bind(existing.is_none() && !request)
                .bind(id(u)?)
                .execute(&mut *c)
                .await?;
            if existing.is_none() || changed > 0 {
                for user_id in [u, v] {
                    event(
                        out,
                        opened.clone(),
                        Event::UserUpdated(user(c, user_id).await?),
                    );
                }
                channel_event(c, out, ch).await?;
                reads_event(c, out, ch).await?;
            }
            Ok(CommandResult::Channel(channel(c, ch).await?))
        }
        CreateGroupDm {
            name: s,
            user_ids: mut users,
        } => {
            // A group without a name is shown by the names of its members.
            let s = if s.trim().is_empty() {
                None
            } else {
                Some(name(&s, 64)?)
            };
            unique(&users, 15)?;
            require(
                users.len() >= 2 && !users.contains(&u),
                ErrorCode::InvalidRequest,
            )?;
            for v in &users {
                friends(c, u, *v).await?;
            }
            users.push(u);
            users.sort();
            for a in &users {
                dm_quota(c, *a, None).await?;
                for b in &users {
                    if a != b {
                        require(!blocked(c, *a, *b).await?, ErrorCode::Blocked)?;
                    }
                }
            }
            let ch = core.id().await?;
            sqlx::query("INSERT INTO channels(id,kind,name,position,owner_id,created_at) VALUES(?,'group_dm',?,0,?,?)").bind(id(ch)?).bind(s).bind(id(u)?).bind(now() as i64).execute(&mut *c).await?;
            for v in users {
                sqlx::query("INSERT INTO dm_members VALUES(?,?,?,1)")
                    .bind(id(ch)?)
                    .bind(id(v)?)
                    .bind(now() as i64)
                    .execute(&mut *c)
                    .await?;
                init_read(c, v, ch, true).await?;
            }
            let ch = channel_event(c, out, ch).await?;
            reads_event(c, out, ch.summary.id).await?;
            Ok(CommandResult::Channel(ch))
        }
        AddGroupRecipient {
            channel_id: ch,
            user_id: v,
        }
        | RemoveGroupRecipient {
            channel_id: ch,
            user_id: v,
        }
        | TransferGroupOwnership {
            channel_id: ch,
            user_id: v,
        } => {
            id(v)?;
            let channel = access(c, u, ch, Permissions::VIEW_CHANNEL).await?;
            require(
                channel.summary.kind == ChannelKind::GroupDm,
                ErrorCode::InvalidRequest,
            )?;
            require(
                channel.summary.owner_id == Some(u),
                ErrorCode::OwnerRequired,
            )?;
            match cmd {
                AddGroupRecipient { .. } => {
                    if !channel.summary.recipient_ids.contains(&v) {
                        require(
                            channel.summary.recipient_ids.len() < 16,
                            ErrorCode::LimitExceeded,
                        )?;
                        friends(c, u, v).await?;
                        dm_quota(c, v, None).await?;
                        for a in &channel.summary.recipient_ids {
                            require(!blocked(c, *a, v).await?, ErrorCode::Blocked)?;
                        }
                        sqlx::query("INSERT INTO dm_members VALUES(?,?,?,1)")
                            .bind(id(ch)?)
                            .bind(id(v)?)
                            .bind(now() as i64)
                            .execute(&mut *c)
                            .await?;
                        init_read(c, v, ch, true).await?;
                        channel_event(c, out, ch).await?;
                        event(out, vec![v], Event::ReadStateUpdated(read(c, v, ch).await?));
                    }
                }
                RemoveGroupRecipient { .. } => {
                    require(v != u, ErrorCode::InvalidRequest)?;
                    if channel.summary.recipient_ids.contains(&v) {
                        sqlx::query("DELETE FROM dm_members WHERE channel_id=? AND user_id=?")
                            .bind(id(ch)?)
                            .bind(id(v)?)
                            .execute(&mut *c)
                            .await?;
                        sqlx::query("DELETE FROM read_states WHERE channel_id=? AND user_id=?")
                            .bind(id(ch)?)
                            .bind(id(v)?)
                            .execute(&mut *c)
                            .await?;
                        event(
                            out,
                            vec![v],
                            Event::ChannelRemoved {
                                channel_id: ch,
                                reason: RemovalReason::Kicked,
                            },
                        );
                        channel_event(c, out, ch).await?;
                    }
                }
                _ => {
                    require(
                        channel.summary.recipient_ids.contains(&v),
                        ErrorCode::InvalidRequest,
                    )?;
                    if v != u {
                        sqlx::query("UPDATE channels SET owner_id=? WHERE id=?")
                            .bind(id(v)?)
                            .bind(id(ch)?)
                            .execute(&mut *c)
                            .await?;
                        channel_event(c, out, ch).await?;
                    }
                }
            }
            Ok(CommandResult::Channel(crate::db::channel(c, ch).await?))
        }
        LeaveGroupDm { channel_id: ch } => {
            let channel = access(c, u, ch, Permissions::VIEW_CHANNEL).await?;
            require(
                channel.summary.kind == ChannelKind::GroupDm,
                ErrorCode::InvalidRequest,
            )?;
            require(
                channel.summary.owner_id != Some(u) || channel.summary.recipient_ids.len() == 1,
                ErrorCode::OwnerRequired,
            )?;
            sqlx::query("DELETE FROM dm_members WHERE channel_id=? AND user_id=?")
                .bind(id(ch)?)
                .bind(id(u)?)
                .execute(&mut *c)
                .await?;
            sqlx::query("DELETE FROM read_states WHERE channel_id=? AND user_id=?")
                .bind(id(ch)?)
                .bind(id(u)?)
                .execute(&mut *c)
                .await?;
            event(
                out,
                vec![u],
                Event::ChannelRemoved {
                    channel_id: ch,
                    reason: RemovalReason::Left,
                },
            );
            if channel.summary.recipient_ids.len() == 1 {
                sqlx::query("DELETE FROM channels WHERE id=?")
                    .bind(id(ch)?)
                    .execute(&mut *c)
                    .await?;
            } else {
                channel_event(c, out, ch).await?;
            }
            Ok(CommandResult::Ack)
        }
        CloseDm { channel_id: ch } => {
            let channel = access(c, u, ch, Permissions::VIEW_CHANNEL).await?;
            require(
                channel.summary.kind == ChannelKind::Direct,
                ErrorCode::InvalidRequest,
            )?;
            let r = sqlx::query(
                "UPDATE dm_members SET visible=0 WHERE channel_id=? AND user_id=? AND visible=1",
            )
            .bind(id(ch)?)
            .bind(id(u)?)
            .execute(&mut *c)
            .await?;
            if r.rows_affected() > 0 {
                event(
                    out,
                    vec![u],
                    Event::ChannelRemoved {
                        channel_id: ch,
                        reason: RemovalReason::Left,
                    },
                );
            }
            Ok(CommandResult::Ack)
        }
        CreateGuild { name: s, icon_id } => {
            let s = name(&s, 64)?;
            guild_quota(c, u).await?;
            if let Some(a) = icon_id {
                attach(c, u, a, UploadPurpose::GuildIcon, None).await?;
            }
            let g = core.id().await?;
            let ch = core.id().await?;
            sqlx::query("INSERT INTO guilds VALUES(?,?,?,?,?)")
                .bind(id(g)?)
                .bind(id(u)?)
                .bind(s)
                .bind(opt(icon_id)?)
                .bind(now() as i64)
                .execute(&mut *c)
                .await?;
            sqlx::query("INSERT INTO guild_members VALUES(?,?,'owner',?)")
                .bind(id(g)?)
                .bind(id(u)?)
                .bind(now() as i64)
                .execute(&mut *c)
                .await?;
            sqlx::query("INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(?,'text',?,'general',0,?)").bind(id(ch)?).bind(id(g)?).bind(now() as i64).execute(&mut *c).await?;
            // A new server is ready to talk in: one text and one voice channel.
            let voice = core.id().await?;
            sqlx::query("INSERT INTO channels(id,kind,guild_id,name,position,created_at) VALUES(?,'voice',?,'Voice',1,?)").bind(id(voice)?).bind(id(g)?).bind(now() as i64).execute(&mut *c).await?;
            joined(c, out, g, u).await?;
            Ok(CommandResult::Guild(detail(c, u, g).await?))
        }
        UpdateGuild {
            guild_id: g,
            patch: p,
        } => {
            guild_perm(c, u, g, Permissions::MANAGE_GUILD).await?;
            let old = guild(c, u, g).await?.guild;
            let s = patch_text(Some(old.name.clone()), p.name, 64, false)?
                .ok_or_else(|| Error::new(ErrorCode::Internal))?;
            let a = match p.icon_id {
                Patch::Keep => old.icon_id,
                Patch::Clear => None,
                Patch::Set(a) => {
                    attach(c, u, a, UploadPurpose::GuildIcon, None).await?;
                    Some(a)
                }
            };
            if old.name != s || old.icon_id != a {
                sqlx::query("UPDATE guilds SET name=?,icon_id=? WHERE id=?")
                    .bind(s)
                    .bind(opt(a)?)
                    .bind(id(g)?)
                    .execute(&mut *c)
                    .await?;
                guild_events(c, out, g).await?;
            }
            Ok(CommandResult::Guild(detail(c, u, g).await?))
        }
        DeleteGuild { guild_id: g } => {
            guild_perm(c, u, g, Permissions::OWNER).await?;
            let users = guild_users(c, g).await?;
            sqlx::query("DELETE FROM guilds WHERE id=?")
                .bind(id(g)?)
                .execute(&mut *c)
                .await?;
            event(
                out,
                users,
                Event::GuildRemoved {
                    guild_id: g,
                    reason: RemovalReason::Deleted,
                },
            );
            Ok(CommandResult::Ack)
        }
        JoinGuild { invite_code: code } => {
            // Invite codes and vanity codes (identity.rs).
            let iid = crate::identity::find_invite(c, &code).await?;
            let i = invite(
                c,
                Id(iid.ok_or_else(|| Error::new(ErrorCode::InvalidInvite))? as u64),
            )
            .await?;
            require(i.expires_at > now(), ErrorCode::InviteExpired)?;
            require(i.uses < i.max_uses, ErrorCode::InviteExhausted)?;
            let g = i.guild_id;
            let banned: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM guild_bans WHERE guild_id=? AND user_id=?",
            )
            .bind(id(g)?)
            .bind(id(u)?)
            .fetch_one(&mut *c)
            .await?;
            require(banned == 0, ErrorCode::Banned)?;
            if role(c, u, g).await.is_ok() {
                return Ok(CommandResult::Guild(detail(c, u, g).await?));
            }
            crate::community::gate(c, g).await?;
            guild_quota(c, u).await?;
            require(
                guild_users(c, g).await?.len() < 1000,
                ErrorCode::LimitExceeded,
            )?;
            sqlx::query("INSERT INTO guild_members VALUES(?,?,'member',?)")
                .bind(id(g)?)
                .bind(id(u)?)
                .bind(now() as i64)
                .execute(&mut *c)
                .await?;
            sqlx::query("UPDATE invites SET uses=uses+1 WHERE id=?")
                .bind(id(i.id)?)
                .execute(&mut *c)
                .await?;
            crate::community::admit(c, g, u, JoinSource::Invite, Some(code.as_str()), false)
                .await?;
            joined(c, out, g, u).await?;
            let i = invite(c, i.id).await?;
            if i.uses == i.max_uses {
                sqlx::query("UPDATE invites SET inactive_notified=1 WHERE id=?")
                    .bind(id(i.id)?)
                    .execute(&mut *c)
                    .await?;
                event(
                    out,
                    invite_users(c, &i).await?,
                    Event::InviteRemoved {
                        guild_id: g,
                        invite_id: i.id,
                    },
                );
            } else {
                event(out, invite_users(c, &i).await?, Event::InviteUpsert(i));
            }
            Ok(CommandResult::Guild(detail(c, u, g).await?))
        }
        LeaveGuild { guild_id: g } => {
            let r = role(c, u, g).await?;
            require(r != MemberRole::Owner, ErrorCode::OwnerRequired)?;
            remove_member(c, out, g, u, RemovalReason::Left).await?;
            Ok(CommandResult::Ack)
        }
        TransferGuildOwnership {
            guild_id: g,
            user_id: v,
        } => {
            guild_perm(c, u, g, Permissions::OWNER).await?;
            role(c, v, g).await?;
            if v != u {
                crate::launch::ensure_legacy_admin(c, g).await?;
                event(
                    out,
                    guild_users(c, g).await?,
                    Event::RolesUpdated {
                        guild_id: g,
                        roles: crate::permissions::roles(c, g).await?,
                    },
                );
                sqlx::query("UPDATE guild_members SET role='admin' WHERE guild_id=? AND user_id=?")
                    .bind(id(g)?)
                    .bind(id(u)?)
                    .execute(&mut *c)
                    .await?;
                sqlx::query("UPDATE guild_members SET role='owner' WHERE guild_id=? AND user_id=?")
                    .bind(id(g)?)
                    .bind(id(v)?)
                    .execute(&mut *c)
                    .await?;
                sqlx::query("UPDATE guilds SET owner_id=? WHERE id=?")
                    .bind(id(v)?)
                    .bind(id(g)?)
                    .execute(&mut *c)
                    .await?;
                guild_events(c, out, g).await?;
                member_event(c, out, g, u).await?;
                member_event(c, out, g, v).await?;
            }
            Ok(CommandResult::Guild(detail(c, u, g).await?))
        }
        SetMemberRole {
            guild_id: g,
            user_id: v,
            role: r,
        } => {
            guild_perm(c, u, g, Permissions::OWNER).await?;
            require(r != MemberRole::Owner, ErrorCode::InvalidRequest)?;
            let old = role(c, v, g).await?;
            require(old != MemberRole::Owner, ErrorCode::HierarchyViolation)?;
            if old != r {
                if r == MemberRole::Admin {
                    crate::launch::ensure_legacy_admin(c, g).await?;
                    event(
                        out,
                        guild_users(c, g).await?,
                        Event::RolesUpdated {
                            guild_id: g,
                            roles: crate::permissions::roles(c, g).await?,
                        },
                    );
                }
                sqlx::query("UPDATE guild_members SET role=? WHERE guild_id=? AND user_id=?")
                    .bind(enum_str(r))
                    .bind(id(g)?)
                    .bind(id(v)?)
                    .execute(&mut *c)
                    .await?;
                guild_events(c, out, g).await?;
                member_event(c, out, g, v).await?;
            }
            Ok(CommandResult::Member(member(c, g, v).await?))
        }
        KickMember {
            guild_id: g,
            user_id: v,
        }
        | BanMember {
            guild_id: g,
            user_id: v,
            ..
        } => {
            let is_ban = matches!(cmd, BanMember { .. });
            guild_perm(
                c,
                u,
                g,
                if is_ban {
                    Permissions::BAN_MEMBERS
                } else {
                    Permissions::KICK_MEMBERS
                },
            )
            .await?;
            require(v != u, ErrorCode::HierarchyViolation)?;
            user(c, v).await?;
            let target = role(c, v, g).await.ok();
            if target.is_some() {
                crate::permissions::above_member(c, g, u, v).await?;
            } else {
                require(is_ban, ErrorCode::NotFound)?;
            }
            if is_ban {
                let reason = if let BanMember { reason, .. } = cmd {
                    optional(reason, 256)?
                } else {
                    None
                };
                let res = sqlx::query("INSERT OR IGNORE INTO guild_bans VALUES(?,?,?,?,?)")
                    .bind(id(g)?)
                    .bind(id(v)?)
                    .bind(id(u)?)
                    .bind(reason)
                    .bind(now() as i64)
                    .execute(&mut *c)
                    .await?;
                if res.rows_affected() > 0 {
                    event(
                        out,
                        admins(c, g).await?,
                        Event::BanUpsert(ban(c, g, v).await?),
                    );
                }
            }
            if target.is_some() {
                remove_member(
                    c,
                    out,
                    g,
                    v,
                    if is_ban {
                        RemovalReason::Banned
                    } else {
                        RemovalReason::Kicked
                    },
                )
                .await?;
            }
            Ok(CommandResult::Ack)
        }
        UnbanMember {
            guild_id: g,
            user_id: v,
        } => {
            guild_perm(c, u, g, Permissions::BAN_MEMBERS).await?;
            let res = sqlx::query("DELETE FROM guild_bans WHERE guild_id=? AND user_id=?")
                .bind(id(g)?)
                .bind(id(v)?)
                .execute(&mut *c)
                .await?;
            if res.rows_affected() > 0 {
                event(
                    out,
                    admins(c, g).await?,
                    Event::BanRemoved {
                        guild_id: g,
                        user_id: v,
                    },
                );
            }
            Ok(CommandResult::Ack)
        }
        CreateInvite {
            guild_id: g,
            ttl_seconds,
            max_uses,
        } => {
            guild_perm(c, u, g, Permissions::CREATE_INVITES).await?;
            require(
                (60..=604800).contains(&ttl_seconds) && (1..=1000).contains(&max_uses),
                ErrorCode::InvalidRequest,
            )?;
            let r=sqlx::query("SELECT COUNT(*) AS n,SUM(creator_id=?) AS own FROM invites WHERE guild_id=? AND expires_at>? AND uses<max_uses").bind(id(u)?).bind(id(g)?).bind(now() as i64).fetch_one(&mut *c).await?;
            require(
                r.get::<i64, _>("n") < 100 && r.get::<Option<i64>, _>("own").unwrap_or(0) < 10,
                ErrorCode::LimitExceeded,
            )?;
            let iid = core.id().await?;
            let code = URL_SAFE_NO_PAD.encode(crate::state::random(16));
            sqlx::query("INSERT INTO invites(id,code_hash,code,guild_id,creator_id,expires_at,max_uses,uses) VALUES(?,?,?,?,?,?,?,0)")
                .bind(id(iid)?)
                .bind(hash(code.as_bytes()))
                .bind(code)
                .bind(id(g)?)
                .bind(id(u)?)
                .bind((now() + u64::from(ttl_seconds) * 1000) as i64)
                .bind(i64::from(max_uses))
                .execute(&mut *c)
                .await?;
            let i = invite(c, iid).await?;
            event(
                out,
                invite_users(c, &i).await?,
                Event::InviteUpsert(i.clone()),
            );
            Ok(CommandResult::Invite(i))
        }
        RevokeInvite { guild_id: g, code } => {
            role(c, u, g).await?;
            let iid: Option<i64> =
                sqlx::query_scalar("SELECT id FROM invites WHERE guild_id=? AND code_hash=?")
                    .bind(id(g)?)
                    .bind(hash(code.as_bytes()))
                    .fetch_optional(&mut *c)
                    .await?;
            if let Some(iid) = iid {
                crate::identity::not_vanity(c, iid).await?;
                let i = invite(c, Id(iid as u64)).await?;
                require(
                    i.creator_id == u
                        || crate::permissions::evaluate(c, u, i.guild_id, None)
                            .await?
                            .contains(Permissions::MANAGE_GUILD),
                    ErrorCode::Forbidden,
                )?;
                let users = invite_users(c, &i).await?;
                sqlx::query("DELETE FROM invites WHERE id=?")
                    .bind(iid)
                    .execute(&mut *c)
                    .await?;
                event(
                    out,
                    users,
                    Event::InviteRemoved {
                        guild_id: g,
                        invite_id: i.id,
                    },
                );
            }
            Ok(CommandResult::Ack)
        }
        CreateChannel {
            guild_id: g,
            kind,
            name: s,
            parent_id: p,
            topic,
        } => {
            guild_perm(c, u, g, Permissions::MANAGE_CHANNELS).await?;
            require(
                matches!(
                    kind,
                    ChannelKind::Category
                        | ChannelKind::Text
                        | ChannelKind::Voice
                        | ChannelKind::Announcement
                        | ChannelKind::Forum
                ),
                ErrorCode::InvalidRequest,
            )?;
            require(
                kind != ChannelKind::Category || topic.is_none(),
                ErrorCode::InvalidRequest,
            )?;
            let s = name(&s, 64)?;
            require(!s.contains(['/', '\\']), ErrorCode::InvalidRequest)?;
            let topic = optional(topic, 512)?;
            parent(c, g, p).await?;
            if let Some(p) = p {
                access(c, u, p, Permissions::MANAGE_CHANNELS).await?;
            }
            require(
                sqlx::query_scalar::<_, i64>(
                    "SELECT COUNT(*) FROM channels WHERE guild_id=? AND kind<>'thread'",
                )
                .bind(id(g)?)
                .fetch_one(&mut *c)
                .await?
                    < MAX_CHANNELS_PER_GUILD as i64,
                ErrorCode::LimitExceeded,
            )?;
            let pos = append_position(c, g, p).await?;
            let ch = core.id().await?;
            sqlx::query("INSERT INTO channels(id,kind,guild_id,parent_id,name,topic,position,created_at) VALUES(?,?,?,?,?,?,?,?)").bind(id(ch)?).bind(enum_str(kind)).bind(id(g)?).bind(opt(p)?).bind(s).bind(topic).bind(pos).bind(now() as i64).execute(&mut *c).await?;
            crate::threads::channel_created(c, ch, kind).await?;
            let messages = matches!(
                kind,
                ChannelKind::Text | ChannelKind::Voice | ChannelKind::Announcement
            );
            if messages {
                for v in guild_users(c, g).await? {
                    init_read(c, v, ch, true).await?;
                }
            }
            let ch = channel_event(c, out, ch).await?;
            if messages {
                reads_event(c, out, ch.summary.id).await?;
            }
            Ok(CommandResult::Channel(ch))
        }
        SetVoicePermission {
            channel_id,
            user_id,
            connect,
        } => {
            let ch = access(c, u, channel_id, Permissions::MANAGE_ROLES).await?;
            require(
                ch.summary.kind == ChannelKind::Voice,
                ErrorCode::InvalidRequest,
            )?;
            let g = ch
                .summary
                .guild_id
                .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
            crate::permissions::above_member(c, g, u, user_id).await?;
            role(c, user_id, g).await?;
            let p = crate::permissions::evaluate(c, u, g, Some(channel_id)).await?;
            require(p.contains(Permissions::CONNECT), ErrorCode::Forbidden)?;
            // Legacy boolean command edits only Connect in the canonical member overwrite.
            sqlx::query("INSERT INTO permission_overwrites VALUES(?,'member',?,0,?) ON CONFLICT(channel_id,kind,target_id) DO UPDATE SET allow=allow & ~?,deny=(deny & ~?) | excluded.deny")
                .bind(id(channel_id)?).bind(id(user_id)?).bind(if connect {0} else {Permissions::CONNECT.bits() as i64}).bind(Permissions::CONNECT.bits() as i64).bind(Permissions::CONNECT.bits() as i64).execute(&mut *c).await?;
            Ok(CommandResult::Ack)
        }
        UpdateChannel {
            channel_id: ch,
            patch: p,
        } => {
            let old = access(c, u, ch, Permissions::VIEW_CHANNEL).await?;
            crate::threads::not_thread(&old)?;
            if old.summary.guild_id.is_some() {
                access(c, u, ch, Permissions::MANAGE_CHANNELS).await?;
            } else {
                require(
                    old.summary.kind == ChannelKind::GroupDm,
                    ErrorCode::InvalidRequest,
                )?;
                require(old.summary.owner_id == Some(u), ErrorCode::OwnerRequired)?;
                require(matches!(p.topic, Patch::Keep), ErrorCode::InvalidRequest)?;
            }
            require(
                old.summary.kind != ChannelKind::Category || !matches!(p.topic, Patch::Set(_)),
                ErrorCode::InvalidRequest,
            )?;
            let s = patch_text(old.summary.name.clone(), p.name, 64, false)?;
            if old.summary.guild_id.is_some() {
                require(
                    !s.as_deref().unwrap_or_default().contains(['/', '\\']),
                    ErrorCode::InvalidRequest,
                )?;
            }
            let topic = patch_text(old.topic.clone(), p.topic, 512, true)?;
            if s != old.summary.name || topic != old.topic {
                sqlx::query("UPDATE channels SET name=?,topic=? WHERE id=?")
                    .bind(s)
                    .bind(topic)
                    .bind(id(ch)?)
                    .execute(&mut *c)
                    .await?;
                channel_event(c, out, ch).await?;
            }
            Ok(CommandResult::Channel(channel(c, ch).await?))
        }
        ReorderChannels {
            guild_id: g,
            placements,
        } => {
            guild_perm(c, u, g, Permissions::MANAGE_CHANNELS).await?;
            let list = channels(c, u, Some(g)).await?;
            require(placements.len() == list.len(), ErrorCode::InvalidRequest)?;
            let ids: Vec<_> = placements.iter().map(|p| p.channel_id).collect();
            unique(&ids, MAX_CHANNELS_PER_GUILD)?;
            // Only placements that differ from what the actor sees are applied, so a
            // channel shown to them outside a hidden parent stays where it really is.
            let mut moved = vec![];
            let mut siblings: HashMap<Option<Id>, (bool, Vec<u16>)> = HashMap::new();
            for p in &placements {
                access(c, u, p.channel_id, Permissions::MANAGE_CHANNELS).await?;
                if let Some(parent) = p.parent_id {
                    access(c, u, parent, Permissions::MANAGE_CHANNELS).await?;
                }
                let ch = list
                    .iter()
                    .find(|ch| ch.id == p.channel_id)
                    .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
                parent(c, g, p.parent_id).await?;
                let set = siblings.entry(p.parent_id).or_default();
                set.1.push(p.position);
                if ch.position != p.position || ch.parent_id != p.parent_id {
                    set.0 = true;
                    moved.push(p);
                }
            }
            for (_, positions) in siblings.values_mut().filter(|(changed, _)| *changed) {
                positions.sort();
                require(
                    positions
                        .iter()
                        .enumerate()
                        .all(|(i, p)| i == usize::from(*p)),
                    ErrorCode::InvalidRequest,
                )?;
            }
            if !moved.is_empty() {
                // No channel may end up inside itself, however deep, including
                // through channels the actor cannot see.
                let mut inside: HashMap<Id, Option<Id>> =
                    sqlx::query("SELECT id,parent_id FROM channels WHERE guild_id=?")
                        .bind(id(g)?)
                        .fetch_all(&mut *c)
                        .await?
                        .into_iter()
                        .map(|r| {
                            (
                                Id(r.get::<i64, _>(0) as u64),
                                r.get::<Option<i64>, _>(1).map(|v| Id(v as u64)),
                            )
                        })
                        .collect();
                for p in &moved {
                    inside.insert(p.channel_id, p.parent_id);
                }
                for p in &moved {
                    let mut at = p.parent_id;
                    for _ in 0..inside.len() {
                        require(at != Some(p.channel_id), ErrorCode::InvalidRequest)?;
                        at = at.and_then(|v| inside.get(&v).copied().flatten());
                    }
                }
                for p in &moved {
                    sqlx::query("UPDATE channels SET parent_id=?,position=? WHERE id=?")
                        .bind(opt(p.parent_id)?)
                        .bind(i64::from(p.position))
                        .bind(id(p.channel_id)?)
                        .execute(&mut *c)
                        .await?;
                }
                event(
                    out,
                    guild_users(c, g).await?,
                    Event::ChannelsReordered {
                        guild_id: g,
                        placements,
                    },
                );
            }
            Ok(CommandResult::Ack)
        }
        DeleteChannel { channel_id: ch } => {
            let old = access(c, u, ch, Permissions::MANAGE_CHANNELS).await?;
            crate::threads::not_thread(&old)?;
            let g = old
                .summary
                .guild_id
                .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
            crate::threads::parent_deleted(c, out, ch).await?;
            // Channels nested in the deleted one are kept: they move up one level, to
            // the end of the channel it was nested in (the top when it was not nested).
            let children: Vec<i64> = sqlx::query_scalar(
                "SELECT id FROM channels WHERE parent_id=? ORDER BY position,id",
            )
            .bind(id(ch)?)
            .fetch_all(&mut *c)
            .await?;
            if !children.is_empty() {
                // `old` carries the parent as the actor sees it; use the real one.
                let up = sqlx::query_scalar::<_, Option<i64>>(
                    "SELECT parent_id FROM channels WHERE id=?",
                )
                .bind(id(ch)?)
                .fetch_one(&mut *c)
                .await?
                .map(|v| Id(v as u64));
                let mut position = append_position(c, g, up).await?;
                for child in children {
                    sqlx::query("UPDATE channels SET parent_id=?,position=? WHERE id=?")
                        .bind(opt(up)?)
                        .bind(position)
                        .bind(child)
                        .execute(&mut *c)
                        .await?;
                    position += 1;
                    channel_event(c, out, Id(child as u64)).await?;
                }
            }
            let previous_audience = audience(c, ch).await?;
            sqlx::query("DELETE FROM channels WHERE id=?")
                .bind(id(ch)?)
                .execute(&mut *c)
                .await?;
            event(
                out,
                previous_audience,
                Event::ChannelRemoved {
                    channel_id: ch,
                    reason: RemovalReason::Deleted,
                },
            );
            compact(c, out, g).await?;
            Ok(CommandResult::Ack)
        }
        SendMessage(mut d) => {
            unique(&d.attachment_ids, MAX_MESSAGE_ATTACHMENTS)?;
            crate::threads::before_send(c, u, &d, out).await?;
            let channel = validate_message(
                c,
                u,
                d.channel_id,
                &d.content,
                &d.mention_ids,
                d.attachment_ids.len(),
            )
            .await?;
            d.content = crate::expression::check_content(c, u, &channel, &d.content).await?;
            crate::slowmode::check(c, u, d.channel_id).await?;
            let ch = d.channel_id;
            let recipients = audience(c, ch).await?;
            if channel.summary.kind == ChannelKind::Direct {
                let hidden = crate::privacy::requests::sending(c, ch, u, &d.content, out).await?;
                let mut changed = vec![];
                for v in recipients.iter().filter(|v| !hidden.contains(v)) {
                    dm_quota(c, *v, Some(ch)).await?;
                    let visible: i64 = sqlx::query_scalar(
                        "SELECT visible FROM dm_members WHERE channel_id=? AND user_id=?",
                    )
                    .bind(id(ch)?)
                    .bind(id(*v)?)
                    .fetch_one(&mut *c)
                    .await?;
                    if visible == 0 {
                        changed.push(*v);
                    }
                }
                sqlx::query("UPDATE dm_members SET visible=1 WHERE channel_id=? AND NOT EXISTS(SELECT 1 FROM message_requests r WHERE r.channel_id=dm_members.channel_id AND r.recipient_id=dm_members.user_id AND r.state IN('pending','spam','ignored'))")
                    .bind(id(ch)?)
                    .execute(&mut *c)
                    .await?;
                if !changed.is_empty() {
                    for v in &recipients {
                        event(out, changed.clone(), Event::UserUpdated(user(c, *v).await?));
                    }
                    event(out, changed, Event::ChannelUpsert(channel.clone()));
                }
            }
            if let Some(reply) = d.reply_to {
                command_message(c, u, ch, reply, false)
                    .await
                    .map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
            }
            let m = core.id().await?;
            sqlx::query("INSERT INTO messages(id,channel_id,author_id,content,created_at,reply_to) VALUES(?,?,?,?,?,?)").bind(id(m)?).bind(id(ch)?).bind(id(u)?).bind(&d.content).bind(now() as i64).bind(opt(d.reply_to)?).execute(&mut *c).await?;
            for (i, a) in d.attachment_ids.iter().enumerate() {
                attach(c, u, *a, UploadPurpose::Message, Some(ch)).await?;
                sqlx::query("INSERT INTO message_attachments VALUES(?,?,?)")
                    .bind(id(m)?)
                    .bind(id(*a)?)
                    .bind(i as i64)
                    .execute(&mut *c)
                    .await?;
            }
            let mentions =
                crate::mentions::resolve(c, core, u, &channel, &d.content, &d.mention_ids).await?;
            for v in &mentions.users {
                sqlx::query("INSERT INTO message_mentions VALUES(?,?)")
                    .bind(id(m)?)
                    .bind(id(*v)?)
                    .execute(&mut *c)
                    .await?;
            }
            crate::mentions::persist(c, m, &mentions).await?;
            let m = message(c, u, ch, m, false).await?;
            event(
                out,
                recipients,
                Event::MessageCreated {
                    message: m.clone(),
                    author: user(c, u).await?,
                },
            );
            crate::threads::after_send(c, u, &m, &d.mention_ids, out).await?;
            reads_event(c, out, ch).await?;
            Ok(CommandResult::Message(m))
        }
        EditMessage(mut e) => {
            let old = command_message(c, u, e.channel_id, e.message_id, false).await?;
            validate_message(
                c,
                u,
                e.channel_id,
                &e.content,
                &e.mention_ids,
                old.attachments.len(),
            )
            .await?;
            require(old.author_id == u, ErrorCode::Forbidden)?;
            require(old.call.is_none(), ErrorCode::Forbidden)?;
            // A crossposted copy changes only with its source.
            require(old.extra.crosspost().is_none(), ErrorCode::Forbidden)?;
            let channel = channel(c, e.channel_id).await?;
            e.content = crate::expression::check_content(c, u, &channel, &e.content).await?;
            let expanded =
                crate::mentions::resolve(c, core, u, &channel, &e.content, &e.mention_ids).await?;
            let mentions = expanded.users.clone();
            if old.content != e.content || old.mention_ids != mentions {
                sqlx::query("UPDATE messages SET content=?,edited_at=? WHERE id=?")
                    .bind(&e.content)
                    .bind(now() as i64)
                    .bind(id(e.message_id)?)
                    .execute(&mut *c)
                    .await?;
                sqlx::query("DELETE FROM message_mentions WHERE message_id=?")
                    .bind(id(e.message_id)?)
                    .execute(&mut *c)
                    .await?;
                for v in mentions {
                    sqlx::query("INSERT INTO message_mentions VALUES(?,?)")
                        .bind(id(e.message_id)?)
                        .bind(id(v)?)
                        .execute(&mut *c)
                        .await?;
                }
                crate::mentions::persist(c, e.message_id, &expanded).await?;
                crate::threads::source_edited(c, e.message_id).await?;
                for v in audience(c, e.channel_id).await? {
                    event(
                        out,
                        vec![v],
                        Event::MessageUpdated(
                            message(c, v, e.channel_id, e.message_id, false).await?,
                        ),
                    );
                }
                reads_event(c, out, e.channel_id).await?;
            }
            Ok(CommandResult::Message(
                message(c, u, e.channel_id, e.message_id, false).await?,
            ))
        }
        DeleteMessage {
            channel_id: ch,
            message_id: m,
        } => {
            let channel = text(c, u, ch, Permissions::VIEW_CHANNEL).await?;
            let old = command_message(c, u, ch, m, true).await?;
            if old.author_id != u {
                require(channel.summary.guild_id.is_some(), ErrorCode::Forbidden)?;
                access(c, u, ch, Permissions::MANAGE_MESSAGES).await?;
            }
            let res=sqlx::query("UPDATE messages SET content='',reply_to=NULL,edited_at=NULL,deleted_at=? WHERE id=? AND deleted_at IS NULL").bind(now() as i64).bind(id(m)?).execute(&mut *c).await?;
            if res.rows_affected() > 0 {
                let pinned: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM pins WHERE channel_id=? AND message_id=?)",
                )
                .bind(id(ch)?)
                .bind(id(m)?)
                .fetch_one(&mut *c)
                .await?;
                if pinned {
                    event(
                        out,
                        audience(c, ch).await?,
                        Event::PinsUpdated {
                            channel_id: ch,
                            message_id: m,
                            pinned: false,
                        },
                    );
                }
                for query in [
                    "DELETE FROM pins WHERE message_id=?",
                    "DELETE FROM message_mentions WHERE message_id=?",
                    "DELETE FROM message_broadcast_mentions WHERE message_id=?",
                    "DELETE FROM reactions WHERE message_id=?",
                    "DELETE FROM message_attachments WHERE message_id=?",
                    "DELETE FROM forwarded_messages WHERE message_id=?",
                    "DELETE FROM polls WHERE message_id=?",
                    "DELETE FROM link_embeds WHERE message_id=?",
                ] {
                    sqlx::query(query).bind(id(m)?).execute(&mut *c).await?;
                }
                crate::threads::source_deleted(c, m).await?;
                event(
                    out,
                    audience(c, ch).await?,
                    Event::MessageDeleted {
                        channel_id: ch,
                        message_id: m,
                    },
                );
                reads_event(c, out, ch).await?;
            }
            Ok(CommandResult::Ack)
        }
        AddReaction {
            channel_id: ch,
            message_id: m,
            mut emoji,
        }
        | RemoveReaction {
            channel_id: ch,
            message_id: m,
            mut emoji,
        } => {
            let added = adding_reaction;
            let channel = text(c, u, ch, Permissions::ADD_REACTIONS).await?;
            command_message(c, u, ch, m, false).await?;
            // Custom emoji: from a server the reactor is in (expression.rs).
            emoji = crate::expression::reaction_emoji(c, u, &channel, &emoji, added).await?;
            let changed = if added {
                let n: i64 = sqlx::query_scalar(
                    "SELECT COUNT(DISTINCT emoji) FROM reactions WHERE message_id=? AND emoji<>?",
                )
                .bind(id(m)?)
                .bind(&emoji)
                .fetch_one(&mut *c)
                .await?;
                require(n < 20, ErrorCode::LimitExceeded)?;
                sqlx::query("INSERT OR IGNORE INTO reactions VALUES(?,?,?)")
                    .bind(id(m)?)
                    .bind(id(u)?)
                    .bind(&emoji)
                    .execute(&mut *c)
                    .await?
                    .rows_affected()
                    > 0
            } else {
                sqlx::query("DELETE FROM reactions WHERE message_id=? AND user_id=? AND emoji=?")
                    .bind(id(m)?)
                    .bind(id(u)?)
                    .bind(&emoji)
                    .execute(&mut *c)
                    .await?
                    .rows_affected()
                    > 0
            };
            let mut result = None;
            for v in audience(c, ch).await? {
                let r=sqlx::query("SELECT COUNT(*) AS n,COALESCE(MAX(user_id=?),0) AS me FROM reactions WHERE message_id=? AND emoji=?").bind(id(v)?).bind(id(m)?).bind(&emoji).fetch_one(&mut *c).await?;
                let update = ReactionUpdate {
                    channel_id: ch,
                    message_id: m,
                    user_id: u,
                    added,
                    reaction: ReactionSummary {
                        emoji: emoji.clone(),
                        count: r.get::<i64, _>("n") as u32,
                        me: r.get::<i64, _>("me") != 0,
                    },
                };
                if v == u {
                    result = Some(update.clone());
                }
                if changed {
                    event(out, vec![v], Event::ReactionUpdated(update));
                }
            }
            Ok(CommandResult::Reaction(
                result.ok_or_else(|| Error::new(ErrorCode::Internal))?,
            ))
        }
        Typing { channel_id: ch } => {
            let channel = text(c, u, ch, Permissions::SEND_MESSAGES).await?;
            if channel.summary.kind == ChannelKind::Direct {
                for v in &channel.summary.recipient_ids {
                    if *v != u {
                        require(!blocked(c, u, *v).await?, ErrorCode::Blocked)?;
                    }
                }
            }
            if core
                .rate(format!("typing:{}:{}", u.0, ch.0), 15., 1.)
                .is_ok()
            {
                crate::expression::typing(c, u, ch, out).await?;
                event(
                    out,
                    audience(c, ch)
                        .await?
                        .into_iter()
                        .filter(|v| *v != u)
                        .collect(),
                    Event::Typing {
                        channel_id: ch,
                        user_id: u,
                        expires_at: now() + 8000,
                    },
                );
            }
            Ok(CommandResult::Ack)
        }
        MarkRead {
            channel_id: ch,
            message_id: m,
        } => {
            text(c, u, ch, Permissions::VIEW_CHANNEL).await?;
            command_message(c, u, ch, m, true).await?;
            if let Some(r) = crate::threads::untracked(c, u, ch).await? {
                return Ok(CommandResult::ReadState(r));
            }
            let old = read(c, u, ch).await?;
            if old.last_read_id.is_none_or(|old| m > old) {
                sqlx::query(
                    "UPDATE read_states SET last_read_id=? WHERE user_id=? AND channel_id=?",
                )
                .bind(id(m)?)
                .bind(id(u)?)
                .bind(id(ch)?)
                .execute(&mut *c)
                .await?;
                recount(c, ch).await?;
                event(out, vec![u], Event::ReadStateUpdated(read(c, u, ch).await?));
            }
            Ok(CommandResult::ReadState(read(c, u, ch).await?))
        }
        SubscribeGuild {
            guild_id: g,
            presence_user_ids: ids,
        } => {
            unique(&ids, 100)?;
            if let Some(g) = g {
                role(c, u, g).await?;
                for v in &ids {
                    role(c, *v, g)
                        .await
                        .map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
                }
            } else {
                require(ids.is_empty(), ErrorCode::InvalidRequest)?;
            }
            let s = core
                .streams
                .get_mut(key)
                .ok_or_else(|| Error::new(ErrorCode::Internal))?;
            s.guild = g;
            s.interests = ids.clone();
            let mut presences = vec![];
            for v in ids {
                presences.push(presence(c, core, u, v).await?);
            }
            Ok(CommandResult::Subscription { presences })
        }
    }
}
#[cfg(test)]
mod audit_receipt_tests {
    use super::*;
    #[tokio::test]
    async fn receipts_cannot_return_removed_nested_entities() {
        let (_dir, app) = crate::audit_tests::fixture().await;
        let mut c = app.0.pool.acquire().await.unwrap();
        let detail = CommandResult::Guild(detail(&mut c, Id(1), Id(10)).await.unwrap());
        let member = CommandResult::Member(member(&mut c, Id(10), Id(2)).await.unwrap());
        sqlx::query("INSERT INTO messages(id,channel_id,author_id,content,created_at) VALUES(12,11,1,'hello',0)").execute(&mut *c).await.unwrap();
        let reaction = CommandResult::Reaction(ReactionUpdate {
            channel_id: Id(11),
            message_id: Id(12),
            user_id: Id(1),
            added: true,
            reaction: ReactionSummary {
                emoji: "👍".into(),
                count: 1,
                me: true,
            },
        });
        let code = "a".repeat(22);
        sqlx::query("INSERT INTO invites(id,code_hash,code,guild_id,creator_id,expires_at,max_uses,uses) VALUES(30,?,?,10,1,?,2,0)").bind(hash(code.as_bytes())).bind(&code).bind((now()+60000) as i64).execute(&mut *c).await.unwrap();
        let invite = CommandResult::Invite(invite(&mut c, Id(30)).await.unwrap());
        for result in [&detail, &member, &reaction, &invite] {
            result_access(&mut c, Id(1), result).await.unwrap();
        }
        sqlx::query("DELETE FROM guild_members WHERE user_id=2")
            .execute(&mut *c)
            .await
            .unwrap();
        sqlx::query("DELETE FROM invites WHERE id=30")
            .execute(&mut *c)
            .await
            .unwrap();
        sqlx::query("UPDATE messages SET deleted_at=1,content='' WHERE id=12")
            .execute(&mut *c)
            .await
            .unwrap();
        for result in [&member, &reaction, &invite] {
            assert_eq!(
                result_access(&mut c, Id(1), result)
                    .await
                    .unwrap_err()
                    .0
                    .code,
                ErrorCode::NotFound
            );
        }
        sqlx::query("DELETE FROM channels WHERE id=11")
            .execute(&mut *c)
            .await
            .unwrap();
        assert_eq!(
            result_access(&mut c, Id(1), &detail)
                .await
                .unwrap_err()
                .0
                .code,
            ErrorCode::NotFound
        );
    }
}
