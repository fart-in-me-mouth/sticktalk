//! Test-only native layout inspection shared by the headless harnesses.
use iced::{
    advanced::widget::{self, Operation},
    Rectangle, Vector,
};
#[derive(Default)]
pub struct Geometry {
    pub viewport: Rectangle,
    pub bounds: Rectangle,
    pub content: Rectangle,
    pub offset: Vector,
    pub composer: Rectangle,
    pub popover: Rectangle,
    pub last: Rectangle,
    pub last_id: String,
}
impl Operation for Geometry {
    fn traverse(&mut self, f: &mut dyn FnMut(&mut dyn Operation)) {
        f(self);
    }
    fn scrollable(
        &mut self,
        id: Option<&widget::Id>,
        bounds: Rectangle,
        content: Rectangle,
        offset: Vector,
        _: &mut dyn widget::operation::Scrollable,
    ) {
        if id == Some(&widget::Id::new("messages")) {
            self.bounds = bounds;
            self.viewport = bounds;
            self.content = content;
            self.offset = offset;
        }
    }
    fn container(&mut self, id: Option<&widget::Id>, bounds: Rectangle) {
        if id == Some(&widget::Id::new("main-popover")) {
            self.popover = bounds;
        }
        if id == Some(&widget::Id::new("composer")) {
            self.composer = bounds;
        }
        if id == Some(&widget::Id::from(self.last_id.clone())) {
            self.last = bounds;
        }
    }
}
