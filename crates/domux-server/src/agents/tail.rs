//! A JSONL file an agent appends to, read forward from a byte cursor. Two readers share it: the
//! transcript reader in `agents::recap`, and the Codex session index reader in
//! `agents::session_index`. Each says what an entry means; this says which bytes are entries it
//! has not read yet.
//!
//! Both files are appended to and never edited in place, so the bytes behind the cursor cannot
//! change and re-reading them would answer what they answered before. That is what lets the
//! core ask once a second: the question costs one `stat` while the file sits still, and a few
//! kilobytes while the agent writes, rather than the whole file either way (MUX-28).

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// How much of a file domux has never seen before is read: enough to reach the last recap and
/// the last checkpoint of a transcript, both of which sit at the end. Everything after the
/// first read is the bytes the agent appended, however large the file has grown.
pub const TAIL_BYTES: u64 = 2 * 1024 * 1024;

/// What a reader keeps from the entries it has read. Fed whole lines only, in file order, and
/// started again from `Default` when the file turns out to be a different one.
pub trait Entries: Default {
    fn feed(&mut self, text: &str);
}

/// One file being read: the cursor, and what the entries behind it said.
#[derive(Default)]
pub struct Tail<T> {
    /// The byte the next read starts at, always on a line boundary.
    read_to: u64,
    entries: T,
}

impl<T: Entries> Tail<T> {
    /// Reads whatever the agent has appended since last time, and answers with what every
    /// entry behind the cursor said. `None` when the file is not there to read, which leaves
    /// what was read before in place.
    ///
    /// A file shorter than the cursor is not the file the cursor was counting, so the reading
    /// starts again from nothing.
    pub fn read(&mut self, path: &Path) -> Option<&T> {
        let Ok(meta) = std::fs::metadata(path) else {
            return None;
        };
        let len = meta.len();
        if len < self.read_to {
            *self = Tail::default();
        }
        if len == self.read_to {
            return Some(&self.entries);
        }
        if !take(self, path, len) {
            // Longer than the cursor, and still not the file the cursor was counting.
            *self = Tail::default();
            take(self, path, len);
        }
        Some(&self.entries)
    }
}

/// Reads what the agent has written past the cursor, and answers whether it was reading the
/// file it thought it was.
///
/// The byte before the cursor is the newline the last read stopped on, so a file that does not
/// have one there was replaced rather than appended to. A shorter file gives itself away by its
/// length; one replaced by something longer would otherwise be read from the middle of a line
/// for the rest of its life.
fn take<T: Entries>(tail: &mut Tail<T>, path: &Path, len: u64) -> bool {
    // A file domux has never seen may already be hours long, and what a reader wants from it
    // is at the end.
    let first = tail.read_to == 0;
    let from = if first && len > TAIL_BYTES {
        len - TAIL_BYTES
    } else {
        tail.read_to
    };
    let base = if first { from } else { from - 1 };
    let Ok(mut file) = std::fs::File::open(path) else {
        return true;
    };
    if file.seek(SeekFrom::Start(base)).is_err() {
        return true;
    }
    let mut bytes = Vec::new();
    if file.read_to_end(&mut bytes).is_err() {
        return true;
    }
    // Where the entries this read has not seen begin.
    let start = if !first {
        if bytes.first() != Some(&b'\n') {
            return false;
        }
        1
    } else if from > 0 {
        // Starting partway into the file lands partway into a line, and half an entry is not
        // one. Only the first read of a long file starts anywhere but a line boundary.
        match bytes.iter().position(|b| *b == b'\n') {
            Some(i) => i + 1,
            None => return true,
        }
    } else {
        0
    };
    // The agent may be midway through writing the last line. Consuming to the last newline
    // leaves that line for the next read, which is when it will be whole.
    let Some(rel) = bytes[start..].iter().rposition(|b| *b == b'\n') else {
        return true;
    };
    let consumed = start + rel + 1;
    // Lossily, because a file half-written by a crashed process can carry one invalid byte, and
    // a good entry in an earlier line must survive it. Cutting on newlines never splits a
    // character: no byte of a multi-byte character is a newline.
    tail.entries
        .feed(&String::from_utf8_lossy(&bytes[start..consumed]));
    tail.read_to = base + consumed as u64;
    true
}
