//! The parallel voice build used bit 14 for Connect, before role negotiation.
//! Canonical storage and launch peers use Connect=19, Manage Roles=14. Never
//! accept legacy masks as authorization input. Project only outbound old DTOs.
use sticktalk_proto::*;
pub(crate) fn mask(p: Permissions) -> Permissions {
    Permissions::from_bits_retain(
        (p.bits() & ((1 << 14) - 1))
            | if p.contains(Permissions::CONNECT) {
                1 << 14
            } else {
                0
            },
    )
}
pub(crate) fn guild(g: &mut GuildSummary) {
    g.my_permissions = mask(g.my_permissions);
}
pub(crate) fn member(m: &mut Member) {
    m.permissions = mask(m.permissions);
}
pub(crate) fn event(e: &mut Event) {
    match e {
        Event::GuildUpsert(g) => guild(g),
        Event::GuildJoined { detail, .. } => guild(&mut detail.summary),
        Event::MemberUpsert(m) => member(m),
        _ => {}
    }
}
pub(crate) fn message(m: &mut ServerMsg) {
    match m {
        ServerMsg::Ready(r) => {
            for g in &mut r.guilds {
                guild(g);
            }
            for m in &mut r.memberships {
                member(m);
            }
            r.roles.clear();
        }
        ServerMsg::Response(r) => {
            if let CommandOutcome::Ok(result) = &mut r.outcome {
                match result {
                    CommandResult::Guild(g) => guild(&mut g.summary),
                    CommandResult::Member(m) => member(m),
                    _ => {}
                }
            }
        }
        ServerMsg::Event(e) => event(&mut e.event),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn voice_and_role_bits_never_alias_across_negotiated_wire_projection() {
        assert_eq!(Permissions::CONNECT.bits(), 1 << 19);
        assert_eq!(Permissions::MANAGE_ROLES.bits(), 1 << 14);
        assert_eq!(mask(Permissions::CONNECT).bits(), 1 << 14);
        assert_eq!(mask(Permissions::MANAGE_ROLES).bits(), 0);
        assert_eq!(
            mask(Permissions::MEMBER | Permissions::CONNECT).bits(),
            16671
        );
        assert_eq!(mask(Permissions::all()).bits(), 32767);
        let identify: Identify =
            serde_json::from_str(r#"{"version":1,"token":"legacy","resume":null}"#).unwrap();
        assert!(!identify.launch_features);
    }
    #[tokio::test]
    async fn persisted_pre_merge_receipts_reproject_permission_masks_before_replay() {
        let (_dir, app) = crate::audit_tests::fixture().await;
        let mut core = app.0.core.lock().await;
        let mut c = app.0.pool.acquire().await.unwrap();
        let mut guild = crate::db::detail(&mut c, Id(2), Id(10)).await.unwrap();
        guild.summary.my_permissions = Permissions::from_bits_retain(16671);
        let command = Command::JoinGuild {
            invite_code: "old-successful-invite".into(),
        };
        let hash = crate::state::hash(&rmp_serde::to_vec_named(&command).unwrap());
        sqlx::query("INSERT INTO command_receipts VALUES(?,?,?,?,?,?)")
            .bind(vec![2u8; 32])
            .bind(80i64)
            .bind(hash)
            .bind(rmp_serde::to_vec_named(&CommandResult::Guild(guild)).unwrap())
            .bind(crate::state::now() as i64)
            .bind((crate::state::now() + 600000) as i64)
            .execute(&mut *c)
            .await
            .unwrap();
        drop(c);
        let response = crate::commands::execute(
            &app,
            &mut core,
            &[2; 32],
            Id(2),
            CommandRequest { nonce: 80, command },
        )
        .await
        .unwrap();
        assert!(response.replayed);
        let CommandOutcome::Ok(CommandResult::Guild(guild)) = response.outcome else {
            panic!()
        };
        assert!(guild.summary.my_permissions.contains(Permissions::CONNECT));
        assert!(!guild
            .summary
            .my_permissions
            .contains(Permissions::MANAGE_ROLES));
        assert_eq!(mask(guild.summary.my_permissions).bits(), 16671);
    }
}
