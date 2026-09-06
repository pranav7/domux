//! The Model: the root of all shared state, owned by the core task.

pub mod focus;
pub mod layout;

pub use focus::{ConfirmKind, Focus, Overlay, PromptKind, RegionKind, TextInput};
pub use layout::{Direction, LayoutNode, Pane, PaneContent, Rect, SplitDir};
