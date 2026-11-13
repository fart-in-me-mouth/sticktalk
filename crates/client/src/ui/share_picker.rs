use super::*;
use crate::voice_ui::Action;
use iced::widget::{column, row};

fn choice(label: String, selected: bool, action: Action) -> Element<'static, Msg> {
    button(
        row![
            txt(label).width(Fill),
            icon(
                if selected { "check" } else { "circle" },
                t::INLINE_ICON,
                if selected { t::ACCENT } else { t::TEXT_MUTED }
            )
        ]
        .spacing(t::S8)
        .align_y(Center),
    )
    .on_press(Msg::Call(action))
    .padding(t::S8)
    .width(Fill)
    .style(t::navigation(selected))
    .into()
}
fn source_tab(label: &'static str, selected: bool, action: Action) -> Element<'static, Msg> {
    button(column![
        container(txt(label).font(t::semibold()).color(if selected {
            t::TEXT_PRIMARY
        } else {
            t::TEXT_SECONDARY
        }))
        .center_x(Fill)
        .padding([t::S8, t::S16]),
        container(space().height(t::FOCUS))
            .width(Fill)
            .style(move |_| t::surface(if selected { t::ACCENT } else { t::TRANSPARENT })),
    ])
    .on_press(Msg::Call(action))
    .width(Fill)
    .padding(0)
    .style(t::navigation(false))
    .into()
}
pub(super) fn view(app: &App) -> Element<'_, Msg> {
    let mut sources = column![].spacing(t::S8);
    let filtered: Vec<_> = app
        .calls
        .sources
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            (s.kind == sticktalk_media::SourceKind::Window) == app.calls.picker_windows
        })
        .collect();
    for chunk in filtered.chunks(2) {
        let mut tiles = row![].spacing(t::S8);
        for (index, s) in chunk {
            let thumbnail: Element<'_, Msg> = s.thumbnail.as_ref().map_or_else(
                || {
                    container(icon("monitor-up", t::CALL_AVATAR, t::TEXT_MUTED))
                        .center_x(Fill)
                        .center_y(t::SOURCE_PREVIEW_H)
                        .style(t::rail)
                        .into()
                },
                |handle| {
                    image(handle.clone())
                        .width(Fill)
                        .height(t::SOURCE_PREVIEW_H)
                        .content_fit(iced::ContentFit::Contain)
                        .into()
                },
            );
            #[cfg(feature = "wgpu")]
            let thumbnail: Element<'_, Msg> = if let Some(frame) = &s.gpu_thumbnail {
                container(
                    widget::shader(sticktalk::voice::video::Video(frame.clone()))
                        .width(t::SOURCE_PREVIEW_W)
                        .height(frame.size.height as f32),
                )
                .center_x(Fill)
                .center_y(t::SOURCE_PREVIEW_H)
                .style(t::rail)
                .into()
            } else {
                thumbnail
            };
            let label = column![
                txt(ellipsis(&s.name, 28)),
                hint(format!("{} × {}", s.size.width, s.size.height))
            ]
            .spacing(t::S4);
            let content: Element<'_, Msg> = if app.calls.quality_menu {
                row![container(thumbnail).width(t::SOURCE_PREVIEW_W), label]
                    .spacing(t::S12)
                    .align_y(Center)
                    .into()
            } else {
                column![thumbnail, label].spacing(t::S4).into()
            };
            tiles = tiles.push(
                button(content)
                    .on_press(Msg::Call(Action::ChooseSource(*index)))
                    .width(Fill)
                    .padding(t::S8)
                    .style(t::navigation(app.calls.source == Some(*index))),
            );
        }
        sources = sources.push(tiles);
    }
    if filtered.is_empty() {
        sources = sources.push(hint("No sources available. Refresh to try again."));
    }
    let settings = &app.calls.call.settings;
    let source = app
        .calls
        .source
        .and_then(|i| app.calls.sources.get(i))
        .map(|s| &s.source)
        .or(app.calls.active_source.as_ref());
    let mut audio = column![].spacing(t::S4);
    if settings.share_include_audio {
        let name = app
            .calls
            .sources
            .iter()
            .find(|s| s.process_id == app.calls.audio_source && s.process_id.is_some())
            .map(|s| s.name.clone())
            .unwrap_or("Choose an application".into());
        audio = audio.push(action(
            format!("Audio source · {name}"),
            Msg::Call(Action::AudioMenu),
        ));
        if app.calls.audio_menu {
            let mut seen = std::collections::BTreeSet::new();
            for s in &app.calls.sources {
                if let Some(pid) = s.process_id {
                    if seen.insert(pid) {
                        audio = audio.push(choice(
                            ellipsis(&s.name, 42),
                            app.calls.audio_source == Some(pid),
                            Action::AudioProcess(pid),
                        ));
                    }
                }
            }
        }
    }
    let mut quality = column![].spacing(t::S8);
    if app.calls.quality_menu {
        let mut fps = column![section("FRAME RATE")].spacing(t::S4);
        for rate in [15, 30, 60] {
            fps = fps.push(choice(
                format!("{rate} FPS"),
                settings.share_fps == rate,
                Action::Fps(rate),
            ));
        }
        let mut resolution = column![section("RESOLUTION")].spacing(t::S4);
        for height in [480, 720, 1080, 1440, 0] {
            let label = if height == 0 {
                source.map_or("Source".into(), |s| {
                    format!("Source ({}×{})", s.size.width, s.size.height)
                })
            } else {
                format!("{height}p")
            };
            resolution = resolution.push(choice(
                label,
                settings.share_height == height,
                Action::Height(height),
            ));
        }
        quality = quality.push(row![fps.width(Fill), resolution.width(Fill)].spacing(t::S16));
    }
    let resolution = if settings.share_height == 0 {
        "Source".into()
    } else {
        format!("{}p", settings.share_height)
    };
    container(
        scroller(
            column![
                row![
                    title(if app.calls.sharing {
                        "Stream Quality"
                    } else {
                        "Share Your Screen"
                    })
                    .width(Fill),
                    icon_button("x", "Close", Msg::Call(Action::ClosePicker))
                ]
                .align_y(Center),
                row![
                    source_tab(
                        "Screens",
                        !app.calls.picker_windows,
                        Action::PickerTab(false)
                    ),
                    source_tab(
                        "Applications",
                        app.calls.picker_windows,
                        Action::PickerTab(true)
                    ),
                    icon_button(
                        "refresh-cw",
                        "Refresh previews",
                        Msg::Call(Action::RefreshSources)
                    )
                ]
                .spacing(t::S8),
                container(scroller(sources)).max_height(if app.calls.quality_menu {
                    t::SOURCE_PREVIEW_H
                } else {
                    t::PICKER_SOURCES_HEIGHT
                }),
                widget::checkbox(settings.share_include_audio)
                    .label("Include application audio")
                    .on_toggle(|a| Msg::Call(Action::IncludeAudio(a)))
                    .style(t::checkbox),
                audio,
                action(
                    format!("Stream quality · {resolution} · {} FPS", settings.share_fps),
                    Msg::Call(Action::QualityMenu)
                ),
                quality,
                row![
                    space().width(Fill),
                    action("Cancel", Msg::Call(Action::ClosePicker)),
                    button(
                        txt(if app.calls.sharing { "Done" } else { "Go Live" })
                            .font(t::semibold())
                            .style(t::inherit_text)
                    )
                    .on_press_maybe(if app.calls.sharing {
                        Some(Msg::Call(Action::ClosePicker))
                    } else {
                        app.calls.source.map(|_| Msg::Call(Action::StartShare))
                    })
                    .padding([t::S8, t::S16])
                    .height(t::BUTTON)
                    .style(t::primary)
                ]
                .spacing(t::S8)
            ]
            .spacing(t::S12),
        )
        .height(Length::Shrink),
    )
    .max_height((app.height - 2. * t::S24).max(t::HEADER))
    .padding(t::S24)
    .width(t::PICKER_WIDTH)
    .style(t::elevated)
    .into()
}
pub(super) fn cameras(app: &App) -> Element<'_, Msg> {
    let mut devices = column![
        title("Choose a Camera"),
        hint("Your camera starts only after you choose a device.")
    ]
    .spacing(t::S12);
    for (i, camera) in app.calls.cameras.iter().enumerate() {
        devices = devices.push(choice(
            camera.name.clone(),
            app.calls.call.settings.camera_device_id.as_ref() == Some(&camera.id),
            Action::ChooseCamera(i),
        ));
    }
    if app.calls.cameras.is_empty() {
        devices = devices.push(hint("No camera found."));
    }
    container(
        scroller(devices.push(action("Cancel", Msg::Call(Action::CloseCameras))))
            .height(Length::Shrink),
    )
    .max_height((app.height - 2. * t::S24).max(t::HEADER))
    .padding(t::S24)
    .width(t::PICKER_WIDTH)
    .style(t::elevated)
    .into()
}
