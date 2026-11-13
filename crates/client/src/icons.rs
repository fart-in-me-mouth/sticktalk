//! Bundled Lucide SVG subset (ISC).
use iced::{widget::svg, Color};
pub fn icon(name: &str, size: f32, color: Color) -> svg::Svg<'static> {
    let path = match name {
        "square"=>Some("M3 3h18v18H3V3Z"),
        "square-check"=>Some("M3 3h18v18H3V3Z m4 9 3 3 7-7"),
        "volume-2"=>Some("M11 5 6 9H3v6h3l5 4V5Z M15.5 8.5a5 5 0 0 1 0 7 M19 5a10 10 0 0 1 0 14"),
        "mic"=>Some("M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3Z M5 10v2a7 7 0 0 0 14 0v-2 M12 19v3 M8 22h8"),
        "mic-off"=>Some("m2 2 20 20 M9 9v3a3 3 0 0 0 5 2 M15 9V5a3 3 0 0 0-6 0 M5 10v2a7 7 0 0 0 12 5 M19 10v2 M12 19v3 M8 22h8"),
        "headphones"=>Some("M3 14v-3a9 9 0 0 1 18 0v3 M3 13h4v8H3v-8Z M17 13h4v8h-4v-8Z"),
        "headphone-off"=>Some("m2 2 20 20 M3 14v-3a9 9 0 0 1 2-5 M9 2.5A9 9 0 0 1 21 11v3 M3 13h4v8H3v-8Z M17 17v4h4"),
        "video"=>Some("m16 8 6-4v16l-6-4 M2 6h14v12H2V6Z"),
        "monitor-up"=>Some("M3 3h18v14H3V3Z M7 21h10 M12 17v4 M12 13V6 m-3 3 3-3 3 3"),
        "phone-off"=>Some("m2 2 20 20 M7 3H4a1 1 0 0 0-1 1c0 9 8 17 17 17a1 1 0 0 0 1-1v-3l-5-2-2 2a16 16 0 0 1-7-7l2-2-2-5Z"),
        "signal"=>Some("M3 20v-3 M8 20v-7 M13 20V9 M18 20V5 M23 20V1"),
        "lock-keyhole"=>Some("M5 10h14v11H5V10Z M8 10V6a4 4 0 0 1 8 0v4 M12 14v3"),
        "pin"=>Some("m16 3 5 5-4 2-3 5-5-5 5-3 2-4Z M9 15l-6 6"),
        "pin-off"=>Some("m2 2 20 20 M16 3l5 5-4 2-1 2 M9 10l-2 2 5 5 2-2 M9 15l-6 6"),
        "crown"=>Some("m2 5 5 5 5-7 5 7 5-5-3 14H5L2 5Z M5 22h14"),
        _=>None,
    };
    if let Some(path) = path {
        return svg::Svg::new(svg::Handle::from_memory(format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="{path}"/></svg>"#).into_bytes())).width(size).height(size).style(crate::theme::svg(color));
    }
    let bytes: &'static [u8] = match name {
        "mic" => include_bytes!("../assets/icons/mic.svg"),
        "mic-off" => include_bytes!("../assets/icons/mic-off.svg"),
        "headphones" => include_bytes!("../assets/icons/headphones.svg"),
        "video" => include_bytes!("../assets/icons/video.svg"),
        "monitor-up" => include_bytes!("../assets/icons/monitor-up.svg"),
        "phone-off" => include_bytes!("../assets/icons/phone-off.svg"),
        "volume-2" => include_bytes!("../assets/icons/volume-2.svg"),
        "signal" => include_bytes!("../assets/icons/signal.svg"),
        "minus" => include_bytes!("../assets/icons/minus.svg"),
        "star" => include_bytes!("../assets/icons/star.svg"),
        "forward" => include_bytes!("../assets/icons/forward.svg"),
        "coffee" => include_bytes!("../assets/icons/coffee.svg"),
        "trophy" => include_bytes!("../assets/icons/trophy.svg"),
        "plane" => include_bytes!("../assets/icons/plane.svg"),
        "lightbulb" => include_bytes!("../assets/icons/lightbulb.svg"),
        "heart" => include_bytes!("../assets/icons/heart.svg"),
        "flag" => include_bytes!("../assets/icons/flag.svg"),
        "monitor" => include_bytes!("../assets/icons/monitor.svg"),
        "keyboard" => include_bytes!("../assets/icons/keyboard.svg"),
        "accessibility" => include_bytes!("../assets/icons/accessibility.svg"),

        "arrow-down" => include_bytes!("../assets/icons/arrow-down.svg"),
        "arrow-left" => include_bytes!("../assets/icons/arrow-left.svg"),
        "arrow-right" => include_bytes!("../assets/icons/arrow-right.svg"),
        "arrow-up" => include_bytes!("../assets/icons/arrow-up.svg"),
        "at-sign" => include_bytes!("../assets/icons/at-sign.svg"),
        "ban" => include_bytes!("../assets/icons/ban.svg"),
        "bell-off" => include_bytes!("../assets/icons/bell-off.svg"),
        "bell" => include_bytes!("../assets/icons/bell.svg"),
        "check-check" => include_bytes!("../assets/icons/check-check.svg"),
        "check" => include_bytes!("../assets/icons/check.svg"),
        "chevron-down" => include_bytes!("../assets/icons/chevron-down.svg"),
        "chevron-right" => include_bytes!("../assets/icons/chevron-right.svg"),
        "chevron-up" => include_bytes!("../assets/icons/chevron-up.svg"),
        "circle-help" => include_bytes!("../assets/icons/circle-help.svg"),
        "circle" => include_bytes!("../assets/icons/circle.svg"),
        "code" => include_bytes!("../assets/icons/code.svg"),
        "copy" => include_bytes!("../assets/icons/copy.svg"),
        "download" => include_bytes!("../assets/icons/download.svg"),
        "external-link" => include_bytes!("../assets/icons/external-link.svg"),
        "file" => include_bytes!("../assets/icons/file.svg"),
        "folder" => include_bytes!("../assets/icons/folder.svg"),
        "hash" => include_bytes!("../assets/icons/hash.svg"),
        "image" => include_bytes!("../assets/icons/image.svg"),
        "layout-grid" => include_bytes!("../assets/icons/layout-grid.svg"),
        "link" => include_bytes!("../assets/icons/link.svg"),
        "loader" => include_bytes!("../assets/icons/loader.svg"),
        "log-out" => include_bytes!("../assets/icons/log-out.svg"),
        "mail" => include_bytes!("../assets/icons/mail.svg"),
        "menu" => include_bytes!("../assets/icons/menu.svg"),
        "message-circle" => include_bytes!("../assets/icons/message-circle.svg"),
        "moon" => include_bytes!("../assets/icons/moon.svg"),
        "more-horizontal" => include_bytes!("../assets/icons/more-horizontal.svg"),
        "palette" => include_bytes!("../assets/icons/palette.svg"),
        "paperclip" => include_bytes!("../assets/icons/paperclip.svg"),
        "pencil" => include_bytes!("../assets/icons/pencil.svg"),
        "plus-circle" => include_bytes!("../assets/icons/plus-circle.svg"),
        "plus" => include_bytes!("../assets/icons/plus.svg"),
        "reply" => include_bytes!("../assets/icons/reply.svg"),
        "search" => include_bytes!("../assets/icons/search.svg"),
        "send" => include_bytes!("../assets/icons/send.svg"),
        "settings" => include_bytes!("../assets/icons/settings.svg"),
        "shield" => include_bytes!("../assets/icons/shield.svg"),
        "smile" => include_bytes!("../assets/icons/smile.svg"),
        "sprout" => include_bytes!("../assets/icons/sprout.svg"),
        "sun" => include_bytes!("../assets/icons/sun.svg"),
        "trash-2" => include_bytes!("../assets/icons/trash-2.svg"),
        "user-minus" => include_bytes!("../assets/icons/user-minus.svg"),
        "user-plus" => include_bytes!("../assets/icons/user-plus.svg"),
        "user-round" => include_bytes!("../assets/icons/user-round.svg"),
        "users" => include_bytes!("../assets/icons/users.svg"),
        "wifi-off" => include_bytes!("../assets/icons/wifi-off.svg"),
        "x" => include_bytes!("../assets/icons/x.svg"),
        _ => include_bytes!("../assets/icons/circle.svg"),
    };
    svg::Svg::new(svg::Handle::from_memory(bytes))
        .width(size)
        .height(size)
        .style(crate::theme::svg(color))
}

