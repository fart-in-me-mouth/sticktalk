use super::*;
use crate::{features::Event as Feature, video::Event};
use iced::widget::{column, slider};
fn msg(e: Event) -> Msg {
    Msg::feature(Feature::Video(e))
}
pub fn view<'a>(app: &'a App, key: &str, fullscreen: bool) -> Element<'a, Msg> {
    let clip = app.features.video.clips.get(key);
    let (width, height) = if fullscreen {
        (app.width * 0.9, app.height * 0.8)
    } else {
        (t::THUMB_W, t::THUMB_H)
    };
    let poster: Element<'_, Msg> = if let Some(frame) = clip.and_then(|c| c.frame.as_ref()) {
        image(frame.clone())
            .width(width)
            .height(height)
            .content_fit(iced::ContentFit::Contain)
            .border_radius(t::S8)
            .into()
    } else {
        container(
            column![
                icon("video", t::S32, t::TEXT_MUTED),
                hint(clip.map_or("Loading video…", |c| if c.error.is_empty() {
                    "Loading video…"
                } else {
                    &c.error
                }))
            ]
            .spacing(t::S8)
            .align_x(Center),
        )
        .center_x(width)
        .center_y(height)
        .style(t::rail)
        .into()
    };
    let playing = clip.is_some_and(|c| c.playing);
    let position = clip.map_or(0., |c| c.position) as f32;
    let duration = clip.map_or(1., |c| c.duration).max(1.) as f32;
    let volume = clip.map_or(0.5, |c| c.volume);
    let download: Element<'a, Msg> = if let Some(a) = clip.and_then(|c| c.attachment.as_ref()) {
        icon_button("download", "Save video", Msg::Download(a.clone()))
    } else {
        space().width(0).into()
    };
    let seek_key = key.to_owned();
    let volume_key = key.to_owned();
    container(
        column![
            mouse_area(poster).on_press(msg(Event::Toggle(key.into()))),
            row![
                icon_button(
                    if playing { "pause" } else { "play" },
                    if playing { "Pause" } else { "Play" },
                    msg(Event::Toggle(key.into()))
                ),
                slider(0.0..=duration, position.clamp(0., duration), move |v| msg(
                    Event::Seek(seek_key.clone(), f64::from(v))
                ))
                .step(0.1_f32)
                .width(Fill)
                .style(t::slider),
                hint(format!(
                    "{}:{:02}",
                    position as u32 / 60,
                    position as u32 % 60
                )),
                icon("volume-2", t::INLINE_ICON, t::TEXT_SECONDARY),
                slider(0.0..=1., volume, move |v| msg(Event::Volume(
                    volume_key.clone(),
                    v
                )))
                .step(0.05_f32)
                .width(t::S32 * 2.)
                .style(t::slider),
                download,
                icon_button("maximize", "Fullscreen", msg(Event::Fullscreen(key.into())))
            ]
            .spacing(t::S8)
            .align_y(Center)
        ]
        .spacing(t::S4),
    )
    .width(width)
    .padding(t::S4)
    .style(t::elevated)
    .into()
}
pub fn overlay(app: &App) -> Option<Element<'_, Msg>> {
    let key = app.features.video.fullscreen.as_ref()?;
    Some(
        container(column![
            row![
                space().width(Fill),
                icon_button("x", "Close fullscreen", msg(Event::CloseFullscreen))
            ],
            container(view(app, key, true))
                .center_x(Fill)
                .center_y(Fill)
        ])
        .width(Fill)
        .height(Fill)
        .padding(t::S16)
        .style(|_| t::surface(t::VIEWER_BACKDROP))
        .into(),
    )
}
