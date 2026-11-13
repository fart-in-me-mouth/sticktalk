//! Upload cards are independent of the selected channel and never own transfer tasks.
use super::*;
use crate::features::Event;
use iced::widget::column;
pub fn cards(app: &App) -> Option<Element<'_, Msg>> {
    if app.features.uploads.is_empty() {
        return None;
    }
    let mut list = column![].spacing(t::S8);
    for (&id, v) in &app.features.uploads {
        let speed = v.speed;
        let left = v.job.size.saturating_sub(v.offset);
        let status = if let Some(e) = &v.error {
            e.clone()
        } else if v.job.attachment.is_some() {
            "Ready to send".into()
        } else if speed > 0. {
            format!(
                "{} / {} · {}/s · {}s left",
                bytes(v.offset),
                bytes(v.job.size),
                bytes(speed as u64),
                (left as f64 / speed).ceil() as u64
            )
        } else {
            "Preparing upload…".into()
        };
        let channel = app
            .state
            .model
            .channels
            .get(&v.job.channel)
            .and_then(|c| c.name.as_deref())
            .unwrap_or("direct message");
        let mut controls = row![].spacing(t::S4);
        if v.error.is_some() {
            controls = controls.push(icon_button(
                "arrow-up",
                "Retry upload",
                Msg::feature(Event::UploadRetry(id)),
            ));
        }
        controls = controls.push(icon_button(
            "x",
            "Cancel upload",
            Msg::feature(Event::UploadCancel(id)),
        ));
        let card = column![
            row![
                icon("file", t::S32, t::TEXT_SECONDARY),
                column![
                    txt(ellipsis(
                        &v.job.path.file_name().unwrap_or_default().to_string_lossy(),
                        42
                    )),
                    hint(format!("#{channel} · {status}"))
                ]
                .spacing(t::S4)
                .width(Fill),
                controls
            ]
            .spacing(t::S12)
            .align_y(Center),
            widget::progress_bar(0.0..=v.job.size.max(1) as f32, v.offset as f32)
                .girth(t::S4)
                .style(|_| widget::progress_bar::Style {
                    background: t::BG_SELECTED.into(),
                    bar: if v.error.is_some() {
                        t::DANGER
                    } else {
                        t::ACCENT
                    }
                    .into(),
                    border: Border {
                        radius: t::S4.into(),
                        ..Default::default()
                    }
                })
        ]
        .spacing(t::S8);
        list = list.push(container(card).padding(t::S12).style(t::elevated));
    }
    Some(
        container(scroller(list).height(Length::Shrink))
            .max_height(t::ATTACHMENT_CARD * 2.)
            .into(),
    )
}
pub fn bytes(n: u64) -> String {
    if n >= 1024 * 1024 * 1024 {
        format!("{:.1} GB", n as f64 / (1024. * 1024. * 1024.))
    } else if n >= 1024 * 1024 {
        format!("{:.1} MB", n as f64 / (1024. * 1024.))
    } else {
        format!("{:.0} KB", n as f64 / 1024.)
    }
}
