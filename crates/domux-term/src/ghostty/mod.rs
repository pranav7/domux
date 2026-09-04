//! libghostty-vt behind the `Emulator` trait. Task 6 fills this in.

pub mod ffi;

#[cfg(test)]
mod tests {
    use super::ffi;

    #[test]
    fn library_links_and_reports_a_codepoint_width() {
        // One call through the static library proves the Zig build and link flags.
        let w = unsafe { ffi::ghostty_unicode_codepoint_width('漢' as u32) };
        assert_eq!(w, 2);
    }
}
