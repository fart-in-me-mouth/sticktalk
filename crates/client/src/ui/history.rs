use super::*;
use iced::widget::column;
pub fn view(app: &App) -> Element<'_, Msg> {
    let total = app.extent.total();
    let rows = app.rows();
    let height = rows.iter().map(|(_, h, _)| *h).sum::<f32>();
    let top = app.extent.top_for(height);
    let visible =
        app.extent.loaded && app.scroll_y + app.viewport_h >= top && app.scroll_y <= top + height;
    let list = if visible {
        let mut list = column![space().height(top)].spacing(0);
        let mut offset = top;
        for (m, h, grouped) in rows {
            if offset + h >= app.scroll_y - t::HISTORY_ROW * 2.
                && offset <= app.scroll_y + app.viewport_h + t::HISTORY_ROW * 2.
            {
                list = list.push(message_row(app, m, h, grouped));
            } else {
                list = list.push(space().height(h));
            }
            offset += h;
        }
        list.push(space().height((total - offset).max(0.)))
    } else {
        let start = app.scroll_y.clamp(0., (total - app.viewport_h).max(0.));
        let count = ((app.viewport_h / t::HISTORY_ROW).ceil() as usize + 2)
            .min(((total - start) / t::HISTORY_ROW).floor() as usize);
        let mut list = column![space().height(start)].spacing(0);
        for _ in 0..count {
            list = list.push(message_skeleton(t::HISTORY_ROW));
        }
        list.push(space().height((total - start - count as f32 * t::HISTORY_ROW).max(0.)))
    };
    scroller(list)
        .id("messages")
        .on_scroll(Msg::Scroll)
        .height(Fill)
        .into()
}
