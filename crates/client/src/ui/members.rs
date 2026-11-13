use super::*;
use crate::main_window::{mm, Entity, Message as Main, Popover};
use iced::widget::column;
pub fn view<'a>(app: &'a App, channel: &'a ChannelSummary) -> Element<'a, Msg> {
    let mut groups: Vec<(String, Vec<Id>)> = vec![];
    let mut assigned = std::collections::BTreeSet::new();
    let ids: Vec<_> = if channel.guild_id.is_some() {
        app.state.model.members.keys().copied().collect()
    } else {
        channel.recipient_ids.clone()
    };
    let online = |id: &Id| {
        app.state
            .model
            .presences
            .get(id)
            .is_some_and(|s| !matches!(s, PresenceStatus::Offline | PresenceStatus::Invisible))
    };
    if let Some(g) = channel.guild_id {
        let mut roles: Vec<_> = app
            .state
            .model
            .launch
            .roles
            .get(&g)
            .into_iter()
            .flatten()
            .filter(|r| r.hoist)
            .collect();
        roles.sort_by_key(|r| std::cmp::Reverse(r.position));
        for r in roles {
            let members: Vec<_> = ids
                .iter()
                .filter(|id| {
                    online(id)
                        && app
                            .state
                            .model
                            .launch
                            .hoist(g, **id)
                            .is_some_and(|h| h.id == r.id)
                })
                .copied()
                .collect();
            if !members.is_empty() {
                assigned.extend(members.iter().copied());
                groups.push((r.name.clone(), members));
            }
        }
    }
    for (label, present) in [("Online", true), ("Offline", false)] {
        let members: Vec<_> = ids
            .iter()
            .filter(|id| !assigned.contains(id) && online(id) == present)
            .copied()
            .collect();
        if !members.is_empty() {
            groups.push((label.into(), members));
        }
    }
    let mut list = column![].spacing(0);
    for (label, mut ids) in groups {
        ids.sort_by_key(|id| app.state.model.name(*id).to_lowercase());
        list = list
            .push(space().height(t::S24))
            .push(section(format!("{label} — {}", ids.len())))
            .push(space().height(t::S4));
        for id in ids {
            let hover = app.main.hovered_member == Some(id);
            let alpha = if online(&id) || hover { 1. } else { 0.3 };
            let owner = channel
                .guild_id
                .and_then(|g| app.state.model.guilds.get(&g))
                .is_some_and(|g| g.guild.owner_id == id);
            let mut name = row![txt(ellipsis(
                &app.state.model.name(id),
                if hover { 13 } else { 17 }
            ))
            .size(t::CHANNEL_NAME)
            .wrapping(iced::widget::text::Wrapping::None)
            .font(t::medium())
            .color(t::alpha(controls::name_color(app, id), alpha))]
            .spacing(t::S4)
            .align_y(Center);
            if owner {
                name = name.push(icon("crown", t::CROWN, t::alpha(t::WARNING, alpha)));
            }
            let sub = app
                .state
                .model
                .users
                .get(&id)
                .and_then(|u| u.status_text.as_deref())
                .unwrap_or("");
            let body = row![
                controls::avatar_fade(
                    app,
                    id,
                    t::SMALL_AVATAR,
                    if hover { t::BG_HOVER } else { t::BG_SIDEBAR },
                    false,
                    alpha
                ),
                column![
                    name,
                    hint(ellipsis(sub, 23)).color(t::alpha(t::TEXT_MUTED, alpha))
                ]
                .spacing(0)
                .width(Fill)
            ]
            .spacing(t::S12)
            .align_y(Center);
            let mut cell = row![button(body)
                .on_press(controls::profile_message(id))
                .width(Fill)
                .height(t::PARITY_MEMBER_ROW)
                .padding(t::S4)
                .style(t::navigation(false))]
            .align_y(Center);
            if hover {
                cell = cell.push(controls::icon_button(
                    "more-horizontal",
                    "More",
                    mm(Main::Open(Popover::Context(Entity::User(id)))),
                    false,
                ));
            }
            list = list.push(observe::anchor(
                mouse_area(container(cell).style(move |_| container::Style {
                    border: Border {
                        radius: t::TOOLTIP_RADIUS.into(),
                        ..Default::default()
                    },
                    ..t::surface(if hover { t::BG_HOVER } else { t::TRANSPARENT })
                }))
                .on_enter(mm(Main::MemberHover(Some(id))))
                .on_exit(mm(Main::MemberHover(None)))
                .on_right_press(mm(Main::Open(Popover::Context(Entity::User(id))))),
            ));
        }
    }
    if app.state.member_after.is_some() {
        list = list.push(action("More members", Msg::MoreMembers).style(t::link));
    }
    container(scroller(list).height(Fill))
        .width(t::MEMBERS)
        .height(Fill)
        .padding([t::S8, t::S16])
        .style(t::sidebar)
        .into()
}
