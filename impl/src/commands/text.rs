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
    rl_insert_mode.set(1);
    unset_state(RL_STATE_OVERWRITE);
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

/// Insert `count` copies of `bytes` (always inserting).
pub fn insert_chars(bytes: &[u8], count: usize) {
    if count == 0 || bytes.is_empty() {
        return;
    }
    let mut text = Vec::with_capacity(bytes.len() * count);
    for _ in 0..count {
        text.extend_from_slice(bytes);
    }
    buffer::insert(&text);
}

/// Self-insert `count` copies of `bytes`, replacing characters in
/// overwrite mode (one undo group per command).
fn self_insert_chars(bytes: &[u8], count: usize) {
    if count == 0 || bytes.is_empty() {
        return;
    }
    if rl_insert_mode.get() != 0 {
        insert_chars(bytes, count);
        return;
    }
    unsafe {
        crate::undo::rl_begin_undo_group();
    }
    for _ in 0..count {
        buffer::insert(bytes);
        let p = point();
        if p < end() {
            buffer::delete(p, mb::next_char(line(), p));
        }
    }
    unsafe {
        crate::undo::rl_end_undo_group();
    }
}

/// Read the rest of a multibyte character whose first byte is `first`.
unsafe fn read_mb_rest(first: u8) -> Vec<u8> {
    let mut v = vec![first];
    if !mb::multibyte() || first < 0x80 {
        return v;
    }
    let from_macro = input::is_state(RL_STATE_MACROINPUT);
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
        // a character split across the end of macro text stays incomplete
        if from_macro && !crate::kbdmacro::macro_input_pending() {
            return v;
        }
        // the rest of the character is not recorded into a keyboard macro
        let defining = input::is_state(RL_STATE_MACRODEF);
        unset_state(RL_STATE_MACRODEF);
        let c = crate::input::rl_read_key();
        if defining {
            set_state(RL_STATE_MACRODEF);
        }
        if c < 0 {
            return v;
        }
        v.push(c as u8);
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_insert(count: c_int, key: c_int) -> c_int {
    self_insert(count, key, true)
}

/// self-insert; with `batch`, typeahead that is also self-insert is
/// inserted right away.
unsafe fn self_insert(count: c_int, key: c_int, batch: bool) -> c_int {
    if count <= 0 {
        return 0;
    }
    let ch = read_mb_rest(key as u8);
    if count > 1 {
        self_insert_chars(&ch, count as usize);
        return 0;
    }
    self_insert_chars(&ch, 1);
    // typeahead that is also self-insert is inserted right away; those
    // keys are not recorded into a keyboard macro being defined
    if batch && !input::is_state(RL_STATE_MACROINPUT) && rl_insert_mode.get() == 1 {
        let map = crate::keymap::current();
        let defining = input::is_state(RL_STATE_MACRODEF);
        unset_state(RL_STATE_MACRODEF);
        while input::input_available(0) {
            let c = crate::input::rl_read_key();
            if c < 0 {
                break;
            }
            let e = *map.add(c as usize & 0xff);
            if e.type_ == ISFUNC && crate::dispatch::same_fn(e.function.func, rl_insert) && c < 256 {
                let more = read_mb_rest(c as u8);
                self_insert_chars(&more, 1);
            } else {
                crate::input::rl_execute_next(c);
                break;
            }
        }
        if defining {
            set_state(RL_STATE_MACRODEF);
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_quoted_insert(count: c_int, _key: c_int) -> c_int {
    // the next character is taken literally, even a signal character
    let sigs = crate::tty::set_tty_signals(false);
    set_state(RL_STATE_MOREINPUT);
    let c = crate::input::rl_read_key();
    unset_state(RL_STATE_MOREINPUT);
    if sigs {
        crate::tty::set_tty_signals(true);
    }
    if c < 0 {
        return -1;
    }
    let ch = read_mb_rest(c as u8);
    // a negative count inserts the character once
    insert_chars(&ch, if count < 0 { 1 } else { count as usize });
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
    // overwrite mode (not at the end): the characters become spaces
    let overwrite = rl_insert_mode.get() == 0 && p < end();
    if overwrite {
        crate::undo::rl_begin_undo_group();
    }
    if count > 1 || crate::numarg::rl_explicit_arg.get() != 0 {
        crate::kill::rl_kill_text(from as c_int, p as c_int);
    } else {
        buffer::delete(from, p);
    }
    if overwrite {
        buffer::insert_at(from, &vec![b' '; count as usize]);
        crate::undo::rl_end_undo_group();
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
pub unsafe extern "C" fn rl_insert_comment(_count: c_int, key: c_int) -> c_int {
    let cb = crate::vars::comment_begin_bytes();
    set_point(0);
    if crate::numarg::rl_explicit_arg.get() != 0 && !cb.is_empty() && line().starts_with(&cb) {
        // with an argument the comment is toggled
        buffer::delete(0, cb.len());
    } else {
        buffer::insert(&cb);
    }
    crate::display::redisplay();
    crate::readline::rl_newline(1, key)
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

/// Move `n` words from `p` (negative: backward).
fn word_move(p: usize, n: c_int) -> usize {
    use crate::commands::movement::{backward_word_pos, forward_word_pos};
    if n >= 0 {
        forward_word_pos(p, n as usize)
    } else {
        backward_word_pos(p, n.unsigned_abs() as usize)
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_transpose_words(count: c_int, _key: c_int) -> c_int {
    if count == 0 {
        return 0;
    }
    let orig = point();
    let w2_end = word_move(orig, count);
    let w2_beg = word_move(w2_end, -1);
    let w1_beg = word_move(w2_beg, -count);
    let w1_end = word_move(w1_beg, 1);
    if w1_beg == w2_beg || w2_beg < w1_end {
        set_point(orig);
        rl_ding();
        return 1;
    }
    let l = line();
    let w1 = l[w1_beg..w1_end].to_vec();
    let w2 = l[w2_beg..w2_end].to_vec();
    crate::undo::rl_begin_undo_group();
    buffer::delete(w2_beg, w2_end);
    buffer::insert_at(w2_beg, &w1);
    buffer::delete(w1_beg, w1_end);
    buffer::insert_at(w1_beg, &w2);
    crate::undo::rl_end_undo_group();
    set_point(w2_end.min(end()));
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
        return 1;
    }
    let l = line();
    let mut p = point();
    if p == e {
        p = mb::prev_char(l, p);
    }
    // drag the character before point forward over `count` characters
    let a = mb::prev_char(l, p);
    let ch = l[a..p].to_vec();
    crate::undo::rl_begin_undo_group();
    buffer::delete(a, p);
    let l = line();
    let mut q = a;
    for _ in 0..count.max(0) {
        if q >= l.len() {
            break;
        }
        q = mb::next_char(l, q);
    }
    set_point(q);
    buffer::insert(&ch);
    crate::undo::rl_end_undo_group();
    0
}

/// Search for the next key's character `count` times (dir 1 / -1).
unsafe fn char_search(count: c_int, dir: i32) -> c_int {
    let Some(target) = read_char_arg() else { return -1 };
    if target.is_empty() {
        return 0;
    }
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

/// The opening bracket matching the closing one at `close` (same type,
/// nested; quoted strings are skipped).
fn find_matching_open(l: &[u8], close: usize) -> Option<usize> {
    let cl = l[close];
    let op = match cl {
        b')' => b'(',
        b']' => b'[',
        b'}' => b'{',
        _ => return None,
    };
    let mut depth = 1;
    let mut i = close;
    while i > 0 {
        i -= 1;
        let c = l[i];
        if c == b'"' || c == b'\'' {
            // skip back over the quoted string
            loop {
                if i == 0 {
                    return None;
                }
                i -= 1;
                if l[i] == c {
                    break;
                }
            }
        } else if c == op {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        } else if c == cl {
            depth += 1;
        }
    }
    None
}

#[no_mangle]
pub unsafe extern "C" fn rl_insert_close(count: c_int, key: c_int) -> c_int {
    if crate::numarg::rl_explicit_arg.get() != 0 || crate::vars::blink_matching_paren.get() == 0 {
        return rl_insert(count, key);
    }
    self_insert(1, key, false);
    crate::display::redisplay();
    let p = point();
    if p == 0 {
        return 0;
    }
    let Some(m) = find_matching_open(line(), p - 1) else {
        rl_ding();
        return 1;
    };
    // show the match until the timeout expires or input arrives
    set_point(m);
    crate::display::redisplay();
    input::input_available(PAREN_BLINK_TIMEOUT.get() as i64);
    set_point(p);
    0
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

/// Read the body of a bracketed paste (after `ESC [ 2 0 0 ~`) up to the
/// closing `ESC [ 2 0 1 ~`; CRs become newlines.
unsafe fn read_paste_body() -> Vec<u8> {
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
    for b in buf.iter_mut() {
        if *b == b'\r' {
            *b = b'\n';
        }
    }
    buf
}

/// After an ESC: if the rest of `ESC [ 2 0 0 ~` follows, read the pasted
/// text; otherwise push back what was read and return None.
unsafe fn try_read_paste() -> Option<Vec<u8>> {
    const START: &[u8] = b"[200~";
    if crate::vars::enable_bracketed_paste.get() == 0 {
        return None;
    }
    let mut got: Vec<u8> = Vec::new();
    while got.len() < START.len() {
        if !input::input_available(crate::input::KEYBOARD_INPUT_TIMEOUT.get() as i64) {
            break;
        }
        let c = crate::input::rl_read_key();
        if c < 0 {
            break;
        }
        got.push(c as u8);
        if START[got.len() - 1] != c as u8 {
            break;
        }
    }
    if got != START {
        input::unread_keys(&got);
        return None;
    }
    Some(read_paste_body())
}

/// After an ESC was read: the text of a bracketed paste that follows, or
/// None (the keys read are pushed back).
pub unsafe fn read_paste_after_esc() -> Option<Vec<u8>> {
    try_read_paste()
}

/// Read the character argument of a command (character-search, vi
/// replace/find, ...): one (multibyte) character.  A bracketed paste
/// counts as its first character and the rest of the pasted text is pushed
/// back as input.  None at end of input; an empty paste gives an empty
/// character.
pub unsafe fn read_char_arg() -> Option<Vec<u8>> {
    set_state(RL_STATE_MOREINPUT);
    let c = crate::input::rl_read_key();
    unset_state(RL_STATE_MOREINPUT);
    if c < 0 {
        return None;
    }
    if c == 0x1b {
        if let Some(text) = try_read_paste() {
            if text.is_empty() {
                return Some(text);
            }
            let n = mb::next_char(&text, 0).max(1);
            input::unread_keys(&text[n..]);
            return Some(text[..n].to_vec());
        }
    }
    Some(read_mb_rest(c as u8))
}

#[no_mangle]
pub unsafe extern "C" fn rl_bracketed_paste_begin(_count: c_int, _key: c_int) -> c_int {
    let buf = read_paste_body();
    let start = point();
    if !buf.is_empty() && crate::vi::in_replace_mode() {
        // vi replace mode: the pasted text overwrites
        crate::vi::overstrike_text(&buf);
    } else if !buf.is_empty() {
        buffer::insert(&buf);
    }
    buffer::rl_mark.set(start as c_int);
    rl_activate_mark();
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
