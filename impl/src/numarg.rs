//! Numeric arguments: digit-argument, universal-argument, the argument
//! loop, rl_explicit_arg / rl_numeric_arg.
//! Owner: emacs workstream.
//!
//! Model (observed): the first argument command starts a loop that reads
//! keys while they extend the argument.  Plain digits replace the default
//! argument or append to an explicit one; `-` before any digit makes it
//! negative; a single key bound to universal-argument multiplies by four
//! (or, after digits, ends the argument and is otherwise ignored).  Any
//! other key is dispatched normally with the argument; if that runs an
//! argument command again (M-digit, a multi-key universal-argument), the
//! loop continues: M-digit always appends, universal-argument multiplies.
//! An argument above 1000000 is discarded.
use crate::input::{is_state, set_state, unset_state};
use crate::prelude::*;

#[no_mangle]
pub static rl_explicit_arg: CVar<c_int> = CVar::new(0);
#[no_mangle]
pub static rl_numeric_arg: CVar<c_int> = CVar::new(1);
/// Sign of the argument (`M--`).
pub static ARG_SIGN: CVar<c_int> = CVar::new(1);

/// What a nested argument command asks the loop to do.
#[derive(Clone, Copy, PartialEq)]
enum Nested {
    None,
    /// keep reading the argument
    Continue,
    /// the argument was consumed or discarded: leave the loop
    Done,
}
static NESTED: CVar<Nested> = CVar::new(Nested::None);

const MAX_ARG: c_int = 1_000_000;

/// The count passed to the next command.
pub fn count() -> c_int {
    rl_numeric_arg.get().wrapping_mul(ARG_SIGN.get())
}

/// Reset after a command consumed the argument.
pub fn reset() {
    rl_numeric_arg.set(1);
    rl_explicit_arg.set(0);
    ARG_SIGN.set(1);
    unset_state(RL_STATE_NUMERICARG);
}

#[no_mangle]
pub unsafe extern "C" fn rl_discard_argument() -> c_int {
    crate::terminal::rl_ding();
    reset();
    crate::display::clear_arg_message();
    0
}

/// Set the argument to `n`; discards it (false) when it gets too large.
fn set_arg(n: c_int) -> bool {
    if n > MAX_ARG {
        unsafe {
            rl_discard_argument();
        }
        return false;
    }
    rl_numeric_arg.set(n);
    rl_explicit_arg.set(1);
    true
}

fn is_universal(f: Option<rl_command_func_t>) -> bool {
    f.map(|f| f as usize) == Some(rl_universal_argument as *const () as usize)
}

/// Read keys while they extend the argument, then dispatch the first
/// other key with it.  Runs only in the outermost argument command.
unsafe fn arg_loop() -> c_int {
    set_state(RL_STATE_NUMERICARG);
    loop {
        crate::display::show_arg_message();
        let c = crate::input::rl_read_key();
        if c < 0 {
            // the message stays; the key loop sees the end of file
            reset();
            crate::input::rl_execute_next(c);
            return 0;
        }
        let map = crate::keymap::current();
        let e = *crate::keymap::entry(map, (c as usize) & 0xff);
        if e.type_ == ISFUNC && is_universal(e.function.func) {
            if rl_explicit_arg.get() != 0 {
                // ends the argument; the next key gets it
                crate::display::clear_arg_message();
                crate::display::redisplay();
                let k = crate::input::rl_read_key();
                if k < 0 {
                    reset();
                    crate::input::rl_execute_next(k);
                    return 0;
                }
                return dispatch_with_arg(k);
            }
            if !set_arg_keep_explicit(rl_numeric_arg.get().saturating_mul(4)) {
                return 0;
            }
            continue;
        }
        let plain = c as u8;
        if c < 256 && plain.is_ascii_digit() {
            let d = (plain - b'0') as c_int;
            let n = if rl_explicit_arg.get() != 0 { rl_numeric_arg.get().saturating_mul(10).saturating_add(d) } else { d };
            if !set_arg(n) {
                return 0;
            }
            continue;
        }
        if plain == b'-' && rl_explicit_arg.get() == 0 {
            ARG_SIGN.set(-1);
            rl_numeric_arg.set(1);
            continue;
        }
        let r = dispatch_with_arg(c);
        match NESTED.replace(Nested::None) {
            Nested::Continue => continue,
            _ => return r,
        }
    }
}

/// Like set_arg but universal-argument does not make the argument explicit.
fn set_arg_keep_explicit(n: c_int) -> bool {
    let explicit = rl_explicit_arg.get();
    if !set_arg(n) {
        return false;
    }
    rl_explicit_arg.set(explicit);
    true
}

/// Leave the argument loop and run `c` with the argument.
unsafe fn dispatch_with_arg(c: c_int) -> c_int {
    crate::display::clear_arg_message();
    crate::display::redisplay();
    NESTED.set(Nested::None);
    if ARG_SIGN.get() < 0 {
        // a negative argument is explicit
        rl_explicit_arg.set(1);
    }
    crate::dispatch::dispatch_key(c, crate::keymap::current())
}

#[no_mangle]
pub unsafe extern "C" fn rl_digit_argument(_count: c_int, key: c_int) -> c_int {
    let k = (key & 0x7f) as u8;
    if is_state(RL_STATE_NUMERICARG) {
        // M-digit / M-- while reading an argument
        if k.is_ascii_digit() {
            let n = rl_numeric_arg.get().saturating_mul(10).saturating_add((k - b'0') as c_int);
            NESTED.set(if set_arg(n) { Nested::Continue } else { Nested::Done });
            return 0;
        }
        if k == b'-' && rl_explicit_arg.get() == 0 {
            ARG_SIGN.set(-1);
            rl_numeric_arg.set(1);
            NESTED.set(Nested::Continue);
            return 0;
        }
        // not part of the argument: run the plain key with it
        NESTED.set(Nested::None);
        let r = crate::dispatch::dispatch_key(k as c_int, crate::keymap::current());
        if NESTED.get() == Nested::None {
            NESTED.set(Nested::Done);
        }
        return r;
    }
    if k == b'-' {
        ARG_SIGN.set(-1);
        rl_numeric_arg.set(1);
    } else if k.is_ascii_digit() {
        set_arg((k - b'0') as c_int);
    }
    arg_loop()
}

#[no_mangle]
pub unsafe extern "C" fn rl_universal_argument(_count: c_int, _key: c_int) -> c_int {
    let ok = set_arg_keep_explicit(rl_numeric_arg.get().saturating_mul(4));
    if is_state(RL_STATE_NUMERICARG) {
        NESTED.set(if ok { Nested::Continue } else { Nested::Done });
        return 0;
    }
    if !ok {
        return 0;
    }
    arg_loop()
}
