//! vi mode: insert/command modes, motions, operators (d c y), counts,
//! redo (`.`), undo groups for insertions, marks, replace (overstrike)
//! mode, character search, vi-specific completion/history helpers.
//! Owner: V workstream.
//!
//! Model (observed from the reference):
//! * A line starts in insert mode without an undo group.  Commands that
//!   enter insert mode (`i a I A c s S R`, ...) open an undo group that the
//!   return to command mode closes, so `u` undoes a whole insertion.  The
//!   first switch to command mode of a line frees the undo list.
//! * The text of the last insertion (the newest UNDO_INSERT record when
//!   leaving insert mode) is kept for `.`.
//! * Operators read a motion (optionally with its own count), run the
//!   command bound to it in the vi command keymap, and act on the range
//!   between the old and the new point; `e E f t ; , %` include the
//!   character under the new point when moving forward.  The mark is left
//!   at the far end of the range (clamped by the deletion).
//! * After every command dispatched in command mode, point may not sit
//!   after the last character.
use crate::buffer::{self, end, line, point, set_point};
use crate::input::{is_state, set_state, unset_state};
use crate::mb;
use crate::prelude::*;
use crate::terminal::rl_ding;

const ESC: c_int = 27;

struct ViState {
    /// Key of the last text-modifying command (for `.`), 0 = none.
    last_cmd: c_int,
    /// Its count.
    last_count: c_int,
    /// The motion of the last operator.
    last_motion: c_int,
    /// The character argument of the last `r` / operator char search.
    last_char: Vec<u8>,
    /// Text of the last insertion.
    insert_text: Vec<u8>,
    /// An insertion started by a command is in progress (undo groups open).
    doing_insert: bool,
    /// Undo groups to close when leaving insert mode.
    open_groups: c_int,
    /// Replace (overstrike) mode is active.
    overstrike: bool,
    /// Characters typed in replace mode that backspace can undo.
    replaced: c_int,
    /// Characters typed in replace mode (for `.`).
    ostext: Vec<u8>,
    /// Last character search: key (f F t T) and character.
    cs_key: c_int,
    cs_char: Vec<u8>,
    /// Marks a-z (-1 = unset).
    marks: [c_int; 26],
    /// The replace-mode keymap (created on first use).
    replace_map: Keymap,
}

static V: CVar<ViState> = CVar::new(ViState {
    last_cmd: b'i' as c_int,
    last_count: 1,
    last_motion: 0,
    last_char: Vec::new(),
    insert_text: Vec::new(),
    doing_insert: false,
    open_groups: 0,
    overstrike: false,
    replaced: 0,
    ostext: Vec::new(),
    cs_key: 0,
    cs_char: Vec::new(),
    marks: [-1; 26],
    replace_map: null_mut(),
});

/// `.` is replaying the last command.
static REDOING: CVar<bool> = CVar::new(false);
/// Skip the end-of-line check after the current command (it aborted).
static SKIP_CHECK: CVar<bool> = CVar::new(false);

fn st() -> &'static mut ViState {
    V.get_mut()
}

fn redoing() -> bool {
    REDOING.get()
}

// ---------------------------------------------------------------- helpers

/// ESC followed by an unbound key in vi command mode: nothing.
pub unsafe extern "C" fn vi_esc_noop(_count: c_int, _key: c_int) -> c_int {
    0
}

fn vi_mode() -> bool {
    crate::readline::rl_editing_mode.get() == crate::readline::VI_MODE
}

/// Is the editor in vi command (movement) mode?  False while an operator
/// executes its motion (motions may then reach the end of the line).
pub fn in_command_mode() -> bool {
    vi_mode() && crate::keymap::current() == crate::keymap::vi_movement() && !is_state(RL_STATE_VIMOTION)
}

/// Is vi replace (overstrike) mode active?
pub fn in_replace_mode() -> bool {
    vi_mode() && st().overstrike && !st().replace_map.is_null() && crate::keymap::current() == st().replace_map
}

