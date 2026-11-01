//! Text-changing commands (emacs): self-insert, quoted-insert, deletion,
//! case changes, transposition, character search, mark, overwrite mode,
//! bracketed paste, the active region.
//! Owner: emacs workstream.
use crate::buffer::{self, end, line, point, set_point};
use crate::input::{self, set_state, unset_state};
use crate::mb;
use crate::prelude::*;
use crate::terminal::rl_ding;

/// 1 = insert, 0 = overwrite
#[no_mangle]
pub static rl_insert_mode: CVar<c_int> = CVar::new(1);

static MARK_ACTIVE: CVar<bool> = CVar::new(false);
static KEEP_MARK_ACTIVE: CVar<bool> = CVar::new(false);
static PAREN_BLINK_TIMEOUT: CVar<c_int> = CVar::new(500_000);

/// Called by rl_initialize for every new line.
pub fn reset_per_line() {
    MARK_ACTIVE.set(false);
    KEEP_MARK_ACTIVE.set(false);
}

/// Called after every dispatched command (the active region is deactivated
/// unless the command asked to keep it).
pub fn after_command() {
    if KEEP_MARK_ACTIVE.replace(false) {
        return;
    }
    MARK_ACTIVE.set(false);
}

/// Insert `count` copies of `bytes` honouring overwrite mode.
pub fn insert_chars(bytes: &[u8], count: usize) {
    if count == 0 || bytes.is_empty() {
        return;
    }
    let mut text = Vec::with_capacity(bytes.len() * count);
    for _ in 0..count {
        text.extend_from_slice(bytes);
    }
    if rl_insert_mode.get() == 0 {
        // overwrite: replace as many characters as we insert
        let nchars = count;
        let p = point();
        let l = line();
        let to = mb::forward_chars(l, p, nchars);
        if to > p {
            unsafe {
                crate::undo::rl_begin_undo_group();
            }
            buffer::delete(p, to);
            buffer::insert(&text);
            unsafe {
                crate::undo::rl_end_undo_group();
            }
            return;
        }
    }
    buffer::insert(&text);
}

