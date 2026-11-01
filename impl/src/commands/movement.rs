//! Cursor movement commands (emacs): characters, bytes, words, line ends.
//! Owner: emacs workstream.
use crate::buffer::{self, end, line, point, set_point};
use crate::mb;
use crate::prelude::*;
use crate::terminal::rl_ding;

/// Last position point may take in the current mode (vi command mode
/// cannot sit after the last character).
fn max_point() -> usize {
    let e = end();
    if crate::vi::in_command_mode() && e > 0 {
        mb::prev_char(line(), e)
    } else {
        e
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_forward_byte(count: c_int, key: c_int) -> c_int {
    if count < 0 {
        return rl_backward_byte(-count, key);
    }
    let p = point() + count as usize;
    let m = max_point();
    if p > m {
        set_point(m);
        rl_ding();
        return 0;
    }
    set_point(p);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_backward_byte(count: c_int, key: c_int) -> c_int {
    if count < 0 {
        return rl_forward_byte(-count, key);
    }
    let p = point();
    if (count as usize) > p {
        set_point(0);
        rl_ding();
        return 0;
    }
    set_point(p - count as usize);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_forward_char(count: c_int, key: c_int) -> c_int {
    if !mb::multibyte() {
        return rl_forward_byte(count, key);
    }
    if count < 0 {
        return rl_backward_char(-count, key);
    }
    let l = line();
    let m = max_point();
    let mut p = point();
    for _ in 0..count {
        if p >= m {
            set_point(m);
            rl_ding();
            return 0;
        }
        p = mb::forward_chars_nz(l, p, 1);
    }
    set_point(p.min(m));
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_backward_char(count: c_int, key: c_int) -> c_int {
    if !mb::multibyte() {
        return rl_backward_byte(count, key);
    }
    if count < 0 {
        return rl_forward_char(-count, key);
    }
    let l = line();
    let mut p = point();
    for _ in 0..count {
        if p == 0 {
            rl_ding();
            break;
        }
        p = mb::prev_char_nz(l, p);
    }
    set_point(p);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_forward(count: c_int, key: c_int) -> c_int {
    rl_forward_char(count, key)
}

#[no_mangle]
pub unsafe extern "C" fn rl_backward(count: c_int, key: c_int) -> c_int {
    rl_backward_char(count, key)
}

#[no_mangle]
pub unsafe extern "C" fn rl_beg_of_line(_count: c_int, _key: c_int) -> c_int {
    set_point(0);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_end_of_line(_count: c_int, _key: c_int) -> c_int {
    set_point(end());
    0
}

/// Position after moving `count` words forward from `p` (emacs words).
pub fn forward_word_pos(mut p: usize, count: usize) -> usize {
    let l = line();
    let e = l.len();
    for _ in 0..count {
        if p >= e {
            break;
        }
        // skip non-word characters, then the word
        while p < e && !mb::is_word_char(l, p) {
            p = mb::next_char(l, p);
        }
        while p < e && mb::is_word_char(l, p) {
            p = mb::next_char(l, p);
        }
    }
    p
}

/// Position after moving `count` words backward from `p`.
pub fn backward_word_pos(mut p: usize, count: usize) -> usize {
    let l = line();
    for _ in 0..count {
        if p == 0 {
            break;
        }
        // skip non-word characters before point, then the word
        loop {
            if p == 0 {
                break;
            }
            let q = mb::prev_char(l, p);
            if mb::is_word_char(l, q) {
                break;
            }
            p = q;
        }
        loop {
            if p == 0 {
                break;
            }
            let q = mb::prev_char(l, p);
            if !mb::is_word_char(l, q) {
                break;
            }
            p = q;
        }
    }
    p
}

#[no_mangle]
pub unsafe extern "C" fn rl_forward_word(count: c_int, key: c_int) -> c_int {
    if count < 0 {
        return rl_backward_word(-count, key);
    }
    set_point(forward_word_pos(point(), count as usize));
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_backward_word(count: c_int, key: c_int) -> c_int {
    if count < 0 {
        return rl_forward_word(-count, key);
    }
    set_point(backward_word_pos(point(), count as usize));
    let _ = buffer::end();
    0
}

/// arrow-key-prefix: the next key selects the direction (A up, B down,
/// C right, D left; either case).
#[no_mangle]
pub unsafe extern "C" fn rl_arrow_keys(count: c_int, _key: c_int) -> c_int {
    crate::input::set_state(RL_STATE_MOREINPUT);
    let c = crate::input::rl_read_key();
    crate::input::unset_state(RL_STATE_MOREINPUT);
    if c < 0 {
        return -1;
    }
    match (c as u8).to_ascii_uppercase() {
        b'A' => crate::histcmds::rl_get_previous_history(count, c),
        b'B' => crate::histcmds::rl_get_next_history(count, c),
        b'C' => rl_forward_char(count, c),
        b'D' => rl_backward_char(count, c),
        _ => {
            rl_ding();
            0
        }
    }
}

/// Screen position (row, column) of buffer index `i` in the current layout.
fn screen_pos(i: usize) -> (usize, usize) {
    let saved = buffer::rl_point.get();
    buffer::rl_point.set(i as c_int);
    let (_, pos) = crate::display::layout();
    buffer::rl_point.set(saved);
    pos
}

/// The last character boundary whose screen position is at or before
/// (row, col).
fn index_at(row: usize, col: usize) -> usize {
    let l = line();
    let mut best = 0;
    let mut i = 0;
    loop {
        if screen_pos(i) <= (row, col) {
            best = i;
        } else {
            break;
        }
        if i >= l.len() {
            break;
        }
        i = mb::next_char(l, i);
    }
    best
}

/// Move to the same column on the previous screen line (the start of
/// the line from the first one).  The count is ignored (observed).
#[no_mangle]
pub unsafe extern "C" fn rl_previous_screen_line(_count: c_int, _key: c_int) -> c_int {
    let (row, col) = screen_pos(point());
    if row == 0 {
        set_point(0);
        return 0;
    }
    set_point(index_at(row - 1, col));
    0
}

/// Move to the same column on the next screen line (the end of the line
/// from the last one).  The count is ignored (observed).
#[no_mangle]
pub unsafe extern "C" fn rl_next_screen_line(_count: c_int, _key: c_int) -> c_int {
    let (row, col) = screen_pos(point());
    let (last_row, _) = screen_pos(end());
    if row >= last_row {
        set_point(end());
        return 0;
    }
    set_point(index_at(row + 1, col));
    0
}