fn is_ws(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

/// vi word classes by byte: 0 blank, 1 identifier (alnum, _), 2 other.
fn class_at(l: &[u8], i: usize) -> u8 {
    let b = l[i];
    if is_ws(b) {
        0
    } else if b < 0x80 && (b.is_ascii_alphanumeric() || b == b'_') {
        1
    } else {
        2
    }
}

fn ws_at(l: &[u8], i: usize) -> bool {
    is_ws(l[i])
}

fn upper(key: c_int) -> bool {
    (b'A' as c_int..=b'Z' as c_int).contains(&key)
}

unsafe fn read_raw_key() -> c_int {
    set_state(RL_STATE_MOREINPUT);
    let c = crate::input::rl_read_key();
    unset_state(RL_STATE_MOREINPUT);
    c
}

/// Read the rest of a multibyte character that starts with `first`.
unsafe fn read_mb_rest(first: u8) -> Vec<u8> {
    let mut v = vec![first];
    if !mb::multibyte() || first < 0x80 {
        return v;
    }
    loop {
        let mut st = crate::util::MbState::new();
        let mut w: libc::wchar_t = 0;
        let r = crate::util::mbrtowc(&mut w, v.as_ptr() as *const c_char, v.len(), &mut st);
        if r != usize::MAX - 1 || v.len() >= 6 {
            return v;
        }
        let c = read_raw_key();
        if c < 0 {
            return v;
        }
        v.push(c as u8);
    }
}

/// Read a character argument (r, f, t, m, `): None on ESC / EOF.
unsafe fn read_char_arg() -> Option<Vec<u8>> {
    let c = read_raw_key();
    if c < 0 {
        return None;
    }
    if c == ESC {
        // cancels the command; the ESC itself is then read as a key
        crate::input::rl_execute_next(ESC);
        return None;
    }
    Some(read_mb_rest(c as u8))
}

/// In callback mode the reference reports a failed character command
/// (f F t T m ` without a usable character) as end of file: the EOF state
/// and rl_eof_found are set (the line itself goes on).
fn callback_char_fail() {
    if is_state(RL_STATE_CALLBACK) && !is_state(RL_STATE_VIMOTION) {
        set_state(RL_STATE_EOF);
        crate::readline::rl_eof_found.set(1);
    }
}

fn set_keymap(m: Keymap) {
    unsafe { crate::keymap::rl_set_keymap(m) };
}

/// Remember the last text-modifying command for `.`.
fn set_last(key: c_int, count: c_int) {
    if redoing() {
        return;
    }
    let s = st();
    s.last_cmd = key;
    s.last_count = count;
}

unsafe fn kill_range(a: usize, b: usize) {
    crate::kill::start_new_kill();
    crate::kill::rl_kill_text(a as c_int, b as c_int);
}

/// Back point up onto the last character (leaving insert mode, after puts).
fn back_one() {
    let p = point();
    if p > 0 {
        set_point(mb::prev_char(line(), p));
    }
}

// ---------------------------------------------------------------- line hooks

/// readline() starts every line in vi insert mode.
pub fn start_line_in_vi() {
    set_keymap(crate::keymap::vi_insertion());
    unset_state(RL_STATE_VICMDONCE);
    let s = st();
    s.doing_insert = false;
    s.open_groups = 0;
    s.overstrike = false;
    s.replaced = 0;
    s.marks = [-1; 26];
}

/// accept-line in vi mode.
pub fn newline_hook() {
    let s = st();
    s.doing_insert = false;
    s.open_groups = 0;
    s.overstrike = false;
}

/// Called after every command dispatched by the key loop: in command mode
/// point may not stay after the last character.
pub fn after_command() {
    if SKIP_CHECK.replace(false) {
        return;
    }
    if in_command_mode() {
        let e = end();
        if e > 0 && point() >= e {
            set_point(mb::prev_char(line(), e));
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_check() -> c_int {
    let e = end();
    if e > 0 && point() >= e {
        set_point(mb::prev_char(line(), e));
    }
    0
}

// ---------------------------------------------------------------- modes

/// Leaving insert mode: close the insertion's undo groups and keep its text.
fn done_inserting() {
    let s = st();
    let top = crate::undo::rl_undo_list.get();
    let top_insert = unsafe {
        if !top.is_null() && (*top).what == UNDO_INSERT {
            let l = line();
            let a = ((*top).start.max(0) as usize).min(l.len());
            let b = ((*top).end.max(0) as usize).min(l.len()).max(a);
            Some(l[a..b].to_vec())
        } else {
            None
        }
    };
    if s.doing_insert {
        // c s S C R: the text typed replaces the saved insertion
        if s.overstrike {
            if !s.ostext.is_empty() {
                s.insert_text = s.ostext.clone();
            }
        } else {
            s.insert_text = top_insert.unwrap_or_default();
        }
    } else if matches!(s.last_cmd as u8, b'i' | b'a' | b'I' | b'A') {
        s.insert_text = top_insert.unwrap_or_default();
    }
    if matches!(s.last_cmd as u8, b'i' | b'a' | b'I' | b'A' | b'R') {
        // a count given to an insertion is not repeated by `.`
        s.last_count = 1;
    }
    for _ in 0..s.open_groups {
        unsafe { crate::undo::rl_end_undo_group() };
    }
    s.open_groups = 0;
    s.doing_insert = false;
    s.overstrike = false;
    s.replaced = 0;
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_movement_mode(_count: c_int, key: c_int) -> c_int {
    if key == ESC && in_replace_mode() {
        // ESC is not a key-sequence prefix in replace mode: a bracketed
        // paste is recognised here
        if let Some(text) = crate::commands::text::read_paste_after_esc() {
            overstrike_text(&text);
            return 0;
        }
    }
    if point() > 0 {
        set_point(mb::prev_char(line(), point()));
    }
    set_keymap(crate::keymap::vi_movement());
    done_inserting();
    if !is_state(RL_STATE_VICMDONCE) {
        set_state(RL_STATE_VICMDONCE);
        crate::undo::rl_free_undo_list();
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_start_inserting(key: c_int, repeat: c_int, _sign: c_int) {
    set_last(key, repeat);
    crate::undo::rl_begin_undo_group();
    st().open_groups += 1;
    set_keymap(crate::keymap::vi_insertion());
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_insertion_mode(count: c_int, key: c_int) -> c_int {
    rl_vi_start_inserting(key, count, 1);
    0
}

/// Replay an insertion (`.` of i a I A).
unsafe fn redo_insert(count: c_int) {
    let t = st().insert_text.clone();
    crate::undo::rl_begin_undo_group();
    for _ in 0..count.max(1) {
        buffer::insert(&t);
    }
    crate::undo::rl_end_undo_group();
    back_one();
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_insert_mode(count: c_int, key: c_int) -> c_int {
    if redoing() && !st().insert_text.is_empty() {
        redo_insert(count);
        return 0;
    }
    rl_vi_start_inserting(key, count, 1);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_append_mode(count: c_int, key: c_int) -> c_int {
    if point() < end() {
        set_point(mb::next_char(line(), point()));
    }
    if redoing() && !st().insert_text.is_empty() {
        redo_insert(count);
        return 0;
    }
    rl_vi_start_inserting(key, count, 1);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_append_eol(count: c_int, key: c_int) -> c_int {
    set_point(end());
    if redoing() && !st().insert_text.is_empty() {
        redo_insert(count);
        return 0;
    }
    rl_vi_start_inserting(key, count, 1);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_insert_beg(count: c_int, key: c_int) -> c_int {
    set_point(0);
    if redoing() && !st().insert_text.is_empty() {
        redo_insert(count);
        return 0;
    }
    rl_vi_start_inserting(key, count, 1);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_editing_mode(_count: c_int, key: c_int) -> c_int {
    crate::readline::set_editing_mode_vi();
    rl_vi_insertion_mode(1, key);
    0
}

// ---------------------------------------------------------------- motions

/// `w`: forward over the current word (identifier or punctuation run) and
/// the blanks after it.
fn fword_pos(mut p: usize, count: c_int) -> usize {
    let l = line();
    let e = l.len();
    for _ in 0..count {
        if p >= e {
            break;
        }
        let c = class_at(l, p);
        if c != 0 {
            while p < e && class_at(l, p) == c {
                p = mb::next_char(l, p);
            }
        }
        while p < e && ws_at(l, p) {
            p = mb::next_char(l, p);
        }
    }
    p
}

fn fbigword_pos(mut p: usize, count: c_int) -> usize {
    let l = line();
    let e = l.len();
    for _ in 0..count {
        if p >= e {
            break;
        }
        while p < e && !ws_at(l, p) {
            p = mb::next_char(l, p);
        }
        while p < e && ws_at(l, p) {
            p = mb::next_char(l, p);
        }
    }
    p
}

fn bword_pos(mut p: usize, count: c_int, big: bool) -> usize {
    let l = line();
    for _ in 0..count {
        if p == 0 {
            break;
        }
        p = mb::prev_char(l, p);
        while p > 0 && ws_at(l, p) {
            p = mb::prev_char(l, p);
        }
        if ws_at(l, p) {
            break;
        }
        let c = class_at(l, p);
        loop {
            if p == 0 {
                break;
            }
            let q = mb::prev_char(l, p);
            let same = if big { !ws_at(l, q) } else { class_at(l, q) == c };
            if !same {
                break;
            }
            p = q;
        }
    }
    p
}

fn eword_pos(mut p: usize, count: c_int, big: bool) -> usize {
    let l = line();
    let e = l.len();
    for _ in 0..count {
        if p >= e || mb::next_char(l, p) >= e {
            break;
        }
        p = mb::next_char(l, p);
        while p < e && ws_at(l, p) {
            p = mb::next_char(l, p);
        }
        if p >= e {
            break;
        }
        let c = class_at(l, p);
        loop {
            let q = mb::next_char(l, p);
            if q >= e {
                break;
            }
            let same = if big { !ws_at(l, q) } else { class_at(l, q) == c };
            if !same {
                break;
            }
            p = q;
        }
    }
    p
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_fword(count: c_int, _key: c_int) -> c_int {
    set_point(fword_pos(point(), count));
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_fWord(count: c_int, _key: c_int) -> c_int {
    set_point(fbigword_pos(point(), count));
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_bword(count: c_int, _key: c_int) -> c_int {
    set_point(bword_pos(point(), count, false));
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_bWord(count: c_int, _key: c_int) -> c_int {
    set_point(bword_pos(point(), count, true));
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_eword(count: c_int, _key: c_int) -> c_int {
    set_point(eword_pos(point(), count, false));
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_eWord(count: c_int, _key: c_int) -> c_int {
    set_point(eword_pos(point(), count, true));
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_next_word(count: c_int, key: c_int) -> c_int {
    if count < 0 {
        return rl_vi_prev_word(-count, key);
    }
    let e = end();
    if e == 0 || point() >= e - 1 {
        rl_ding();
        return 0;
    }
    if upper(key) {
        rl_vi_fWord(count, key)
    } else {
        rl_vi_fword(count, key)
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_prev_word(count: c_int, key: c_int) -> c_int {
    if count < 0 {
        return rl_vi_next_word(-count, key);
    }
    if point() == 0 {
        rl_ding();
        return 0;
    }
    if upper(key) {
        rl_vi_bWord(count, key)
    } else {
        rl_vi_bword(count, key)
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_end_word(count: c_int, key: c_int) -> c_int {
    if count < 0 {
        rl_ding();
        return 1;
    }
    if upper(key) {
        rl_vi_eWord(count, key)
    } else {
        rl_vi_eword(count, key)
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_back_to_indent(_count: c_int, _key: c_int) -> c_int {
    let l = line();
    let mut p = 0;
    while p < l.len() && is_ws(l[p]) {
        p += 1;
    }
    set_point(p);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_first_print(count: c_int, key: c_int) -> c_int {
    // keep a body distinct from rl_vi_back_to_indent: identical functions
    // may be folded into one address, and the dumpers print names by address
    core::hint::black_box(key);
    rl_vi_back_to_indent(count, key)
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_column(count: c_int, _key: c_int) -> c_int {
    let l = line();
    let p = if count <= 1 { 0 } else { mb::forward_chars(l, 0, (count - 1) as usize) };
    set_point(p);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_arg_digit(count: c_int, key: c_int) -> c_int {
    if key == b'0' as c_int && crate::numarg::rl_explicit_arg.get() == 0 {
        crate::commands::movement::rl_beg_of_line(1, key)
    } else {
        let r = crate::numarg::rl_digit_argument(count, key);
        if crate::input::rl_pending_input.get() == EOF {
            // end of input inside a vi count: the reference accepts the line
            crate::input::rl_clear_pending_input();
            crate::readline::rl_newline(1, b'\n' as c_int);
        }
        r
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_bracktype(c: c_int) -> c_int {
    match c as u8 {
        b'(' => 1,
        b')' => -1,
        b'[' => 2,
        b']' => -2,
        b'{' => 3,
        b'}' => -3,
        _ => 0,
    }
}

fn bracktype(b: u8) -> c_int {
    unsafe { rl_vi_bracktype(b as c_int) }
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_match(_count: c_int, _key: c_int) -> c_int {
    let l = line();
    let e = l.len();
    let orig = point();
    let mut p = orig;
    let mut b;
    loop {
        if p >= e {
            rl_ding();
            return -1;
        }
        b = bracktype(l[p]);
        if b != 0 {
            break;
        }
        p = mb::next_char(l, p);
    }
    let mut depth = 1;
    if b > 0 {
        let mut i = p;
        loop {
            i = mb::next_char(l, i);
            if i >= e {
                set_point(orig);
                rl_ding();
                return -1;
            }
            let t = bracktype(l[i]);
            if t == b {
                depth += 1;
            } else if t == -b {
                depth -= 1;
                if depth == 0 {
                    set_point(i);
                    return 0;
                }
            }
        }
    } else {
        let mut i = p;
        loop {
            if i == 0 {
                set_point(orig);
                rl_ding();
                return -1;
            }
            i = mb::prev_char(l, i);
            let t = bracktype(l[i]);
            if t == b {
                depth += 1;
            } else if t == -b {
                depth -= 1;
                if depth == 0 {
                    set_point(i);
                    return 0;
                }
            }
        }
    }
}

/// Search for `ch` `count` times; kind f F t T.  Moves as far as it finds
/// occurrences; fails (without moving) when there is none.
unsafe fn char_search(count: c_int, kind: u8, ch: &[u8]) -> c_int {
    let l = line();
    let e = l.len();
    let p0 = point();
    let fwd = kind == b'f' || kind == b't';
    let mut q = p0;
    let mut found = false;
    for _ in 0..count.max(1) {
        let mut hit = None;
        if fwd {
            let mut i = mb::next_char(l, q);
            while i < e {
                if l[i..].starts_with(ch) {
                    hit = Some(i);
                    break;
                }
                i = mb::next_char(l, i);
            }
        } else {
            let mut i = q;
            while i > 0 {
                i = mb::prev_char(l, i);
                if l[i..].starts_with(ch) {
                    hit = Some(i);
                    break;
                }
            }
        }
        match hit {
            Some(i) => {
                q = i;
                found = true;
            }
            None => break,
        }
    }
    if !found {
        rl_ding();
        return -1;
    }
    let r = match kind {
        b't' => mb::prev_char(l, q),
        b'T' => mb::next_char(l, q),
        _ => q,
    };
    set_point(r);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_char_search(count: c_int, key: c_int) -> c_int {
    let k = key as u8;
    let (kind, ch) = match k {
        b';' | b',' => {
            let s = st();
            if s.cs_key == 0 {
                rl_ding();
                return -1;
            }
            let mut kind = s.cs_key as u8;
            if k == b',' {
                kind = match kind {
                    b'f' => b'F',
                    b'F' => b'f',
                    b't' => b'T',
                    _ => b't',
                };
            }
            (kind, s.cs_char.clone())
        }
        _ => {
            let kind = if matches!(k, b'f' | b'F' | b't' | b'T') { k } else { b'f' };
            let ch = if redoing() {
                st().last_char.clone()
            } else {
                set_state(RL_STATE_CHARSEARCH);
                let c = read_char_arg();
                unset_state(RL_STATE_CHARSEARCH);
                match c {
                    Some(c) => c,
                    None => {
                        callback_char_fail();
                        return -1;
                    }
                }
            };
            let s = st();
            s.cs_key = kind as c_int;
            s.cs_char = ch.clone();
            if is_state(RL_STATE_VIMOTION) {
                s.last_char = ch.clone();
            }
            (kind, ch)
        }
    };
    let r = char_search(count, kind, &ch);
    if r != 0 && k != b';' && k != b',' {
        callback_char_fail();
    }
    r
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_set_mark(_count: c_int, _key: c_int) -> c_int {
    let Some(c) = read_char_arg() else {
        callback_char_fail();
        return -1;
    };
    if c.len() != 1 || !c[0].is_ascii_lowercase() {
        rl_ding();
        callback_char_fail();
        return 1;
    }
    st().marks[(c[0] - b'a') as usize] = point() as c_int;
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_goto_mark(_count: c_int, _key: c_int) -> c_int {
    let Some(c) = read_char_arg() else {
        callback_char_fail();
        return -1;
    };
    if c.len() == 1 && c[0] == b'`' {
        set_point(buffer::mark());
        return 0;
    }
    if c.len() != 1 || !c[0].is_ascii_lowercase() {
        rl_ding();
        callback_char_fail();
        return 1;
    }
    let m = st().marks[(c[0] - b'a') as usize];
    if m < 0 || m as usize > end() {
        rl_ding();
        callback_char_fail();
        return 1;
    }
    set_point(m as usize);
    0
}

// ---------------------------------------------------------------- operators

const MOTIONS: &[u8] = b" hl^$0ftFT;,%wbeWBE|`";
const INCLUSIVE: &[u8] = b"eEft;,%";

fn is_universal_arg(k: c_int) -> bool {
    if !(0..256).contains(&k) {
        return false;
    }
    unsafe {
        let e = *crate::keymap::entry(crate::keymap::current(), k as usize);
        e.type_ == ISFUNC && crate::dispatch::same_fn(e.function.func, crate::numarg::rl_universal_argument)
    }
}

fn is_digit(k: c_int) -> bool {
    (b'0' as c_int..=b'9' as c_int).contains(&k)
}

/// Read the motion of an operator and move point.  Returns the motion key
/// or None (aborted / invalid motion / failed motion).  On success point
/// and mark delimit the range (point <= mark).
unsafe fn domove(op: u8, count: c_int) -> Option<(c_int, c_int)> {
    domove_with(op, count, None)
}

unsafe fn domove_with(op: u8, count: c_int, forced: Option<c_int>) -> Option<(c_int, c_int)> {
    let start = point();
    let mut total = count.max(1);
    let mut c;
    if redoing() {
        c = st().last_motion;
    } else if let Some(f) = forced {
        c = f;
    } else {
        set_state(RL_STATE_VIMOTION);
        c = read_raw_key();
        unset_state(RL_STATE_VIMOTION);
        if c < 0 {
            return None;
        }
        // a count for the motion
        if (is_digit(c) && c != b'0' as c_int) || is_universal_arg(c) {
            let mut n: c_int = 1;
            let mut have = false;
            loop {
                if is_digit(c) {
                    let d = c - b'0' as c_int;
                    n = if have { n.saturating_mul(10).saturating_add(d).min(1_000_000) } else { d };
                    have = true;
                } else if is_universal_arg(c) {
                    n = if have { n.saturating_mul(4).min(1_000_000) } else { 4 };
                    have = true;
                } else {
                    break;
                }
                let after_digit = is_digit(c);
                set_state(RL_STATE_VIMOTION);
                c = read_raw_key();
                unset_state(RL_STATE_VIMOTION);
                if c < 0 {
                    if after_digit {
                        // the reference accepts the line when the input
                        // ends inside a motion count
                        crate::readline::rl_newline(1, b'\n' as c_int);
                    }
                    return None;
                }
            }
            total = total.saturating_mul(n);
        }
    }
    let lc = (c as u8).to_ascii_lowercase();
    if c >= 0 && c < 128 && lc == op && (c as u8).is_ascii_lowercase() {
        // dd, cc, yy: the whole line
        st().last_motion = c;
        set_point(0);
        buffer::rl_mark.set(end() as c_int);
        return Some((c, total));
    }
    if !(0..128).contains(&c) || !MOTIONS.contains(&(c as u8)) {
        rl_ding();
        return None;
    }
    if !redoing() {
        st().last_motion = c;
    }
    let cb = c as u8;
    // cw / cW change to the end of the word (not over the blanks after it)
    let cw = op == b'c' && (cb == b'w' || cb == b'W');
    set_state(RL_STATE_VIMOTION);
    crate::numarg::rl_numeric_arg.set(total);
    crate::numarg::rl_explicit_arg.set(0);
    crate::numarg::ARG_SIGN.set(1);
    let r = crate::dispatch::dispatch_key(c, crate::keymap::current());
    crate::numarg::reset();
    unset_state(RL_STATE_VIMOTION);
    // in callback mode the reference does not see a failed f/t search
    // inside an operator: the motion "succeeds" without moving
    let cb_search = is_state(RL_STATE_CALLBACK) && matches!(cb, b'f' | b'F' | b't' | b'T');
    if r != 0 && cb_search {
        set_point(start);
    } else if r != 0 && cb != b'w' && cb != b'W' && cb != b'b' && cb != b'B' {
        set_point(start);
        return None;
    }
    let l = line();
    let mut np = point().min(l.len());
    if (cb == b'w' || cb == b'W') && np == start && start < l.len() {
        // w cannot move from the last character: act up to the end
        np = l.len();
    }
    if cw && np < l.len() {
        // back over the blanks the motion skipped, keeping at least one char
        while np > start + 1 && np > 0 && is_ws(l[mb::prev_char(l, np)]) {
            np = mb::prev_char(l, np);
        }
    }
    let mut m = np;
    if INCLUSIVE.contains(&cb) && np >= start && np < l.len() {
        m = mb::next_char(l, np);
    }
    if m < start {
        set_point(m);
        buffer::rl_mark.set(start as c_int);
    } else {
        set_point(start);
        buffer::rl_mark.set(m as c_int);
    }
    Some((c, total))
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_domove(key: c_int, nextkey: *mut c_int) -> c_int {
    let op = (key as u8).to_ascii_lowercase();
    match domove(op, crate::numarg::count()) {
        Some((c, _)) => {
            if !nextkey.is_null() {
                *nextkey = c;
            }
            0
        }
        None => -1,
    }
}

/// The count for an operator: the explicit count, or the saved one when
/// redoing without a new count.
fn op_count(count: c_int) -> c_int {
    if redoing() {
        count
    } else {
        count.max(1)
    }
}

/// Run the motion of an operator `op` (`key` is the invoking key, upper
/// case D C Y act to the end of line).  Returns the motion key.
unsafe fn operator_range(op: u8, key: c_int, count: c_int) -> Option<(c_int, c_int)> {
    if upper(key) {
        // D C Y: to the end of the line
        return domove_with(op, count, Some(b'$' as c_int));
    }
    domove(op, count)
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_delete_to(count: c_int, key: c_int) -> c_int {
    let count = op_count(count);
    let Some((_m, count)) = operator_range(b'd', key, count) else {
        return -1;
    };
    let a = point();
    let b = buffer::mark();
    if a < b {
        kill_range(a, b);
    }
    set_point(a.min(end()));
    set_last(key, count);
    0
}

/// c s S C open an undo group (when redoing only if there is text to
/// insert again).
fn change_group() -> bool {
    let g = !redoing() || !st().insert_text.is_empty();
    if g {
        unsafe { crate::undo::rl_begin_undo_group() };
    }
    g
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_change_to(count: c_int, key: c_int) -> c_int {
    let count = op_count(count);
    let g = change_group();
    let Some((_m, count)) = operator_range(b'c', key, count) else {
        if g {
            crate::undo::rl_end_undo_group();
        }
        return -1;
    };
    let a = point();
    let b = buffer::mark();
    if a < b {
        kill_range(a, b);
    }
    set_point(a.min(end()));
    finish_change(key, count);
    0
}

/// After the deletion of c / s / S / C: start inserting (one more undo
/// group), or, when redoing, insert the saved text.
unsafe fn finish_change(key: c_int, count: c_int) {
    if redoing() {
        let t = st().insert_text.clone();
        if !t.is_empty() {
            buffer::insert(&t);
            crate::undo::rl_end_undo_group();
        }
        return;
    }
    rl_vi_start_inserting(key, count, 1);
    let s = st();
    s.open_groups += 1;
    s.doing_insert = true;
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_yank_to(count: c_int, key: c_int) -> c_int {
    let count = op_count(count);
    let orig = point();
    let Some((_m, count)) = operator_range(b'y', key, count) else {
        set_point(orig);
        return -1;
    };
    let a = point();
    let b = buffer::mark();
    if a < b {
        // kill the text and undo the kill: the kill ring gets the text and
        // the mark is clamped as if it had been deleted
        crate::undo::rl_begin_undo_group();
        kill_range(a, b);
        crate::undo::rl_end_undo_group();
        crate::undo::rl_do_undo();
    }
    set_point(orig.min(end()));
    set_last(key, count);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_delete(count: c_int, key: c_int) -> c_int {
    if count < 0 {
        return rl_vi_rubout(-count, key);
    }
    if end() == 0 {
        rl_ding();
        return 1;
    }
    let p = point();
    let to = mb::forward_chars(line(), p, count as usize);
    kill_range(p, to);
    let e = end();
    if e > 0 && point() >= e {
        set_point(mb::prev_char(line(), e));
    }
    set_last(key, count);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_rubout(count: c_int, key: c_int) -> c_int {
    if count < 0 {
        return rl_vi_delete(-count, key);
    }
    let p = point();
    if p == 0 {
        rl_ding();
        return 1;
    }
    let from = mb::backward_chars(line(), p, count as usize);
    kill_range(p, from);
    set_point(from);
    set_last(key, count);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_subst(count: c_int, key: c_int) -> c_int {
    // s = c with an l motion, S = cc
    let count = op_count(count);
    let g = change_group();
    let motion = if upper(key) { b'c' } else { b'l' } as c_int;
    let Some((_m, count)) = domove_with(b'c', count, Some(motion)) else {
        if g {
            crate::undo::rl_end_undo_group();
        }
        return -1;
    };
    let a = point();
    let b = buffer::mark();
    if a < b {
        kill_range(a, b);
    }
    set_point(a.min(end()));
    finish_change(key, count);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_change_char(count: c_int, key: c_int) -> c_int {
    let ch = if redoing() {
        st().last_char.clone()
    } else {
        match read_char_arg() {
            Some(c) => c,
            None => return -1,
        }
    };
    if !redoing() {
        st().last_char = ch.clone();
    }
    set_last(key, count);
    if end() == 0 || point() >= end() {
        rl_ding();
        return 1;
    }
    crate::undo::rl_begin_undo_group();
    let mut p = point();
    for _ in 0..count.max(1) {
        if p >= end() {
            break;
        }
        let q = mb::next_char(line(), p);
        buffer::delete(p, q);
        buffer::insert_at(p, &ch);
        p += ch.len();
    }
    crate::undo::rl_end_undo_group();
    set_point(p);
    back_one();
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_change_case(count: c_int, key: c_int) -> c_int {
    if end() == 0 {
        rl_ding();
        return 1;
    }
    set_last(key, count);
    for _ in 0..count.max(1) {
        let p = point();
        let l = line();
        if p >= l.len() {
            break;
        }
        let (wc, n) = mb::decode_at(l, p);
        let mut repl: Option<Vec<u8>> = None;
        if let Some(c) = wc {
            let nc = if crate::util::iswupper(c as _) != 0 {
                crate::util::towlower(c as _)
            } else if crate::util::iswlower(c as _) != 0 {
                crate::util::towupper(c as _)
            } else {
                c as _
            };
            if nc as u32 != c {
                if n == 1 && nc < 256 {
                    repl = Some(vec![nc as u8]);
                } else {
                    let mut buf = [0u8; 16];
                    let mut st = crate::util::MbState::new();
                    let r = crate::util::wcrtomb(buf.as_mut_ptr() as *mut c_char, nc as libc::wchar_t, &mut st);
                    if r != usize::MAX && r > 0 {
                        repl = Some(buf[..r].to_vec());
                    }
                }
            }
        }
        if let Some(r) = repl {
            crate::undo::rl_begin_undo_group();
            buffer::delete(p, p + n);
            buffer::insert_at(p, &r);
            crate::undo::rl_end_undo_group();
            set_point(p + r.len());
        } else {
            set_point(p + n);
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_put(count: c_int, key: c_int) -> c_int {
    if key != b'P' as c_int && point() < end() {
        set_point(mb::next_char(line(), point()));
    }
    for _ in 0..count.max(1) {
        if crate::kill::rl_yank(1, key) != 0 {
            // empty kill ring: the reference aborts the command here
            SKIP_CHECK.set(true);
            crate::commands::misc::abort_internal();
            return 1;
        }
    }
    back_one();
    set_last(key, count);
    0
}

// ---------------------------------------------------------------- replace mode

unsafe fn replace_keymap() -> Keymap {
    let s = st();
    if s.replace_map.is_null() {
        let m = crate::keymap::rl_copy_keymap(crate::keymap::vi_insertion());
        let os = KEYMAP_ENTRY::func(rl_vi_overstrike);
        for i in 32..127 {
            *m.add(i) = os;
        }
        for i in 128..256 {
            *m.add(i) = os;
        }
        let del = KEYMAP_ENTRY::func(rl_vi_overstrike_delete);
        *m.add(127) = del;
        *m.add(8) = del;
        // ESC leaves replace mode at once (no escape-sequence prefix)
        *m.add(ESC as usize) = KEYMAP_ENTRY::func(rl_vi_movement_mode);
        s.replace_map = m;
    }
    s.replace_map
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_replace(count: c_int, key: c_int) -> c_int {
    if redoing() && !st().insert_text.is_empty() {
        // replace as many characters as the saved text has, in one change
        let t = st().insert_text.clone();
        let mut text = Vec::new();
        for _ in 0..count.max(1) {
            text.extend_from_slice(&t);
        }
        let nchars = {
            let mut n = 0;
            let mut i = 0;
            while i < text.len() {
                i += mb::char_len_at(&text, i);
                n += 1;
            }
            n
        };
        crate::undo::rl_begin_undo_group();
        crate::undo::rl_begin_undo_group();
        let p = point();
        let to = mb::forward_chars(line(), p, nchars);
        if to > p {
            buffer::delete(p, to);
        }
        buffer::insert(&text);
        crate::undo::rl_end_undo_group();
        crate::undo::rl_end_undo_group();
        back_one();
        return 0;
    }
    set_last(key, count);
    crate::undo::rl_begin_undo_group();
    let m = replace_keymap();
    let s = st();
    s.doing_insert = true;
    s.open_groups += 1;
    s.overstrike = true;
    s.replaced = 0;
    s.ostext.clear();
    set_keymap(m);
    0
}

/// Overwrite characters at point with `text` (character by character).
unsafe fn overstrike_bytes(text: &[u8]) {
    let mut i = 0;
    while i < text.len() {
        let n = mb::char_len_at(text, i);
        let ch = &text[i..(i + n).min(text.len())];
        overstrike_one(ch);
        i += n;
    }
}

/// Overwrite one character (one undo group, so backspace can undo it).
unsafe fn overstrike_one(ch: &[u8]) {
    if st().replaced == 0 {
        // the replacement as a whole is one more group
        crate::undo::rl_begin_undo_group();
        st().open_groups += 1;
    }
    crate::undo::rl_begin_undo_group();
    buffer::insert(ch);
    let q = point();
    if q < end() {
        let r = mb::next_char(line(), q);
        buffer::delete(q, r);
    }
    crate::undo::rl_end_undo_group();
    let s = st();
    s.replaced += 1;
    s.ostext.extend_from_slice(ch);
}

/// Insert text in replace mode (bracketed paste).
pub unsafe fn overstrike_text(text: &[u8]) {
    overstrike_bytes(text);
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_overstrike(count: c_int, key: c_int) -> c_int {
    if !st().overstrike {
        return crate::commands::text::rl_insert(count, key);
    }
    let ch = read_mb_rest(key as u8);
    for _ in 0..count.max(1) {
        overstrike_one(&ch);
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_overstrike_delete(count: c_int, _key: c_int) -> c_int {
    for _ in 0..count.max(1) {
        let s = st();
        if s.replaced == 0 {
            rl_ding();
            break;
        }
        s.replaced -= 1;
        // drop the typed character from the redo text
        if !s.ostext.is_empty() {
            let l = s.ostext.len();
            let p = mb::prev_char(&s.ostext, l);
            s.ostext.truncate(p);
        }
        // undo the character's overwrite
        crate::undo::rl_do_undo();
        if st().replaced == 0 {
            // and drop the group the first character opened
            crate::undo::rl_do_undo();
            st().open_groups -= 1;
        }
    }
    0
}

// ---------------------------------------------------------------- redo, undo

#[no_mangle]
pub unsafe extern "C" fn rl_vi_redo(count: c_int, _key: c_int) -> c_int {
    let s = st();
    let cmd = s.last_cmd;
    if cmd == 0 {
        rl_ding();
        return 0;
    }
    let count = if crate::numarg::rl_explicit_arg.get() != 0 {
        s.last_count = count;
        count
    } else {
        s.last_count
    };
    REDOING.set(true);
    let r = match cmd as u8 {
        b'i' => rl_vi_insert_mode(count, cmd),
        b'a' => rl_vi_append_mode(count, cmd),
        b'A' => rl_vi_append_eol(count, cmd),
        b'I' => rl_vi_insert_beg(count, cmd),
        b'R' => rl_vi_replace(count, cmd),
        _ => {
            let e = *crate::keymap::entry(crate::keymap::vi_movement(), (cmd as usize) & 0xff);
            match (e.type_, e.function.func) {
                (ISFUNC, Some(f)) if !crate::dispatch::same_fn(Some(f), rl_vi_redo) => {
                    crate::dispatch::rl_executing_key.set(cmd);
                    f(count, cmd)
                }
                _ => {
                    rl_ding();
                    0
                }
            }
        }
    };
    REDOING.set(false);
    r
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_undo(count: c_int, key: c_int) -> c_int {
    crate::undo::rl_undo_command(count, key)
}

// ---------------------------------------------------------------- misc

#[no_mangle]
pub unsafe extern "C" fn rl_vi_eof_maybe(_count: c_int, _key: c_int) -> c_int {
    if end() == 0 {
        crate::readline::signal_eof();
        return 0;
    }
    crate::readline::rl_newline(1, b'\n' as c_int)
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_unix_word_rubout(count: c_int, _key: c_int) -> c_int {
    let p0 = point();
    if p0 == 0 {
        rl_ding();
        return 0;
    }
    let l = line();
    let mut p = p0;
    for _ in 0..count.max(1) {
        while p > 0 && is_ws(l[mb::prev_char(l, p)]) {
            p = mb::prev_char(l, p);
        }
        if p == 0 {
            break;
        }
        let c = class_at(l, mb::prev_char(l, p));
        while p > 0 {
            let q = mb::prev_char(l, p);
            if class_at(l, q) != c {
                break;
            }
            p = q;
        }
    }
    crate::kill::rl_kill_text(p0 as c_int, p as c_int);
    set_point(p);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_complete(_count: c_int, key: c_int) -> c_int {
    let l = line();
    if point() < l.len() && !is_ws(l[point()]) {
        let n = mb::next_char(l, point());
        if n < l.len() && !is_ws(l[n]) {
            rl_vi_end_word(1, b'E' as c_int);
        }
        set_point(mb::next_char(line(), point()));
    }
    let r = match key as u8 {
        b'*' => crate::complete::rl_insert_completions(0, key),
        b'=' => crate::complete::rl_possible_completions(0, key),
        b'\\' => crate::complete::rl_complete(0, b'\t' as c_int),
        _ => crate::complete::rl_complete(0, key),
    };
    if key == b'*' as c_int || key == b'\\' as c_int {
        rl_vi_start_inserting(key, 1, 1);
    }
    r
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_tilde_expand(_count: c_int, key: c_int) -> c_int {
    crate::commands::text::rl_tilde_expand(0, key);
    rl_vi_start_inserting(key, 1, 1);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_fetch_history(count: c_int, key: c_int) -> c_int {
    use crate::history::{history_base, history_length, history_offset};
    if crate::numarg::rl_explicit_arg.get() == 0 {
        // G: the oldest entry
        return crate::histcmds::rl_beginning_of_history(1, key);
    }
    let wanted = count - history_base.get();
    if wanted < 0 || wanted >= history_length.get() {
        rl_ding();
        return 0;
    }
    // the reference only moves back in the history here
    let off = history_offset.get();
    if wanted < off {
        crate::histcmds::rl_get_previous_history(off - wanted, key);
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_yank_arg(count: c_int, key: c_int) -> c_int {
    // `_`: append the last word (with a count: word count-1) of the
    // previous history line, preceded by a space, and enter insert mode
    let n = if crate::numarg::rl_explicit_arg.get() != 0 { count - 1 } else { b'$' as c_int };
    let h = crate::history::previous_history();
    if h.is_null() {
        rl_ding();
        return 1;
    }
    let text = (*h).line;
    crate::history::next_history();
    let w = crate::history::expand::history_arg_extract(n, n, text);
    if w.is_null() {
        rl_ding();
        return 1;
    }
    let word = cstr(w).to_vec();
    cfree(w);
    buffer::rl_mark.set(point() as c_int);
    rl_vi_append_mode(1, key);
    let mut t = b" ".to_vec();
    t.extend_from_slice(&word);
    buffer::insert(&t);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_yank_pop(count: c_int, key: c_int) -> c_int {
    if !crate::kill::last_command_was_yank() {
        // aborts without moving point
        return crate::kill::rl_yank_pop(count, key);
    }
    if point() < end() {
        set_point(mb::next_char(line(), point()));
    }
    let r = crate::kill::rl_yank_pop(count, key);
    back_one();
    r
}

/// The last vi search string and direction (-1 older, 1 newer).
static SEARCH: CVar<Option<(Vec<u8>, c_int)>> = CVar::new(None);

/// Read a search string in the line buffer (the prompt shows the last
/// line of the prompt followed by `pchar`).  The line, point, mark and undo
/// list are restored afterwards.  None when aborted.
unsafe fn read_search_string(pchar: u8) -> Option<Vec<u8>> {
    let saved = line().to_vec();
    let (sp, sm) = (buffer::rl_point.get(), buffer::rl_mark.get());
    let saved_undo = crate::undo::rl_undo_list.replace(null_mut());
    set_state(RL_STATE_NSEARCH);
    let prompt = cstr(crate::display::rl_prompt.get()).to_vec();
    let mut msg = crate::display::split_prompt(&prompt).1.to_vec();
    msg.push(pchar);
    crate::display::set_message(&msg);
    buffer::set_line(b"");
    set_point(0);
    buffer::rl_mark.set(0);
    let result = loop {
        crate::display::redisplay();
        let c = read_raw_key();
        if c < 0 {
            break None;
        }
        let p = point();
        match c {
            7 => break None,
            127 | 8 => {
                if end() == 0 {
                    break None;
                }
                if p > 0 {
                    let a = mb::prev_char(line(), p);
                    buffer::delete(a, p);
                    set_point(a);
                }
            }
            21 => {
                buffer::delete(0, p);
                set_point(0);
            }
            23 => {
                let l = line();
                let mut a = p;
                while a > 0 && is_ws(l[a - 1]) {
                    a -= 1;
                }
                while a > 0 && !is_ws(l[a - 1]) {
                    a -= 1;
                }
                buffer::delete(a, p);
                set_point(a);
            }
            22 => {
                let q = read_raw_key();
                if q >= 0 {
                    buffer::insert(&read_mb_rest(q as u8));
                }
            }
            13 | 10 => break Some(line().to_vec()),
            _ => {
                buffer::insert(&read_mb_rest(c as u8));
            }
        }
    };
    unset_state(RL_STATE_NSEARCH);
    crate::display::clear_arg_message();
    let l = crate::undo::rl_undo_list.replace(saved_undo);
    crate::undo::free_list(l);
    buffer::set_line(&saved);
    buffer::rl_point.set(sp);
    buffer::rl_mark.set(sm);
    result
}

fn search_matches(line: &[u8], pat: &[u8]) -> bool {
    if let Some(rest) = pat.strip_prefix(b"^") {
        return line.starts_with(rest);
    }
    pat.is_empty() || line.windows(pat.len()).any(|w| w == pat)
}

/// Search the history from the current entry in `dir` and show the match.
unsafe fn search_history(pat: &[u8], dir: c_int) -> c_int {
    use crate::history::{history_length, history_offset, line_at};
    let off = history_offset.get();
    let len = history_length.get();
    let mut i = off;
    loop {
        i += dir;
        if i < 0 || i >= len {
            rl_ding();
            return 0;
        }
        if line_at(i).is_some_and(|l| search_matches(l, pat)) {
            break;
        }
    }
    if i < off {
        crate::histcmds::rl_get_previous_history(off - i, 0);
    } else {
        crate::histcmds::rl_get_next_history(i - off, 0);
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_search(_count: c_int, key: c_int) -> c_int {
    let dir = match key as u8 {
        b'/' => -1,
        b'?' => 1,
        _ => {
            rl_ding();
            return 0;
        }
    };
    let Some(mut pat) = read_search_string(key as u8) else {
        crate::display::redisplay();
        return 0;
    };
    if pat.is_empty() {
        match SEARCH.get_mut() {
            Some((p, _)) => pat = p.clone(),
            None => {
                rl_ding();
                return 0;
            }
        }
    }
    SEARCH.set(Some((pat.clone(), dir)));
    search_history(&pat, dir)
}

#[no_mangle]
pub unsafe extern "C" fn rl_vi_search_again(_count: c_int, key: c_int) -> c_int {
    let Some((pat, _dir)) = SEARCH.get_mut().clone() else {
        rl_ding();
        return 0;
    };
    // n searches back (older), N forward, whatever the last search was
    let dir = if key == b'N' as c_int { 1 } else { -1 };
    search_history(&pat, dir)
}
