//! GIF popover with bounded pages, stale-query protection and server-backed favorites.
use super::*;
use crate::features::{Event, Picker};
use iced::widget::column;
#[derive(Default)]
pub struct State {
    pub capability: GifCapability,
    pub query: String,
    pub serial: u64,
    pub categories: Vec<GifCategory>,
    pub items: Vec<GifItem>,
    pub favorites: Vec<String>,
    pub favorite_items: Vec<GifItem>,
    pub tab: bool,
    pub loading: bool,
    pub cursor: Option<String>,
    pub error: String,
    pub selected: usize,
    pub scroll_y: f32,
    pub request: Option<iced::task::Handle>,
    pub hovered: Option<String>,
}
impl State {
    pub fn items(&self) -> &[GifItem] {
        if self.tab {
            &self.favorite_items
        } else {
            &self.items
        }
    }
}
pub fn view(app: &App) -> Element<'_, Msg> {
    let s = &app.features.gifs;
    let tabs = row![
        button(txt("GIFs").font(t::semibold()))
            .on_press(Msg::feature(Event::GifTab(false)))
            .padding([t::S8, t::S12])
            .style(t::navigation(!s.tab)),
        button(txt("Favorites").font(t::semibold()))
            .on_press(Msg::feature(Event::GifTab(true)))
            .padding([t::S8, t::S12])
            .style(t::navigation(s.tab)),
        space().width(Fill),
        icon_button("x", "Close", Msg::feature(Event::Picker(Picker::None)))
    ]
    .align_y(Center)
    .spacing(t::S8);
    let mut content = column![].spacing(t::S8);
    if !s.tab && s.query.is_empty() {
        let mut cats = column![].spacing(t::S8);
        for chunk in s.categories.chunks(2) {
            let mut line = row![].spacing(t::S8);
            for c in chunk {
                let preview: Element<'_, Msg> = if let Some(frame) = app
                    .features
                    .animations
                    .get(&c.preview.url)
                    .and_then(|a| a.frame())
                {
                    image(frame.clone())
                        .width(Fill)
                        .height(t::ATTACHMENT_CARD)
                        .content_fit(iced::ContentFit::Cover)
                        .border_radius(t::S8)
                        .into()
                } else {
                    container(icon("image", t::S32, t::TEXT_MUTED))
                        .center_x(Fill)
                        .center_y(t::ATTACHMENT_CARD)
                        .style(t::chat)
                        .into()
                };
                let tile = stack![
                    preview,
                    container(
                        container(txt(&c.title).font(t::semibold()))
                            .padding([t::S4, t::S8])
                            .style(t::tooltip)
                    )
                    .padding(t::S8)
                    .align_bottom(Fill)
                ];
                line = line.push(
                    mouse_area(
                        button(tile)
                            .on_press(Msg::feature(Event::GifSearch(c.query.clone())))
                            .padding(0)
                            .width(Fill)
                            .style(t::button),
                    )
                    .on_enter(Msg::feature(Event::GifHover(Some(c.query.clone()))))
                    .on_exit(Msg::feature(Event::GifHover(None))),
                );
            }
            cats = cats.push(line);
        }
        content = content.push(cats);
    }
    for (r, chunk) in s.items().chunks(2).enumerate() {
        let mut line = row![].spacing(t::S8);
        for (i, item) in chunk.iter().enumerate() {
            let favorite = s.favorites.contains(&item.id);
            let frame = app
                .features
                .animations
                .get(&item.preview.url)
                .and_then(|a| a.frame());
            let poster: Element<'_, Msg> = if let Some(frame) = frame {
                image(frame.clone())
                    .width(Fill)
                    .height(t::ATTACHMENT_CARD + t::S24)
                    .content_fit(iced::ContentFit::Cover)
                    .border_radius(t::S8)
                    .into()
            } else {
                container(
                    column![
                        icon("image", t::S32, t::TEXT_MUTED),
                        hint(ellipsis(&item.title, 22))
                    ]
                    .spacing(t::S8),
                )
                .center_x(Fill)
                .center_y(t::ATTACHMENT_CARD + t::S24)
                .style(t::chat)
                .into()
            };
            let send = button(poster)
                .on_press(Msg::feature(Event::GifSend(item.clone())))
                .padding(0)
                .style(t::navigation(s.selected == r * 2 + i));
            let star = container(if s.hovered.as_ref() == Some(&item.id) || favorite {
                icon_button(
                    "star",
                    if favorite {
                        "Remove favorite"
                    } else {
                        "Add favorite"
                    },
                    Msg::feature(Event::GifFavorite(item.id.clone(), !favorite)),
                )
            } else {
                space().width(t::S32).height(t::S32).into()
            })
            .align_right(Fill)
            .align_top(Fill)
            .padding(t::S4);
            line = line.push(
                mouse_area(stack![send, star])
                    .on_enter(Msg::feature(Event::GifHover(Some(item.id.clone()))))
                    .on_exit(Msg::feature(Event::GifHover(None))),
            );
        }
        content = content.push(line);
    }
    if s.loading {
        content = content.push(container(hint("Loading GIFs…")).padding(t::S16));
    }
    if !s.error.is_empty() {
        content = content.push(hint(&s.error).color(t::DANGER));
    }
    if s.tab && s.favorite_items.is_empty() && !s.loading {
        content = content.push(
            container(
                column![
                    icon("star", t::AVATAR, t::TEXT_MUTED),
                    txt("No favorite GIFs yet"),
                    hint("Star a GIF to keep it here.")
                ]
                .spacing(t::S12)
                .align_x(Center),
            )
            .center_x(Fill)
            .height(t::ATTACHMENT_CARD * 2.),
        );
    }
    container(
        column![
            tabs,
            text_input("Search KLIPY", &s.query)
                .id("gif-search")
                .on_input(|q| Msg::feature(Event::GifSearch(q)))
                .padding(t::S12)
                .size(t::BODY)
                .font(t::regular())
                .style(t::input),
            scroller(content)
                .id("gif-results")
                .height(320)
                .on_scroll(|v| Msg::feature(Event::GifScroll(
                    v.relative_offset().y,
                    v.absolute_offset().y
                ))),
            rule::horizontal(t::HAIRLINE).style(t::divider),
            row![
                hint("Powered by KLIPY"),
                space().width(Fill),
                hint("Esc to close")
            ]
        ]
        .spacing(t::S12),
    )
    .padding(t::S12)
    .width(t::PICKER_W)
    .style(t::popover)
    .into()
}
pub fn inline<'a>(app: &'a App, m: &'a Message) -> Option<Element<'a, Msg>> {
    if !crate::gifs::inline_url(&m.content) {
        return None;
    }
    let animation = app.features.animations.get(m.content.trim());
    let picture: Element<'_, Msg> = if let Some(frame) = animation.and_then(|a| a.frame()) {
        image(frame.clone())
            .width(t::THUMB_W)
            .height(t::THUMB_H)
            .content_fit(iced::ContentFit::Contain)
            .border_radius(t::S8)
            .into()
    } else {
        container(column![icon("image", t::S32, t::TEXT_MUTED), hint("GIF")].spacing(t::S8))
            .center_x(t::THUMB_W)
            .center_y(t::THUMB_H)
            .style(t::elevated)
            .into()
    };
    Some(
        mouse_area(
            button(picture)
                .on_press(Msg::feature(Event::GifOpen(m.content.trim().into())))
                .padding(0)
                .style(t::icon),
        )
        .on_enter(Msg::feature(Event::InlineHover(Some(m.id))))
        .on_exit(Msg::feature(Event::InlineHover(None)))
        .into(),
    )
}
