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
    // Fixed-height rows keep even the protocol's 1,000-member page cheap.
    // Overscan covers fast wheel/drag input without shaping offscreen names.
    let overscan = t::PARITY_MEMBER_ROW * 4.;
    let header_height = t::S24 + t::HINT * t::LINE + t::S4;
    let total: f32 = groups
        .iter()
        .map(|(_, ids)| header_height + ids.len() as f32 * t::PARITY_MEMBER_ROW)
        .sum();
    let viewport = (app.height - t::HEADER - t::S16).max(0.);
    let offset = app.main.members_scroll.min((total - viewport).max(0.));
    let start = (offset - overscan).max(0.);
    let end = offset + viewport + overscan;
    let mut y = 0.;
    let mut skipped = 0.;
    for (label, mut ids) in groups {
        ids.sort_by_key(|id| app.state.model.name(*id).to_lowercase());
        if y + header_height >= start && y <= end {
            list = list.push(space().height(skipped)).push(
                container(column![
                    space().height(t::S24),
                    elided::label(format!("{label} — {}", ids.len()))
                        .size(t::HINT)
                        .font(t::semibold())
                        .color(t::TEXT_MUTED)
                        .width(Fill),
                    space().height(t::S4)
                ])
                .height(header_height),
            );
            skipped = 0.;
        } else {
            skipped += header_height;
        }
        y += header_height;
        for id in ids {
            if y + t::PARITY_MEMBER_ROW < start || y > end {
                skipped += t::PARITY_MEMBER_ROW;
                y += t::PARITY_MEMBER_ROW;
                continue;
            }
            list = list.push(space().height(skipped));
            skipped = 0.;
            y += t::PARITY_MEMBER_ROW;
            let hover = app.main.hovered_member == Some(id);
            let alpha = if online(&id) || hover { 1. } else { 0.3 };
            let owner = channel
                .guild_id
                .and_then(|g| app.state.model.guilds.get(&g))
                .is_some_and(|g| g.guild.owner_id == id);
            let mut name = row![elided::label(app.state.model.name(id))
                .size(t::CHANNEL_NAME)
                .line_height(iced::Pixels(t::ICON))
                .width(Fill)
                .font(t::medium())
                .color(t::alpha(controls::name_color(app, id), alpha))]
            .spacing(t::S4)
            .width(Fill)
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
            let mut copy = column![name].spacing(0).width(Fill);
            if !sub.is_empty() {
                copy = copy.push(
                    elided::label(sub)
                        .size(t::HINT)
                        .line_height(iced::Pixels(t::USER_NAME))
                        .width(Fill)
                        .color(t::alpha(t::TEXT_MUTED, alpha)),
                );
            }
            let body = row![
                controls::avatar_fade(
                    app,
                    id,
                    t::SMALL_AVATAR,
                    if hover { t::BG_HOVER } else { t::BG_SIDEBAR },
                    false,
                    alpha
                ),
                copy
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
    list = list.push(space().height(skipped));
    if app.state.member_after.is_some() {
        list = list.push(action("More members", Msg::MoreMembers).style(t::link));
    }
    container(
        scroller(list)
            .id("members")
            .on_scroll(|v| mm(Main::MemberScroll(v.absolute_offset().y)))
            .height(Fill),
    )
    .width(t::MEMBERS)
    .height(Fill)
    .padding([t::S8, t::S16])
    .style(t::sidebar)
    .into()
}
