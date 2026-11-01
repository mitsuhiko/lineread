//! Keyboard macros (start/end/call/print) and macro input (key-sequence
//! macros bound with ISMACR, rl_push_macro_input).
//! Owner: emacs workstream (macros), input workstream (macro input stack).
use crate::input::{is_state, set_state, unset_state};
use crate::prelude::*;

/// The macro text currently being executed (malloc'd copy, NULL if none).
#[no_mangle]
pub static rl_executing_macro: CVar<*mut c_char> = CVar::new(null_mut());

struct MacroInput {
    text: Vec<u8>,
    pos: usize,
    cstr: *mut c_char,
}

static STACK: CVar<Vec<MacroInput>> = CVar::new(Vec::new());
/// Keys recorded by start-kbd-macro.
static CURRENT: CVar<Vec<u8>> = CVar::new(Vec::new());
/// The last defined keyboard macro.
static LAST: CVar<Vec<u8>> = CVar::new(Vec::new());

/// Macro input may nest this deep (pushes beyond fail with an error).
const MAX_MACRO_LEVEL: usize = 16;

/// Push `text` as input (read before the terminal).
pub fn push_macro_input(text: &[u8]) {
    if STACK.get_mut().len() > MAX_MACRO_LEVEL {
        crate::inputrc::report_error(b"maximum macro execution nesting level exceeded");
        return;
    }
    unsafe {
        let c = c_strdup(text);
        STACK.get_mut().push(MacroInput { text: text.to_vec(), pos: 0, cstr: c });
        rl_executing_macro.set(c);
        set_state(RL_STATE_MACROINPUT);
    }
}

fn pop_macro() {
    unsafe {
        let s = STACK.get_mut();
        if let Some(m) = s.pop() {
            cfree(m.cstr);
        }
        rl_executing_macro.set(s.last().map(|m| m.cstr).unwrap_or(null_mut()));
        if s.is_empty() {
            unset_state(RL_STATE_MACROINPUT);
        }
    }
}

pub fn macro_input_pending() -> bool {
    STACK.get_mut().iter().any(|m| m.pos < m.text.len())
}

/// Next key of the innermost macro with input left.
pub fn next_macro_key() -> Option<u8> {
    loop {
        let s = STACK.get_mut();
        let m = s.last_mut()?;
        if m.pos < m.text.len() {
            let k = m.text[m.pos];
            m.pos += 1;
            return Some(k);
        }
        pop_macro();
    }
}

/// Drop all macro input (abort, signals).
pub fn clear_macro_input() {
    while !STACK.get_mut().is_empty() {
        pop_macro();
    }
}

pub fn record_key(k: u8) {
    CURRENT.get_mut().push(k);
}

#[no_mangle]
pub unsafe extern "C" fn rl_push_macro_input(text: *mut c_char) {
    if text.is_null() {
        return;
    }
    push_macro_input(cstr(text));
    cfree(text);
}

#[no_mangle]
pub unsafe extern "C" fn rl_start_kbd_macro(_count: c_int, _key: c_int) -> c_int {
    if is_state(RL_STATE_MACRODEF) {
        crate::commands::misc::abort_internal();
        return 1;
    }
    if crate::numarg::rl_explicit_arg.get() != 0 {
        // append to the last macro: replay it while recording (the
        // replayed keys are recorded again after the old definition)
        let last = LAST.get_mut().clone();
        let mut cur = last.clone();
        cur.extend_from_slice(&last);
        *CURRENT.get_mut() = cur;
        if !last.is_empty() {
            push_macro_input(&last);
        }
    } else {
        CURRENT.get_mut().clear();
    }
    set_state(RL_STATE_MACRODEF);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_end_kbd_macro(count: c_int, _key: c_int) -> c_int {
    if !is_state(RL_STATE_MACRODEF) {
        crate::commands::misc::abort_internal();
        return 1;
    }
    // drop the keys of the sequence that invoked end-kbd-macro
    let cur = CURRENT.get_mut();
    let n = (crate::dispatch::rl_key_sequence_length.get().max(0) as usize).min(cur.len());
    cur.truncate(cur.len() - n);
    *LAST.get_mut() = cur.clone();
    unset_state(RL_STATE_MACRODEF);
    rl_call_last_kbd_macro(count - 1, 0)
}

#[no_mangle]
pub unsafe extern "C" fn rl_call_last_kbd_macro(count: c_int, _key: c_int) -> c_int {
    let last = LAST.get_mut().clone();
    if last.is_empty() {
        crate::terminal::rl_ding();
        return 0;
    }
    if is_state(RL_STATE_MACRODEF) {
        // calling a macro while defining one: ding, keep going
        crate::terminal::rl_ding();
        let cur = CURRENT.get_mut();
        let n = (crate::dispatch::rl_key_sequence_length.get().max(0) as usize).min(cur.len());
        cur.truncate(cur.len() - n);
        return 0;
    }
    for _ in 0..count.max(0) {
        push_macro_input(&last);
    }
    0
}

/// Readable form of macro text: `\C-x`, `\M-x`, `\e`, `\t`, ...
pub fn macro_text(v: &[u8]) -> Vec<u8> {
    fn key(k: u8, out: &mut Vec<u8>) {
        match k {
            0x07 => out.extend_from_slice(b"\\a"),
            0x08 => out.extend_from_slice(b"\\b"),
            b'\t' => out.extend_from_slice(b"\\t"),
            b'\n' => out.extend_from_slice(b"\\n"),
            0x0b => out.extend_from_slice(b"\\v"),
            0x0c => out.extend_from_slice(b"\\f"),
            b'\r' => out.extend_from_slice(b"\\r"),
            0x1b => out.extend_from_slice(b"\\e"),
            0x7f => out.extend_from_slice(b"\\C-?"),
            0..=31 => {
                out.extend_from_slice(b"\\C-");
                let c = (k + 64).to_ascii_lowercase();
                if c == b'\\' {
                    out.push(b'\\');
                }
                out.push(c);
            }
            b'\\' | b'"' => {
                out.push(b'\\');
                out.push(k);
            }
            128..=255 => {
                out.extend_from_slice(b"\\M-");
                key(k - 128, out);
            }
            _ => out.push(k),
        }
    }
    let mut out = Vec::new();
    for &b in v {
        key(b, &mut out);
    }
    out
}

#[no_mangle]
pub unsafe extern "C" fn rl_print_last_kbd_macro(_count: c_int, _key: c_int) -> c_int {
    let last = LAST.get_mut().clone();
    if last.is_empty() {
        crate::terminal::rl_ding();
        return 0;
    }
    crate::display::output_before_listing();
    let mut t = macro_text(&last);
    t.push(b'\n');
    crate::terminal::out(&t);
    crate::terminal::flush();
    crate::display::after_listing();
    crate::display::redisplay();
    0
}
