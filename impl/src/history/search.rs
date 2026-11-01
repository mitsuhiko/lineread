//! Searching the history list (history(3) "Searching the History List").
//! Owner: history workstream.
use super::{history_length, history_offset, line_at};
use crate::prelude::*;

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn rfind(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).rposition(|w| w == needle)
}

/// Search the list from `history_offset` for `s` (`anchored`: at the start
/// of the line).  On success `history_offset` is the entry and the result
/// the index within the line; else -1 and the offset is unchanged.
pub fn search_list(s: &[u8], direction: c_int, anchored: bool) -> c_int {
    let len = history_length.get();
    if s.is_empty() || len == 0 {
        return -1;
    }
    let mut i = history_offset.get();
    if direction < 0 {
        if i >= len {
            i = len - 1;
        }
    } else if i >= len {
        return -1;
    }
    while i >= 0 && i < len {
        if let Some(line) = line_at(i) {
            let hit = if anchored {
                if line.starts_with(s) {
                    Some(0)
                } else {
                    None
                }
            } else if direction < 0 {
                rfind(line, s)
            } else {
                find(line, s)
            };
            if let Some(off) = hit {
                history_offset.set(i);
                return off as c_int;
            }
        }
        if direction < 0 {
            i -= 1;
        } else {
            i += 1;
        }
    }
    -1
}

#[no_mangle]
pub unsafe extern "C" fn history_search(s: *const c_char, direction: c_int) -> c_int {
    search_list(cstr(s), direction, false)
}

#[no_mangle]
pub unsafe extern "C" fn history_search_prefix(s: *const c_char, direction: c_int) -> c_int {
    search_list(cstr(s), direction, true)
}

#[no_mangle]
pub unsafe extern "C" fn history_search_pos(s: *const c_char, direction: c_int, pos: c_int) -> c_int {
    let old = history_offset.get();
    super::history_set_pos(pos);
    let r = search_list(cstr(s), direction, false);
    let found = history_offset.get();
    history_offset.set(old);
    if r == -1 {
        -1
    } else {
        found
    }
}
