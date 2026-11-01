//! Numeric arguments: digit-argument, universal-argument, the argument
//! loop, rl_explicit_arg / rl_numeric_arg.
//! Owner: emacs workstream.
use crate::input::{is_state, set_state, unset_state};
use crate::prelude::*;

#[no_mangle]
pub static rl_explicit_arg: CVar<c_int> = CVar::new(0);
#[no_mangle]
pub static rl_numeric_arg: CVar<c_int> = CVar::new(1);
/// Sign of the argument (`M--`).
pub static ARG_SIGN: CVar<c_int> = CVar::new(1);
/// A nested argument command ran inside the argument loop.
static CONTINUE: CVar<bool> = CVar::new(false);

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
    0
}

/// Add digit `d` to the argument.
fn add_digit(d: c_int) {
    if rl_explicit_arg.get() != 0 {
        rl_numeric_arg.set(rl_numeric_arg.get().saturating_mul(10).saturating_add(d).min(1_000_000));
    } else {
        rl_numeric_arg.set(d);
    }
    rl_explicit_arg.set(1);
}

/// Read keys while they extend the argument, then dispatch the first
/// other key with it.  Runs only in the outermost argument command.
unsafe fn arg_loop() -> c_int {
    loop {
        crate::display::show_arg_message();
        let c = crate::input::rl_read_key();
        if c < 0 {
            reset();
            return 0;
        }
        let k = c & 0x7f;
        let plain = c as u8;
        if plain.is_ascii_digit() {
            add_digit((plain - b'0') as c_int);
            continue;
        }
        if plain == b'-' && rl_explicit_arg.get() == 0 && ARG_SIGN.get() == 1 && !crate::input::is_state(0) {
            ARG_SIGN.set(-1);
            rl_explicit_arg.set(1);
            rl_numeric_arg.set(1);
            continue;
        }
        let _ = k;
        // not part of the argument: execute it with the argument
        crate::display::clear_arg_message();
        CONTINUE.set(false);
        let r = crate::dispatch::dispatch_key(c, crate::keymap::current());
        if CONTINUE.replace(false) {
            // the key was another argument command (M-1 M-2 ...)
            continue;
        }
        return r;
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_digit_argument(_count: c_int, key: c_int) -> c_int {
    let k = (key & 0x7f) as u8;
    let nested = is_state(RL_STATE_NUMERICARG);
    if k == b'-' {
        if rl_explicit_arg.get() == 0 {
            ARG_SIGN.set(-1);
            rl_explicit_arg.set(1);
            rl_numeric_arg.set(1);
        } else {
            // a second '-' is not part of the argument
            ARG_SIGN.set(-ARG_SIGN.get());
        }
    } else if k.is_ascii_digit() {
        add_digit((k - b'0') as c_int);
    }
    if nested {
        CONTINUE.set(true);
        return 0;
    }
    set_state(RL_STATE_NUMERICARG);
    arg_loop()
}

#[no_mangle]
pub unsafe extern "C" fn rl_universal_argument(_count: c_int, _key: c_int) -> c_int {
    let nested = is_state(RL_STATE_NUMERICARG);
    if rl_explicit_arg.get() == 0 && !nested {
        rl_numeric_arg.set(4);
    } else {
        rl_numeric_arg.set(rl_numeric_arg.get().saturating_mul(4).min(1_000_000));
    }
    if nested {
        CONTINUE.set(true);
        return 0;
    }
    set_state(RL_STATE_NUMERICARG);
    arg_loop()
}
