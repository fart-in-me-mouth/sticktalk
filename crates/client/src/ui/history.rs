use super::*;
use iced::widget::column;
// Skeletons stand in for missing history, never for already-loaded bodies.
fn missing<'a>(mut list: Column<'a, Msg>, app: &App, start: f32, height: f32) -> Column<'a, Msg> {
    let end = start + height;
    let visible_start = app.scroll_y.max(start).min(end);
    let visible_end = (app.scroll_y + app.viewport_h + t::HISTORY_ROW)
        .max(visible_start)
        .min(end);
    list = list.push(space().height(visible_start - start));
    let mut y = visible_start;
    while y < visible_end {
        let h = t::HISTORY_ROW.min(visible_end - y);
        list = list.push(message_skeleton(h));
        y += h;
    }
    list.push(space().height(end - visible_end))
}
pub fn view(app: &App) -> Element<'_, Msg> {
    let total = app.extent.total();
    let rows = app.rows();
    let height = rows.iter().map(|(_, h, _)| *h).sum::<f32>();
    let top = app.extent.top_for(height);
    let mut list = missing(column![].spacing(0), app, 0., top);
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
    list = missing(
        list,
        app,
        offset,
        (total - t::HISTORY_BOTTOM - offset).max(0.),
    );
    list = list.push(space().height(t::HISTORY_BOTTOM));
    scroller(list)
        .id("messages")
        .on_scroll(Msg::Scroll)
        .height(Fill)
        .into()
}
