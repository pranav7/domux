//! The name box (`leader N`). Task 15 draws it; this is the seam the overlay match needs.

use crate::render::RenderInput;
use domux_core::ids::WorkspaceId;
use ratatui::buffer::Buffer;

pub fn draw(_input: &RenderInput, _workspace: &WorkspaceId, _buf: &mut Buffer) {}
