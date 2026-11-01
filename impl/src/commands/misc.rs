//! Miscellaneous commands: abort, execute-named-command (the M-x
//! minibuffer), terminal flow control.
//! Owner: emacs workstream.

use crate::buffer;
use crate::input::{set_state, unset_state};
use crate::prelude::*;

extern "C" {
    fn lr_catch(f: unsafe extern "C" fn(*mut c_void), arg: *mut c_void) -> c_int;
    fn lr_catching() -> c_int;
    fn lr_throw();
}

/// Run `f` so that an abort inside it returns here (true) instead of to
/// its caller.  Used by the editing loop around each command.
pub fn catch_abort(f: &mut dyn FnMut()) -> bool {
    unsafe extern "C" fn tramp(arg: *mut c_void) {
        let f = &mut *(arg as *mut &mut dyn FnMut());
        f();
    }
    let mut f = f;
    unsafe { lr_catch(tramp, &mut f as *mut &mut dyn FnMut() as *mut c_void) != 0 }
}

/// Abort the current command: ding, drop numeric argument, macros and
/// pending state, then leave the command (back to the editing loop's
/// top level, when there is one).
pub fn abort_internal() {
    unsafe {
        crate::terminal::rl_ding();
    }
    crate::numarg::reset();
    crate::kbdmacro::clear_macro_input();
    crate::input::unset_state(RL_STATE_MACRODEF | RL_STATE_MULTIKEY | RL_STATE_MOREINPUT);
    crate::display::clear_arg_message();
    unsafe {
        crate::input::rl_clear_pending_input();
        if lr_catching() != 0 {
            lr_throw();
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_abort(_count: c_int, _key: c_int) -> c_int {
    abort_internal();
    0
}

/// Bindable names starting with `prefix`, sorted, without duplicates.
fn names_with_prefix(prefix: &[u8]) -> Vec<Vec<u8>> {
    let mut v: Vec<Vec<u8>> =
        crate::funmap::entries().map(|(n, _)| n.to_vec()).filter(|n| n.starts_with(prefix)).collect();
    v.sort();
    v.dedup();
    v
}

/// Print `items` in columns below the line (like a completion listing).
fn list_names(items: &[Vec<u8>]) {
    use crate::terminal::{out, out_byte};
    crate::display::output_before_listing();
    let width = items.iter().map(|i| i.len()).max().unwrap_or(0) + 2;
    let cols = (crate::terminal::SCREEN_COLS.get().max(1) as usize / width).max(1);
    let rows = items.len().div_ceil(cols);
    for r in 0..rows {
        for c in 0..cols {
            let i = c * rows + r;
            let Some(it) = items.get(i) else { break };
            out(it);
            if c + 1 < cols && i + rows < items.len() {
                for _ in it.len()..width {
                    out_byte(b' ');
                }
            }
        }
        out_byte(b'\n');
    }
    crate::terminal::flush();
    crate::display::after_listing();
}

/// Complete the command name in `name` (TAB / space).
unsafe fn complete_name(name: &mut Vec<u8>) {
    let m = names_with_prefix(name);
    if m.is_empty() {
        crate::terminal::rl_ding();
        return;
    }
    // longest common prefix
    let mut lcp = m[0].clone();
    for n in &m[1..] {
        let k = lcp.iter().zip(n.iter()).take_while(|(a, b)| a == b).count();
        lcp.truncate(k);
    }
    if lcp.len() > name.len() {
        *name = lcp;
    } else if m.len() > 1 {
        list_names(&m);
    }
}

/// Show the minibuffer: the prompt followed by `!` and the name.
unsafe fn show_minibuffer(prefix: &[u8], name: &[u8]) {
    crate::display::set_message(prefix);
    buffer::set_line(name);
    buffer::set_point(name.len());
    crate::display::redisplay();
}

/// Read a command name.  None if aborted.
unsafe fn read_command_name() -> Option<Vec<u8>> {
    let mut prefix = crate::display::split_prompt(cstr(crate::display::rl_display_prompt.get())).1.to_vec();
    prefix.push(b'!');
    let mut name: Vec<u8> = Vec::new();
    loop {
        show_minibuffer(&prefix, &name);
        set_state(RL_STATE_MOREINPUT);
        let c = crate::input::rl_read_key();
        unset_state(RL_STATE_MOREINPUT);
        if c < 0 {
            return None;
        }
        let k = c as u8;
        match k {
            b'\r' | b'\n' => return Some(name),
            0x07 => return None,
            0x7f | 0x08 => {
                if name.is_empty() {
                    return None;
                }
                let p = crate::mb::prev_char(&name, name.len());
                name.truncate(p);
            }
            0x17 => {
                // C-w: the last whitespace-delimited word
                while name.last().is_some_and(|b| *b == b' ' || *b == b'\t') {
                    name.pop();
                }
                while name.last().is_some_and(|b| *b != b' ' && *b != b'\t') {
                    name.pop();
                }
            }
            0x15 => name.clear(),
            0x16 | 0x11 => {
                set_state(RL_STATE_MOREINPUT);
                let q = crate::input::rl_read_key();
                unset_state(RL_STATE_MOREINPUT);
                if q < 0 {
                    return None;
                }
                name.push(q as u8);
            }
            b'\t' | b' ' => complete_name(&mut name),
            0x1b => match crate::commands::text::read_paste_after_esc() {
                Some(t) => name.extend_from_slice(&t),
                None => name.push(k),
            },
            _ => name.push(k),
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_execute_named_command(count: c_int, key: c_int) -> c_int {
    let saved = buffer::line().to_vec();
    let (p, m) = (buffer::rl_point.get(), buffer::rl_mark.get());
    crate::display::clear_arg_message();
    let name = read_command_name();
    crate::display::clear_arg_message();
    buffer::set_line(&saved);
    buffer::rl_point.set(p);
    buffer::rl_mark.set(m);
    let Some(name) = name else {
        crate::terminal::rl_ding();
        crate::display::redisplay();
        return 1;
    };
    crate::display::redisplay();
    match crate::funmap::named_function(&name) {
        Some(f) => f(count, key),
        None => {
            crate::terminal::rl_ding();
            1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_restart_output(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_restart_output");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_stop_output(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_stop_output");
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_tty_status(_a0: c_int, _a1: c_int) -> c_int {
    todo_stub!("rl_tty_status");
    0
}