/// Read the rest of a multibyte character whose first byte is `first`.
unsafe fn read_mb_rest(first: u8) -> Vec<u8> {
    let mut v = vec![first];
    if !mb::multibyte() || first < 0x80 {
        return v;
    }
    loop {
        let (wc, n) = mb::decode_at(&v, 0);
        if wc.is_some() && n == v.len() {
            return v;
        }
        // incomplete?  read more only if the prefix can still be valid
        let mut st = crate::util::MbState::new();
        let mut w: libc::wchar_t = 0;
        let r = crate::util::mbrtowc(&mut w, v.as_ptr() as *const c_char, v.len(), &mut st);
        if r != usize::MAX - 1 || v.len() >= 6 {
            return v;
        }
        if !input::input_available(0) && v.len() > 1 && false {
            return v;
        }
        let c = crate::input::rl_read_key();
        if c < 0 {
            return v;
        }
        v.push(c as u8);
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_insert(count: c_int, key: c_int) -> c_int {
    if count <= 0 {
        return 0;
    }
    let ch = read_mb_rest(key as u8);
    if count > 1 {
        insert_chars(&ch, count as usize);
        return 0;
    }
    // batch typeahead that is also self-insert
    let mut text = ch;
    if !crate::kbdmacro::macro_input_pending() && rl_insert_mode.get() == 1 {
        let map = crate::keymap::current();
        while input::input_available(0) {
            let c = crate::input::rl_read_key();
            if c < 0 {
                break;
            }
            let e = *map.add(c as usize & 0xff);
            if e.type_ == ISFUNC && crate::dispatch::same_fn(e.function.func, rl_insert) && c < 256 {
                let more = read_mb_rest(c as u8);
                text.extend_from_slice(&more);
            } else {
                crate::input::rl_execute_next(c);
                break;
            }
        }
    }
    insert_chars(&text, 1);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_quoted_insert(count: c_int, _key: c_int) -> c_int {
    set_state(RL_STATE_MOREINPUT);
    let c = crate::input::rl_read_key();
    unset_state(RL_STATE_MOREINPUT);
    if c < 0 {
        return -1;
    }
    let ch = read_mb_rest(c as u8);
    if count < 0 {
        return 0;
    }
    insert_chars(&ch, count.max(1) as usize);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_tab_insert(count: c_int, _key: c_int) -> c_int {
    insert_chars(b"\t", count.max(0) as usize);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_rubout(count: c_int, key: c_int) -> c_int {
    if count < 0 {
        return rl_delete(-count, key);
    }
    let p = point();
    if p == 0 {
        rl_ding();
        return -1;
    }
    let l = line();
    let from = mb::backward_chars(l, p, count as usize);
    if count > 1 || crate::numarg::rl_explicit_arg.get() != 0 {
        crate::kill::rl_kill_text(from as c_int, p as c_int);
    } else {
        buffer::delete(from, p);
    }
    set_point(from);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_delete(count: c_int, key: c_int) -> c_int {
    if count < 0 {
        return rl_rubout(-count, key);
    }
    let p = point();
    if eof_on_empty_line(key) && p == 0 && crate::numarg::rl_explicit_arg.get() == 0 {
        crate::readline::signal_eof();
        return 0;
    }
    if p >= end() {
        rl_ding();
        return -1;
    }
    let l = line();
    let to = mb::forward_chars(l, p, count as usize);
    if count > 1 || crate::numarg::rl_explicit_arg.get() != 0 {
        crate::kill::rl_kill_text(p as c_int, to as c_int);
    } else {
        buffer::delete(p, to);
    }
    set_point(p);
    0
}

/// Is `key` the terminal's EOF character and the line empty (delete-char
/// then means end of file)?
pub fn eof_on_empty_line(key: c_int) -> bool {
    end() == 0 && key == crate::tty::SPECIALS.get().eof as c_int
}

#[no_mangle]
pub unsafe extern "C" fn rl_rubout_or_delete(count: c_int, key: c_int) -> c_int {
    if end() != 0 && point() == end() {
        rl_rubout(count, key)
    } else {
        rl_delete(count, key)
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_delete_horizontal_space(_count: c_int, _key: c_int) -> c_int {
    let l = line();
    let mut a = point();
    let mut b = point();
    while a > 0 && (l[a - 1] == b' ' || l[a - 1] == b'\t') {
        a -= 1;
    }
    while b < l.len() && (l[b] == b' ' || l[b] == b'\t') {
        b += 1;
    }
    if a != b {
        buffer::delete(a, b);
    }
    set_point(a);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_insert_comment(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_insert_comment");
    0
}

/// Change the case of `count` words: 0 = down, 1 = up, 2 = capitalize.
unsafe fn change_case(count: c_int, op: u8) -> c_int {
    let start = point();
    let stop = if count < 0 {
        crate::commands::movement::backward_word_pos(start, (-count) as usize)
    } else {
        crate::commands::movement::forward_word_pos(start, count as usize)
    };
    let (a, b) = if start < stop { (start, stop) } else { (stop, start) };
    let l = line();
    let mut out = Vec::with_capacity(b - a);
    let mut in_word = false;
    let mut i = a;
    while i < b {
        let (wc, n) = mb::decode_at(l, i);
        let is_w = mb::is_word_char(l, i);
        let want_up = match op {
            1 => true,
            0 => false,
            _ => !in_word,
        };
        match wc {
            Some(c) if is_w => {
                let nc = if want_up { crate::util::towupper(c) } else { crate::util::towlower(c) };
                if n == 1 {
                    out.push(nc as u8);
                } else {
                    let mut buf = [0u8; 16];
                    let mut st = crate::util::MbState::new();
                    let r = crate::util::wcrtomb(buf.as_mut_ptr() as *mut c_char, nc as libc::wchar_t, &mut st);
                    if r == usize::MAX || r == 0 {
                        out.extend_from_slice(&l[i..i + n]);
                    } else {
                        out.extend_from_slice(&buf[..r]);
                    }
                }
            }
            _ => out.extend_from_slice(&l[i..i + n]),
        }
        in_word = is_w;
        i += n;
    }
    if out != l[a..b] {
        crate::undo::rl_modifying(a as c_int, b as c_int);
        let mut nl = l[..a].to_vec();
        nl.extend_from_slice(&out);
        nl.extend_from_slice(&l[b..]);
        crate::input::set_state(RL_STATE_UNDOING);
        buffer::set_line(&nl);
        crate::input::unset_state(RL_STATE_UNDOING);
    }
    set_point(stop.max(start).min(end()));
    if count < 0 {
        set_point(start);
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_upcase_word(count: c_int, _key: c_int) -> c_int {
    change_case(count, 1)
}

#[no_mangle]
pub unsafe extern "C" fn rl_downcase_word(count: c_int, _key: c_int) -> c_int {
    change_case(count, 0)
}

#[no_mangle]
pub unsafe extern "C" fn rl_capitalize_word(count: c_int, _key: c_int) -> c_int {
    change_case(count, 2)
}

#[no_mangle]
pub unsafe extern "C" fn rl_transpose_words(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_transpose_words");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_transpose_chars(count: c_int, _key: c_int) -> c_int {
    if count == 0 {
        return 0;
    }
    let e = end();
    if point() == 0 || e < 2 {
        rl_ding();
        return -1;
    }
    let l = line();
    let mut p = point();
    if p == e {
        p = mb::prev_char(l, p);
    }
    let a = mb::prev_char(l, p);
    let b = mb::next_char(l, p);
    let first = l[a..p].to_vec();
    let second = l[p..b].to_vec();
    crate::undo::rl_begin_undo_group();
    buffer::delete(a, b);
    set_point(a);
    buffer::insert(&second);
    buffer::insert(&first);
    crate::undo::rl_end_undo_group();
    // with a count, drag the character further
    let np = point();
    if count > 1 {
        set_point(np);
        return rl_transpose_chars(count - 1, _key);
    }
    0
}

/// Search for the next key's character `count` times (dir 1 / -1).
unsafe fn char_search(count: c_int, dir: i32) -> c_int {
    set_state(RL_STATE_MOREINPUT);
    let c = crate::input::rl_read_key();
    unset_state(RL_STATE_MOREINPUT);
    if c < 0 {
        return -1;
    }
    let target = read_mb_rest(c as u8);
    let (count, dir) = if count < 0 { (-count, -dir) } else { (count, dir) };
    let l = line();
    let mut p = point();
    for _ in 0..count {
        let mut found = None;
        if dir > 0 {
            let mut i = mb::next_char(l, p);
            while i < l.len() {
                if l[i..].starts_with(&target) {
                    found = Some(i);
                    break;
                }
                i = mb::next_char(l, i);
            }
        } else {
            let mut i = p;
            while i > 0 {
                i = mb::prev_char(l, i);
                if l[i..].starts_with(&target) {
                    found = Some(i);
                    break;
                }
            }
        }
        match found {
            Some(i) => p = i,
            None => {
                rl_ding();
                return -1;
            }
        }
    }
    set_point(p);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_char_search(count: c_int, _key: c_int) -> c_int {
    char_search(count, 1)
}

#[no_mangle]
pub unsafe extern "C" fn rl_backward_char_search(count: c_int, _key: c_int) -> c_int {
    char_search(count, -1)
}

#[no_mangle]
pub unsafe extern "C" fn rl_set_mark(_count: c_int, _key: c_int) -> c_int {
    let p = if crate::numarg::rl_explicit_arg.get() != 0 {
        crate::numarg::count()
    } else {
        point() as c_int
    };
    if p < 0 || p > end() as c_int {
        return 1;
    }
    buffer::rl_mark.set(p);
    rl_activate_mark();
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_exchange_point_and_mark(_count: c_int, _key: c_int) -> c_int {
    let m = buffer::rl_mark.get();
    if m > end() as c_int || m < 0 {
        buffer::rl_mark.set(0);
        rl_ding();
        return -1;
    }
    let p = buffer::rl_point.get();
    buffer::rl_point.set(m);
    buffer::rl_mark.set(p);
    rl_activate_mark();
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_overwrite_mode(count: c_int, _key: c_int) -> c_int {
    if crate::numarg::rl_explicit_arg.get() == 0 {
        rl_insert_mode.set(if rl_insert_mode.get() == 0 { 1 } else { 0 });
    } else if count > 0 {
        rl_insert_mode.set(0);
    } else {
        rl_insert_mode.set(1);
    }
    if rl_insert_mode.get() == 0 {
        set_state(RL_STATE_OVERWRITE);
    } else {
        unset_state(RL_STATE_OVERWRITE);
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_insert_close(count: c_int, key: c_int) -> c_int {
    // TODO(display): blink-matching-paren
    rl_insert(count, key)
}

#[no_mangle]
pub unsafe extern "C" fn rl_set_paren_blink_timeout(u: c_int) -> c_int {
    let old = PAREN_BLINK_TIMEOUT.get();
    if u > 0 {
        PAREN_BLINK_TIMEOUT.set(u);
    }
    old
}

#[no_mangle]
pub unsafe extern "C" fn rl_alphabetic(c: c_int) -> c_int {
    if (0..128).contains(&c) {
        return (c as u8).is_ascii_alphanumeric() as c_int;
    }
    if (0..256).contains(&c) {
        return (libc::isalnum(c) != 0) as c_int;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_bracketed_paste_begin(_count: c_int, _key: c_int) -> c_int {
    // read until ESC [ 2 0 1 ~
    const END: &[u8] = b"\x1b[201~";
    let mut buf: Vec<u8> = Vec::new();
    set_state(RL_STATE_MOREINPUT);
    loop {
        let c = crate::input::rl_read_key();
        if c < 0 {
            break;
        }
        buf.push(c as u8);
        if buf.ends_with(END) {
            buf.truncate(buf.len() - END.len());
            break;
        }
    }
    unset_state(RL_STATE_MOREINPUT);
    // the pasted text is inserted literally (CRs become newlines)
    for b in buf.iter_mut() {
        if *b == b'\r' {
            *b = b'\n';
        }
    }
    if !buf.is_empty() {
        buffer::insert(&buf);
        rl_activate_mark();
        buffer::rl_mark.set((point() - buf.len()) as c_int);
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_tilde_expand(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_tilde_expand");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_keep_mark_active() {
    KEEP_MARK_ACTIVE.set(true);
}

#[no_mangle]
pub unsafe extern "C" fn rl_activate_mark() {
    if crate::vars::enable_active_region.get() != 0 {
        MARK_ACTIVE.set(true);
        KEEP_MARK_ACTIVE.set(true);
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_deactivate_mark() {
    MARK_ACTIVE.set(false);
}

#[no_mangle]
pub unsafe extern "C" fn rl_mark_active_p() -> c_int {
    MARK_ACTIVE.get() as c_int
}
