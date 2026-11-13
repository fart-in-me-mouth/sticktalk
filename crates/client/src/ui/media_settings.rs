//! Additive settings section; kept separate from the settings team's screens.
use super::*;
use iced::widget::column;
use sticktalk_client_core::gifs::Autoplay;
pub fn view(app: &App) -> Element<'_, Msg> {
    let mut choices = row![].spacing(t::S8);
    for (label, mode) in [
        ("Always", Autoplay::Always),
        ("On hover", Autoplay::OnHover),
        ("Never", Autoplay::Never),
    ] {
        choices = choices.push(
            action(label, Msg::feature(crate::features::Event::Autoplay(mode)))
                .style(t::navigation(app.features.autoplay == mode)),
        );
    }
    column![
        title("Play GIFs automatically"),
        hint("Animations pause when StickTalk is unfocused or the image is offscreen."),
        choices
    ]
    .spacing(t::S12)
    .into()
}
