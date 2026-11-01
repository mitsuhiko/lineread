//! Key dispatch: resolving a key (sequence) in a keymap and running the
//! bound command, macro or prefix map.  Multi-key sequences are read here
//! (blocking style; the callback interface suspends inside rl_read_key).
//! Owner: emacs workstream (core: coordinate changes, everyone depends on it).
use crate::input::{self, is_state, set_state, unset_state};
use crate::keymap;
use crate::prelude::*;
use crate::util::xrealloc;

#[no_mangle]
pub static rl_dispatching: CVar<c_int> = CVar::new(0);
#[no_mangle]
pub static rl_executing_key: CVar<c_int> = CVar::new(0);
#[no_mangle]
pub static rl_executing_keymap: CVar<Keymap> = CVar::new(null_mut());
#[no_mangle]
pub static rl_executing_keyseq: CVar<*mut c_char> = CVar::new(null_mut());
#[no_mangle]
pub static rl_key_sequence_length: CVar<c_int> = CVar::new(0);
#[no_mangle]
pub static rl_last_func: CVar<Option<rl_command_func_t>> = CVar::new(None);

static KEYSEQ_CAP: CVar<usize> = CVar::new(0);
static KEYSEQ_LEN: CVar<usize> = CVar::new(0);

const ESC: c_int = 27;

pub fn same_fn(a: Option<rl_command_func_t>, b: rl_command_func_t) -> bool {
    a.map(|a| a as usize) == Some(b as usize)
}

/// Start a new key sequence (top-level dispatch).
pub fn keyseq_reset() {
    KEYSEQ_LEN.set(0);
    rl_key_sequence_length.set(0);
    unsafe {
        ensure_keyseq(16);
        *rl_executing_keyseq.get() = 0;
    }
}

unsafe fn ensure_keyseq(n: usize) {
    if KEYSEQ_CAP.get() < n + 1 || rl_executing_keyseq.get().is_null() {
        let cap = (n + 1).max(KEYSEQ_CAP.get() * 2).max(16);
        let p = xrealloc(rl_executing_keyseq.get() as *mut c_void, cap) as *mut c_char;
        if KEYSEQ_CAP.get() == 0 {
            *p = 0;
        }
        rl_executing_keyseq.set(p);
        KEYSEQ_CAP.set(cap);
    }
}

fn keyseq_push(k: c_int) {
    unsafe {
        let n = KEYSEQ_LEN.get();
        ensure_keyseq(n + 1);
        let p = rl_executing_keyseq.get();
        *p.add(n) = k as c_char;
        *p.add(n + 1) = 0;
        KEYSEQ_LEN.set(n + 1);
        rl_key_sequence_length.set((n + 1) as c_int);
    }
}

/// A key read by the numeric-argument loop.  The reference's callback
/// interface dispatches such keys one by one, so they show up in
/// rl_executing_keyseq there (not in readline()).
pub fn callback_arg_key(k: c_int) {
    if is_state(RL_STATE_CALLBACK) && k >= 0 {
        keyseq_push(k);
    }
}

/// The keys of the current sequence.
pub fn keyseq() -> Vec<u8> {
    unsafe {
        let p = rl_executing_keyseq.get();
        if p.is_null() {
            return Vec::new();
        }
        core::slice::from_raw_parts(p as *const u8, KEYSEQ_LEN.get()).to_vec()
    }
}

fn is_arg_func(f: rl_command_func_t) -> bool {
    let a = f as usize;
    a == crate::numarg::rl_digit_argument as *const () as usize
        || a == crate::numarg::rl_universal_argument as *const () as usize
        || a == crate::vi::rl_vi_arg_digit as *const () as usize
}

/// Commands being executed (innermost last).
static CMD_STACK: CVar<Vec<rl_command_func_t>> = CVar::new(Vec::new());

/// The innermost command being executed, if any.
pub fn current_command() -> Option<rl_command_func_t> {
    CMD_STACK.get_mut().last().copied()
}

/// Forget the command stack (the callback coroutine running the commands
/// was abandoned).
pub fn reset_command_stack() {
    CMD_STACK.get_mut().clear();
}

pub fn is_arg_command(f: rl_command_func_t) -> bool {
    is_arg_func(f)
}

/// Run a command with the current numeric argument.
pub unsafe fn call_command(f: rl_command_func_t, key: c_int, map: Keymap) -> c_int {
    rl_executing_keymap.set(map);
    rl_executing_key.set(key);
    // (commands dispatched from inside a command, e.g. by the numeric
    // argument loop, leave the outer dispatch state alone)
    // (an aborted command leaves DISPATCHING set but the stack empty)
    let nested = !CMD_STACK.get_mut().is_empty();
    let outer = nested && is_state(RL_STATE_DISPATCHING);
    let outer_disp = if nested { rl_dispatching.get() } else { 0 };
    set_state(RL_STATE_DISPATCHING);
    rl_dispatching.set(1);
    CMD_STACK.get_mut().push(f);
    let r = f(crate::numarg::count(), key);
    CMD_STACK.get_mut().pop();
    rl_dispatching.set(outer_disp);
    if !outer {
        unset_state(RL_STATE_DISPATCHING);
    }
    if !is_arg_func(f) {
        rl_last_func.set(Some(f));
    }
    r
}

