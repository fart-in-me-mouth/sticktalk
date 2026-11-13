//! A bounded sparse history window backed by server rank anchors, not ID/time interpolation.
//! Scroll geometry stays fixed during an asynchronous landing, including skeletons.
use crate::{
    app::{App, Msg, Screen},
    backend::{HistoryAnchor as FetchAnchor, Request},
    theme as t,
};
use iced::{
    widget::{operation, scrollable},
    Task,
};
use sticktalk_proto::*;
#[derive(Default)]
pub struct Window {
    pub channel: Option<Id>,
    pub count: u64,
    pub anchors: Vec<HistoryAnchor>,
    pub start_rank: u64,
    pub loaded: bool,
    pub ends_at_present: bool,
    pub window_top: Option<f32>,
    pub landed_anchor: Option<Id>,
    pub landing_y: Option<f32>,
    pub pending: Option<(u64, HistoryAnchor)>,
}
impl Window {
    pub fn active(&self) -> bool {
        self.channel.is_some() && self.count >= 200
    }
    pub fn total(&self) -> f32 {
        self.count as f32 * t::HISTORY_ROW + t::VIRTUAL_HEADER
    }
    pub fn top(&self) -> f32 {
        self.start_rank as f32 * t::HISTORY_ROW + t::VIRTUAL_HEADER
    }
    pub fn top_for(&self, height: f32) -> f32 {
        let end = (self.total() - height).max(0.);
        if self.ends_at_present {
            end
        } else {
            self.window_top.unwrap_or_else(|| self.top()).clamp(0., end)
        }
    }
    pub fn nearest(&self, y: f32) -> Option<HistoryAnchor> {
        let rank = ((y - t::VIRTUAL_HEADER).max(0.) / t::HISTORY_ROW) as u64;
        self.anchors
            .iter()
            .min_by_key(|a| a.rank.abs_diff(rank))
            .cloned()
    }
}
impl App {
    pub(crate) fn extent_snapshot(&mut self, e: &HistoryExtent) {
        if self.state.screen != Screen::Channel(e.channel_id) {
            return;
        }
        let old = self.extent.active();
        self.extent.channel = Some(e.channel_id);
        self.extent.count = e.message_count;
        self.extent.anchors = e.anchors.clone();
        if self.extent.active() && !old {
            let n = self.rows().len() as u64;
            self.extent.start_rank = e.message_count.saturating_sub(n);
            self.extent.loaded = true;
            self.extent.ends_at_present = self.state.at_present;
        }
    }
    pub(crate) fn extent_landing(&mut self) -> bool {
        if !self.extent.active() {
            return false;
        }
        let rows = self.rows();
        let loaded_height = rows.iter().map(|(_, h, _)| *h).sum::<f32>();
        let center = self.scroll_y + self.viewport_h / 2.;
        let top = self.extent.top_for(loaded_height);
        if self.extent.loaded && center >= top && center < top + loaded_height {
            return true;
        }
        // Wheel traversal follows adjacent bodies, preserving the old boundary's
        // absolute position. Only distant thumb jumps use a sparse server anchor.
        let adjacent = if self.extent.loaded
            && center < top
            && top - center < self.viewport_h * 2.
            && self.state.before.is_some()
        {
            rows.first().map(|(m, _, _)| {
                (
                    HistoryAnchor {
                        rank: self.extent.start_rank,
                        message_id: m.id,
                    },
                    top,
                )
            })
        } else if self.extent.loaded
            && center >= top + loaded_height
            && center - top - loaded_height < self.viewport_h * 2.
            && self.state.after.is_some()
        {
            rows.last().map(|(m, h, _)| {
                (
                    HistoryAnchor {
                        rank: self.extent.start_rank + rows.len().saturating_sub(1) as u64,
                        message_id: m.id,
                    },
                    top + loaded_height - h,
                )
            })
        } else {
            None
        };
        let Some((anchor, landing_y)) =
            adjacent.or_else(|| self.extent.nearest(center).map(|a| (a, center)))
        else {
            return true;
        };
        if self
            .extent
            .pending
            .as_ref()
            .is_some_and(|(_, a)| a.message_id == anchor.message_id)
        {
            return true;
        }
        self.state.history_seek = self.state.history_seek.wrapping_add(1);
        let seek = self.state.history_seek;
        self.extent.loaded = false;
        self.extent.landing_y = Some(landing_y);
        self.extent.pending = Some((seek, anchor.clone()));
        self.request(Request::AroundHistory {
            channel: self.extent.channel.unwrap(),
            anchor: FetchAnchor::Message(anchor.message_id),
            seek,
            generation: self.state.generation,
        });
        true
    }
    pub(crate) fn extent_page(
        &mut self,
        channel: Id,
        seek: u64,
        page: Page<Message>,
    ) -> Option<Task<Msg>> {
        let (serial, anchor) = self.extent.pending.clone()?;
        if serial != seek || self.extent.channel != Some(channel) {
            return None;
        }
        self.extent.pending = None;
        self.state.seek_loading = false;
        self.state.jump_seek = None;
        if !self.state.model.snapshot_valid(&page.cursor) {
            self.error = "History changed. Try this position again.".into();
            return Some(Task::none());
        }
        let before = page
            .items
            .iter()
            .filter(|m| m.id < anchor.message_id)
            .count() as u64;
        self.extent.start_rank = anchor.rank.saturating_sub(before);
        self.extent.loaded = true;
        self.extent.ends_at_present = page.next_after.is_none() && self.state.at_present;
        self.state.before = page.next_before;
        self.state.after = page.next_after;
        self.state.model.messages.remove(&channel);
        self.state.model.merge_page(channel, page);
        self.reset_history_geometry();
        let before_height = self
            .rows()
            .iter()
            .take_while(|(m, _, _)| m.id < anchor.message_id)
            .map(|(_, h, _)| *h)
            .sum::<f32>();
        let landing_y = self
            .extent
            .landing_y
            .take()
            .unwrap_or(self.scroll_y + self.viewport_h / 2.);
        self.extent.window_top = Some((landing_y - before_height).max(0.));
        if self.state.before.is_none() && self.scroll_y < self.viewport_h {
            self.extent.window_top = Some(0.);
        }
        self.extent.landed_anchor = Some(anchor.message_id);
        // Do not center on arrival: the user's absolute thumb position is the anchor.
        Some(operation::scroll_to(
            "messages",
            scrollable::AbsoluteOffset {
                x: 0.,
                y: self.scroll_y,
            },
        ))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sparse_anchors_are_channel_ranks_not_snowflake_distances() {
        let w = Window {
            channel: Some(Id(1)),
            count: 100_000,
            anchors: vec![
                HistoryAnchor {
                    rank: 0,
                    message_id: Id(3),
                },
                HistoryAnchor {
                    rank: 50_000,
                    message_id: Id(4),
                },
                HistoryAnchor {
                    rank: 99_999,
                    message_id: Id(i64::MAX as u64),
                },
            ],
            ..Default::default()
        };
        assert_eq!(
            w.nearest(t::VIRTUAL_HEADER + 49_900. * t::HISTORY_ROW)
                .unwrap()
                .message_id,
            Id(4)
        );
        assert_eq!(w.anchors.len(), 3);
    }
    #[test]
    fn landing_preserves_thumb_geometry_and_ignores_stale_pages() {
        let mut app = App::headless("main-history-skeleton", iced::Size::new(1280., 800.));
        let y = app.scroll_y;
        let total = app.extent.total();
        let anchor = app.extent.nearest(y).unwrap();
        app.state.history_seek = 9;
        app.extent.pending = Some((9, anchor.clone()));
        let mut mock = crate::mock::Mock::new();
        mock.configure("scroll-5000");
        let Some(crate::backend::Data::AroundHistory { page, .. }) = mock
            .request::<std::path::PathBuf, crate::backend::Handle>(Request::AroundHistory {
                channel: Id(12),
                anchor: FetchAnchor::Message(anchor.message_id),
                seek: 9,
                generation: app.state.generation,
            })
        else {
            panic!()
        };
        assert!(app.extent_page(Id(12), 8, page.clone()).is_none());
        assert!(app.extent_page(Id(12), 9, page).is_some());
        assert_eq!(app.scroll_y, y);
        assert_eq!(app.extent.total(), total);
        assert!(app.extent.loaded);
        assert!(app.state.model.retained_count() <= 50);
    }
}
