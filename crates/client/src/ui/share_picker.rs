use super::*;
use crate::voice_ui::Action;
use iced::widget::{column, row};
pub(super) fn view(app: &App) -> Element<'_, Msg> {
    let mut sources = column![].spacing(t::S8);
    for (index, s) in app.calls.sources.iter().enumerate() {
        let thumbnail: Element<'_, Msg> = s.thumbnail.as_ref().map_or_else(
            || {
                container(icon("monitor-up", t::ICON, t::TEXT_MUTED))
                    .width(96)
                    .height(54)
                    .center_x(96)
                    .center_y(54)
                    .style(t::elevated)
                    .into()
            },
            |handle| {
                image(handle.clone())
                    .width(96)
                    .height(54)
                    .content_fit(iced::ContentFit::Contain)
                    .into()
            },
        );
        sources = sources.push(
            button(
                row![
                    thumbnail,
                    icon(
                        if s.kind == sticktalk_media::SourceKind::Monitor {
                            "monitor-up"
                        } else {
                            "layout-grid"
                        },
                        t::ICON,
                        t::TEXT_SECONDARY
                    ),
                    column![
                        txt(ellipsis(&s.name, 42)),
                        hint(format!("{} × {}", s.size.width, s.size.height))
                    ]
                    .width(Fill)
                ]
                .spacing(t::S12)
                .align_y(Center),
            )
            .on_press(Msg::Call(Action::ChooseSource(index)))
            .padding(t::S12)
            .width(Fill)
            .style(t::navigation(app.calls.source == Some(index))),
        );
    }
    let s = &app.calls.call.settings;
    let mut fps = row![].spacing(t::S8);
    for f in [15, 30, 60] {
        fps = fps.push(
            button(txt(format!("{f} FPS")))
                .on_press(Msg::Call(Action::Fps(f)))
                .padding(t::S8)
                .style(t::navigation(s.share_fps == f)),
        );
    }
    let source = app.calls.source.and_then(|i| app.calls.sources.get(i));
    let mut resolutions = column![].spacing(t::S4);
    for height in [480, 720, 1080, 1440, 0] {
        let mut settings = s.clone();
        settings.share_height = height;
        let actual = source.and_then(|source| {
            sticktalk::voice::quality(&settings)
                .ok()?
                .output_size(source.size)
                .ok()
        });
        let label = if height == 0 {
            "Source".into()
        } else {
            format!("{height}p")
        };
        let label = actual.map_or(label.clone(), |size| {
            format!("{label} ({} × {})", size.width, size.height)
        });
        resolutions = resolutions.push(
            button(txt(label))
                .on_press(Msg::Call(Action::Height(height)))
                .width(Fill)
                .padding(t::S8)
                .style(t::navigation(s.share_height == height)),
        );
    }
    container(
        column![
            row![
                title("Share Your Screen").width(Fill),
                icon_button("x", "Close", Msg::Call(Action::ClosePicker))
            ]
            .align_y(Center),
            container(scroller(sources)).max_height(220),
            line(),
            section("Frame Rate"),
            fps,
            section("Resolution"),
            resolutions,
            widget::checkbox(s.share_include_audio)
                .label("Include application audio")
                .on_toggle(|a| Msg::Call(Action::IncludeAudio(a)))
                .style(t::checkbox),
            row![
                space().width(Fill),
                action("Cancel", Msg::Call(Action::ClosePicker)),
                primary("Go Live", Msg::Call(Action::StartShare))
            ]
            .spacing(t::S8)
        ]
        .spacing(t::S12),
    )
    .padding(t::S24)
    .width(560)
    .style(t::elevated)
    .into()
}
