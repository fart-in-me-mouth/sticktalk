//! Re-opening a recently visited conversation at the present costs no request.
//!
//! The model keeps the messages of the four most recently visited
//! conversations, and message events (which reach every conversation you can
//! see, open or not) keep them current. Once a conversation's newest page was
//! loaded in this stream, its cached tail is complete, so switching back shows
//! it from the cache instead of fetching the newest page and the history
//! extent again (`docs/features/integration.md`, "Channel switching").
//! Anything that could leave a gap forgets it: a disconnect or resync, an
//! older or anchored page (which may trim the newest messages), eviction.
use super::*;

/// Newest messages shown when a conversation opens (the history page size).
const WINDOW: usize = 50;

#[derive(Default)]
pub(super) struct HistoryCache {
    /// Conversations whose cached newest messages are complete, with the
    /// stream position of the page they started from.
    fresh: BTreeMap<Id, ResumeCursor>,
    /// History extents of conversations not open now, kept counted by events.
    extents: BTreeMap<Id, HistoryExtent>,
}
impl HistoryCache {
    pub(super) fn clear(&mut self) {
        self.fresh.clear();
        self.extents.clear();
    }
    /// A page was merged for `c`: the newest one (`latest`) or another.
    pub(super) fn loaded(&mut self, c: Id, cursor: ResumeCursor, latest: bool) {
        if latest {
            self.fresh.insert(c, cursor);
        } else {
            self.fresh.remove(&c);
        }
    }
    /// Leaving `c`: keep its extent for a return.
    pub(super) fn park(&mut self, c: Id, extent: Option<HistoryExtent>) {
        match extent {
            Some(e) => {
                self.extents.insert(c, e);
            }
            None => {
                self.extents.remove(&c);
            }
        }
    }
    /// A message was created (+1) or deleted (-1) in a conversation not open.
    pub(super) fn counted(&mut self, c: Id, delta: i64) {
        if let Some(e) = self.extents.get_mut(&c) {
            e.message_count = if delta >= 0 {
                e.message_count.saturating_add(delta as u64)
            } else {
                e.message_count.saturating_sub(delta.unsigned_abs())
            };
        }
    }
    /// Drop what the model no longer caches.
    fn prune(&mut self, model: &sticktalk_client_core::model::Model) {
        self.fresh
            .retain(|c, cursor| model.messages.contains_key(c) && model.snapshot_valid(cursor));
        self.extents.retain(|c, _| model.messages.contains_key(c));
    }
}

/// Show `c` at the present from the cache, if its tail is known complete.
/// Returns false when the newest page has to be fetched.
pub(super) fn reopen(app: &Shared, c: Id) -> bool {
    {
        let mut a = app.borrow_mut();
        let a = &mut *a;
        a.history_cache.prune(&a.model);
        if !a.history_cache.fresh.contains_key(&c) {
            return false;
        }
        let ids: Vec<Id> = a
            .model
            .messages
            .get(&c)
            .map(|m| m.keys().rev().take(WINDOW).rev().copied().collect())
            .unwrap_or_default();
        a.extent = a.history_cache.extents.remove(&c);
        let count = a
            .extent
            .as_ref()
            .map_or(ids.len() as u64, |e| e.message_count);
        a.window_rank = crate::wire::page_rank(count, ids.len(), None);
        a.window_ids = ids;
        a.history_busy = false;
        a.model.touch(c);
        media_action("present");
        a.touch();
    }
    chat_ready();
    operations::mark_read(app);
    true
}