/// SVG foreground follows its enclosing button, including the whole hit area's hover state.
pub fn inherited<'a>(name: &str, size: f32) -> iced::Element<'a, crate::app::Msg> {
    iced::Element::new(InheritedIcon {
        name: name.into(),
        size,
    })
}
struct InheritedIcon {
    name: String,
    size: f32,
}
impl iced::advanced::Widget<crate::app::Msg, iced::Theme, iced::Renderer> for InheritedIcon {
    fn size(&self) -> iced::Size<iced::Length> {
        iced::Size::new(self.size.into(), self.size.into())
    }
    fn layout(
        &mut self,
        _: &mut iced::advanced::widget::Tree,
        _: &iced::Renderer,
        limits: &iced::advanced::layout::Limits,
    ) -> iced::advanced::layout::Node {
        iced::advanced::layout::Node::new(limits.resolve(
            self.size,
            self.size,
            iced::Size::new(self.size, self.size),
        ))
    }
    fn draw(
        &self,
        tree: &iced::advanced::widget::Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &iced::advanced::renderer::Style,
        layout: iced::advanced::Layout<'_>,
        cursor: iced::mouse::Cursor,
        viewport: &iced::Rectangle,
    ) {
        <svg::Svg<'static> as iced::advanced::Widget<
            crate::app::Msg,
            iced::Theme,
            iced::Renderer,
        >>::draw(
            &icon(&self.name, self.size, style.text_color),
            tree,
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }
}
