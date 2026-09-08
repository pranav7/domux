//! The confirmation overlay. Task 16 writes the project kind's copy and Task 18 the
//! workspace kinds'; this is the seam the overlay match needs.
//!
//! `ConfirmKind::CloseTab` reaches here too, and must stay drawing nothing: M1 asks that
//! question in the tab's own cell (`top_bar::draw`), so a box drawn here would ask it twice.

use crate::render::RenderInput;
use domux_core::model::ConfirmKind;
use ratatui::buffer::Buffer;

pub fn draw(_input: &RenderInput, _kind: &ConfirmKind, _buf: &mut Buffer) {}
