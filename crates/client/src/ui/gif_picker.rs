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
pub struct Placement {
    pub index: usize,
    pub column: usize,
    pub top: f32,
    pub height: f32,
}
pub fn category_height(s: &State) -> f32 {
    if !s.tab && s.query.is_empty() {
        s.categories.len().div_ceil(2) as f32 * (t::GIF_CATEGORY_H + t::S8)
    } else {
        0.
    }
}
pub fn placements(s: &State) -> Vec<Placement> {
    let mut heights = [category_height(s); 2];
    s.items()
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let column = usize::from(heights[1] < heights[0]);
            let height = (t::GIF_TILE_W * item.preview.height.max(1) as f32
                / item.preview.width.max(1) as f32)
                .clamp(t::S32 * 2., t::S32 * 10.);
            let top = heights[column];
            heights[column] += height + t::S8;
            Placement {
                index,
                column,
                top,
                height,
            }
        })
        .collect()
}
pub fn visible(s: &State) -> Vec<(&str, bool)> {
    let intersects =
        |top: f32, height: f32| top + height > s.scroll_y && top < s.scroll_y + t::GIF_VIEWPORT_H;
    let mut result = Vec::new();
    if category_height(s) > 0. {
        for (i, c) in s.categories.iter().enumerate() {
            if intersects(
                (i / 2) as f32 * (t::GIF_CATEGORY_H + t::S8),
                t::GIF_CATEGORY_H,
            ) {
                result.push((c.preview.url.as_str(), s.hovered.as_ref() == Some(&c.query)));
            }
        }
    }
    for p in placements(s) {
        if intersects(p.top, p.height) {
            let item = &s.items()[p.index];
            result.push((
                item.preview.url.as_str(),
                s.hovered.as_ref() == Some(&item.id),
            ));
        }
    }
    result.truncate(8);
    result
}
fn preview<'a>(app: &'a App, media: &GifMedia, height: f32) -> Element<'a, Msg> {
    if let Some(frame) = frame(app, &media.url) {
        container(
            image(tile_frame(frame, height))
                .width(t::GIF_TILE_W)
                .height(height)
                .content_fit(iced::ContentFit::Cover)
                .border_radius(t::S4),
        )
        .width(t::GIF_TILE_W)
        .height(height)
        .clip(true)
        .into()
    } else {
        container(icon("image", t::S32, t::TEXT_MUTED))
            .center_x(t::GIF_TILE_W)
            .center_y(height)
            .style(t::chat)
            .into()
    }
}
// Bake the cover crop and corner alpha once per frame: tiny-skia does not
// implement image border radii. The bounded cache also covers animated previews.
fn tile_frame(handle: &image::Handle, height: f32) -> image::Handle {
    use std::{cell::RefCell, collections::VecDeque};
    type Entry = (iced::advanced::image::Id, u32, image::Handle);
    thread_local! { static TILES: RefCell<VecDeque<Entry>> = const { RefCell::new(VecDeque::new()) }; }
    TILES.with(|cache| {
        let mut cache = cache.borrow_mut();
        let key = height.to_bits();
        if let Some((_, _, result)) = cache
            .iter()
            .find(|(id, h, _)| *id == handle.id() && *h == key)
        {
            return result.clone();
        }
        let image::Handle::Rgba {
            width,
            height: source_height,
            pixels,
            ..
        } = handle
        else {
            return handle.clone();
        };
        let Some(source) = ::image::RgbaImage::from_raw(*width, *source_height, pixels.to_vec())
        else {
            return handle.clone();
        };
        let w = t::GIF_TILE_W.round() as u32;
        let h = height.round() as u32;
        let mut tile = ::image::DynamicImage::ImageRgba8(source)
            .resize_to_fill(w, h, ::image::imageops::FilterType::Triangle)
            .to_rgba8();
        for (x, y, pixel) in tile.enumerate_pixels_mut() {
            let dx = (t::S4 - (x as f32 + 0.5).min(w as f32 - x as f32 - 0.5)).max(0.);
            let dy = (t::S4 - (y as f32 + 0.5).min(h as f32 - y as f32 - 0.5)).max(0.);
            let coverage = (t::S4 + 0.5 - dx.hypot(dy)).clamp(0., 1.);
            pixel.0[3] = (pixel.0[3] as f32 * coverage) as u8;
        }
        let result = image::Handle::from_rgba(w, h, tile.into_raw());
        cache.push_back((handle.id(), key, result.clone()));
        while cache.len() > 64 {
            cache.pop_front();
        }
        result
    })
}
fn frame<'a>(app: &'a App, url: &str) -> Option<&'a image::Handle> {
    crate::video::frame(app, url)
        .or_else(|| app.features.animations.get(url).and_then(|a| a.frame()))
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
    if category_height(s) > 0. {
        let mut cats = column![].spacing(t::S8);
        for chunk in s.categories.chunks(2) {
            let mut line = row![].spacing(t::S8);
            for c in chunk {
                let tile = stack![
                    preview(app, &c.preview, t::GIF_CATEGORY_H),
                    container(txt(&c.title).font(t::semibold()).color(Color::WHITE))
                        .center_x(t::GIF_TILE_W)
                        .center_y(t::GIF_CATEGORY_H)
                        .style(|_| container::Style {
                            border: Border {
                                radius: t::S4.into(),
                                ..Default::default()
                            },
                            ..t::surface(t::GIF_CATEGORY_OVERLAY)
                        })
                ];
                line = line.push(
                    mouse_area(
                        button(tile)
                            .on_press(Msg::feature(Event::GifSearch(c.query.clone())))
                            .padding(0)
                            .style(t::icon),
                    )
                    .on_enter(Msg::feature(Event::GifHover(Some(c.query.clone()))))
                    .on_exit(Msg::feature(Event::GifHover(None))),
                );
            }
            cats = cats.push(line);
        }
        content = content.push(cats);
    }
    let mut columns = [
        column![].spacing(t::S8).width(t::GIF_TILE_W),
        column![].spacing(t::S8).width(t::GIF_TILE_W),
    ];
    for p in placements(s) {
        let item = &s.items()[p.index];
        let favorite = s.favorites.contains(&item.id);
        let send = button(preview(app, &item.preview, p.height))
            .on_press(Msg::feature(Event::GifSend(item.clone())))
            .padding(0)
            .style(t::navigation(s.selected == p.index));
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
        .align_right(t::GIF_TILE_W - 2. * t::S4)
        .align_top(p.height - 2. * t::S4)
        .padding(t::S4);
        let tile = mouse_area(stack![send, star])
            .on_enter(Msg::feature(Event::GifHover(Some(item.id.clone()))))
            .on_exit(Msg::feature(Event::GifHover(None)));
        let col = std::mem::replace(
            &mut columns[p.column],
            column![].spacing(t::S8).width(t::GIF_TILE_W),
        );
        columns[p.column] = col.push(tile);
    }
    let [left, right] = columns;
    content = content.push(row![left, right].spacing(t::S8));
    if s.loading {
        content = content.push(container(hint("Loading GIFs…")).padding(t::S16));
    }
    if !s.error.is_empty() {
        content = content.push(
            container(
                column![
                    icon("circle-help", t::S32, t::DANGER),
                    txt(&s.error).color(t::DANGER),
                    action("Try again", Msg::feature(Event::GifSearch(s.query.clone())))
                ]
                .spacing(t::S16)
                .align_x(Center),
            )
            .center_x(Fill)
            .padding(t::S24),
        );
    } else if !s.tab && !s.query.is_empty() && s.items.is_empty() && !s.loading {
        content = content.push(
            container(
                column![
                    icon("search", t::S32, t::TEXT_SECONDARY),
                    txt("No GIFs found"),
                    hint("Try a different word.").color(t::TEXT_SECONDARY),
                    action(
                        "Clear search",
                        Msg::feature(Event::GifSearch(String::new()))
                    )
                ]
                .spacing(t::S16)
                .align_x(Center),
            )
            .center_x(Fill)
            .padding(t::S24),
        );
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
                .height(
                    (app.height - t::ATTACHMENT_CARD - t::S24 - t::S8 - t::S32 * 6.)
                        .clamp(t::S32 * 4., t::GIF_VIEWPORT_H)
                )
                .on_scroll(|v| Msg::feature(Event::GifScroll(
                    v.relative_offset().y,
                    v.absolute_offset().y
                ))),
            rule::horizontal(t::HAIRLINE).style(t::divider),
            hint("Powered by KLIPY")
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
    let picture: Element<'_, Msg> = if let Some(frame) = frame(app, m.content.trim()) {
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tiles_are_cropped_and_rounded_before_rendering() {
        let source = image::Handle::from_rgba(40, 80, vec![255; 40 * 80 * 4]);
        let image::Handle::Rgba {
            width,
            height,
            pixels,
            ..
        } = tile_frame(&source, 96.)
        else {
            panic!()
        };
        assert_eq!((width, height), (t::GIF_TILE_W as u32, 96));
        assert_eq!(pixels[3], 0);
        assert_eq!(pixels[(48 * width as usize + 48) * 4 + 3], 255);
    }
    #[test]
    fn masonry_uses_metadata_and_only_returns_visible_tiles() {
        let mut app = App::headless("gif-search", iced::Size::new(1280., 800.));
        let s = &mut app.features.gifs;
        let layout = placements(s);
        assert_ne!(layout[0].height, layout[1].height);
        assert_eq!(layout[2].column, 0);
        assert_eq!(layout[2].top, layout[0].height + t::S8);
        s.scroll_y = 300.;
        let urls: Vec<_> = visible(s).into_iter().map(|(u, _)| u).collect();
        assert!(!urls.contains(&s.items()[0].preview.url.as_str()));
        assert!(urls.len() <= 8);
    }
}
