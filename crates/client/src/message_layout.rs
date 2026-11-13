//! Bounded persistent message-body widget state and layout. Keeping iced's Tree keeps
//! cosmic-text's shaped Paragraphs alive even after a row leaves the visible window.
//! This wrapper is deliberately for inline/code content (no popup-producing widgets).
use crate::app::Msg;
use iced::{
    advanced::{
        layout, mouse, renderer,
        widget::{Operation, Tree},
        Clipboard, Layout, Shell, Widget,
    },
    Element, Event, Length, Rectangle, Size,
};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, VecDeque},
    rc::Rc,
};
use sticktalk_proto::Id;
#[derive(Default)]
pub struct Cache {
    entries: RefCell<BTreeMap<Id, Rc<RefCell<Entry>>>>,
    lru: RefCell<VecDeque<Id>>,
    pub hits: Cell<u64>,
    pub misses: Cell<u64>,
}
struct Entry {
    tree: Tree,
    layout: Option<(u64, layout::Limits, layout::Node)>,
}
impl Cache {
    pub fn clear(&self) {
        self.entries.borrow_mut().clear();
        self.lru.borrow_mut().clear();
    }
    pub fn wrap<'a>(
        &'a self,
        id: Id,
        revision: u64,
        content: Element<'a, Msg>,
    ) -> Element<'a, Msg> {
        let mut entries = self.entries.borrow_mut();
        let entry = entries
            .entry(id)
            .or_insert_with(|| {
                Rc::new(RefCell::new(Entry {
                    tree: Tree::empty(),
                    layout: None,
                }))
            })
            .clone();
        let mut lru = self.lru.borrow_mut();
        lru.retain(|v| *v != id);
        lru.push_back(id);
        // A few viewports of shaped paragraphs, not an unbounded channel history.
        while entries.len() > 256 {
            if let Some(old) = lru.pop_front() {
                entries.remove(&old);
            }
        }
        Element::new(Cached {
            cache: self,
            entry,
            revision,
            content,
        })
    }
}
struct Cached<'a> {
    cache: &'a Cache,
    entry: Rc<RefCell<Entry>>,
    revision: u64,
    content: Element<'a, Msg>,
}
impl Widget<Msg, iced::Theme, iced::Renderer> for Cached<'_> {
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }
    fn layout(
        &mut self,
        _: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let mut entry = self.entry.borrow_mut();
        entry.tree.diff(self.content.as_widget());
        if let Some((revision, old, node)) = &entry.layout {
            if *revision == self.revision && old == limits {
                self.cache.hits.set(self.cache.hits.get() + 1);
                return node.clone();
            }
        }
        self.cache.misses.set(self.cache.misses.get() + 1);
        let node = self
            .content
            .as_widget_mut()
            .layout(&mut entry.tree, renderer, limits);
        entry.layout = Some((self.revision, *limits, node.clone()));
        node
    }
    fn draw(
        &self,
        _: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &self.entry.borrow().tree,
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }
    fn update(
        &mut self,
        _: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Msg>,
        viewport: &Rectangle,
    ) {
        self.content.as_widget_mut().update(
            &mut self.entry.borrow_mut().tree,
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }
    fn operate(
        &mut self,
        _: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content.as_widget_mut().operate(
            &mut self.entry.borrow_mut().tree,
            layout,
            renderer,
            operation,
        );
    }
    fn mouse_interaction(
        &self,
        _: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &self.entry.borrow().tree,
            layout,
            cursor,
            viewport,
            renderer,
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use iced_runtime::user_interface::{Cache as UiCache, UserInterface};
    #[test]
    fn shaped_tree_and_layout_survive_leaving_the_visible_window() {
        let cache = Cache::default();
        #[cfg(feature = "wgpu")]
        let mut renderer = iced::Renderer::Secondary(iced_tiny_skia::Renderer::new(
            crate::theme::regular(),
            iced::Pixels(16.),
        ));
        #[cfg(not(feature = "wgpu"))]
        let mut renderer =
            iced_tiny_skia::Renderer::new(crate::theme::regular(), iced::Pixels(16.));
        for (revision, width) in [(1, 400.), (1, 400.), (2, 400.), (2, 300.)] {
            // A fresh UI cache models eviction from the virtualized widget tree.
            let body = iced::widget::text("A shaped message body").into();
            let ui = UserInterface::build(
                cache.wrap(Id(1), revision, body),
                Size::new(width, 200.),
                UiCache::new(),
                &mut renderer,
            );
            drop(ui);
        }
        assert_eq!(cache.hits.get(), 1);
        assert_eq!(cache.misses.get(), 3);
        assert_eq!(cache.entries.borrow().len(), 1);
    }
}
