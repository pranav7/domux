//! Composes one client's frame. Task 15 adds `compose`; the primitives live in submodules.

pub mod boxed;
pub mod pane_box;
pub mod theme;

use ratatui::layout::Rect;

pub fn to_rect(r: domux_core::model::Rect) -> Rect {
    Rect::new(r.x, r.y, r.width, r.height)
}