/// Dispatch `key` in `map`: the entry point for one key of the main loop
/// (after `keyseq_reset`) and for keys inside prefixes.
pub unsafe fn dispatch_key(key: c_int, map: Keymap) -> c_int {
    let mut map = map;
    let mut key = key;
    if key < 0 {
        return -1;
    }
    keyseq_push(key);
    if key >= 128 && crate::vars::convert_meta.get() != 0 {
        let e = *keymap::entry(map, ESC as usize);
        if e.type_ == ISKMAP {
            map = e.function.map;
            key &= 0x7f;
        }
    }
    let e = *keymap::entry(map, key as usize);
    match e.type_ {
        ISFUNC => match e.function.func {
            Some(f) => call_command(f, key, map),
            None => {
                // unbound key
                unbound_key();
                -1
            }
        },
        ISKMAP => dispatch_prefix(key, e.function.map),
        ISMACR => {
            let m = cstr(e.function.macro_).to_vec();
            crate::kbdmacro::push_macro_input(&m);
            0
        }
        _ => -1,
    }
}

unsafe fn unbound_key() {
    crate::terminal::rl_ding();
    crate::numarg::reset();
}

/// Inside a prefix map `sub` (reached with `key`): read the next key and
/// resolve it, falling back to the map's ANYOTHERKEY binding.
unsafe fn dispatch_prefix(key: c_int, sub: Keymap) -> c_int {
    let other = *sub.add(ANYOTHERKEY);
    let has_other = !other.is_null();
    if has_other {
        let t = crate::vars::keyseq_timeout.get();
        if t > 0 && !input::input_available(t as i64 * 1000) {
            return run_anyother(key, sub, other);
        }
    }
    // callback mode shows a pending key sequence as MULTIKEY (until the
    // sequence is complete); readline() reads the next key with MOREINPUT
    let cb = is_state(RL_STATE_CALLBACK);
    if cb {
        set_state(RL_STATE_MULTIKEY);
    } else {
        set_state(RL_STATE_MOREINPUT);
    }
    let next = crate::input::rl_read_key();
    if !cb {
        unset_state(RL_STATE_MOREINPUT);
    }
    if next < 0 {
        return -1;
    }
    // does `next` lead anywhere in `sub`?
    let mut nk = next;
    let mut m = sub;
    if nk >= 128 && crate::vars::convert_meta.get() != 0 {
        let e = *keymap::entry(m, ESC as usize);
        if e.type_ == ISKMAP {
            m = e.function.map;
            nk &= 0x7f;
        }
    }
    let e = *keymap::entry(m, nk as usize);
    if e.is_null() && e.type_ == ISFUNC {
        if has_other {
            // run the prefix's own binding, then re-read `next`
            crate::input::rl_execute_next(next);
            return run_anyother(key, sub, other);
        }
        keyseq_push(next);
        unbound_key();
        return -1;
    }
    dispatch_key(next, sub)
}

unsafe fn run_anyother(key: c_int, sub: Keymap, other: KEYMAP_ENTRY) -> c_int {
    match other.type_ {
        ISFUNC => match other.function.func {
            Some(f) => call_command(f, key, sub),
            None => -1,
        },
        ISMACR => {
            let m = cstr(other.function.macro_).to_vec();
            crate::kbdmacro::push_macro_input(&m);
            0
        }
        ISKMAP => dispatch_prefix(key, other.function.map),
        _ => -1,
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_do_lowercase_version(_count: c_int, key: c_int) -> c_int {
    let lower = (key as u8).to_ascii_lowercase() as c_int;
    if lower == key {
        crate::terminal::rl_ding();
        return 1;
    }
    let map = rl_executing_keymap.get();
    let map = if map.is_null() { keymap::current() } else { map };
    // replace the key in the sequence
    let n = KEYSEQ_LEN.get();
    if n > 0 {
        KEYSEQ_LEN.set(n - 1);
    }
    dispatch_key(lower, map)
}

/// Skip a CSI sequence's parameters and final byte.
#[no_mangle]
pub unsafe extern "C" fn rl_skip_csi_sequence(_count: c_int, _key: c_int) -> c_int {
    set_state(RL_STATE_MOREINPUT);
    loop {
        let c = crate::input::rl_read_key();
        if c < 0 || !(0x20..0x40).contains(&c) {
            break;
        }
    }
    unset_state(RL_STATE_MOREINPUT);
    0
}

/// Is the current dispatch (still) inside a numeric argument?
pub fn in_numeric_arg() -> bool {
    is_state(RL_STATE_NUMERICARG)
}
