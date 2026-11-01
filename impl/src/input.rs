//! Character input: `rl_getc` (the default `rl_getc_function`),
//! `rl_read_key` (pending input, stuffed characters, macros, event hook),
//! timeouts, input availability.
//!
//! Every byte from the terminal goes through `rl_getc_function`; all key
//! reading inside the library goes through `read_key()` (= `rl_read_key`),
//! which is also the single suspension point of the callback interface
//! (see callback.rs).
//! Owner: input workstream.
use crate::prelude::*;
use crate::readline::{rl_instream, rl_readline_state};
use std::collections::VecDeque;

#[no_mangle]
pub static rl_getc_function: CVar<Option<rl_getc_func_t>> = CVar::new(Some(rl_getc));
#[no_mangle]
pub static rl_event_hook: CVar<Option<rl_hook_func_t>> = CVar::new(None);
#[no_mangle]
pub static rl_input_available_hook: CVar<Option<rl_hook_func_t>> = CVar::new(None);
#[no_mangle]
pub static rl_timeout_event_hook: CVar<Option<rl_hook_func_t>> = CVar::new(None);
/// A key to be read next (rl_execute_next); 0 = none.
#[no_mangle]
pub static rl_pending_input: CVar<c_int> = CVar::new(0);

/// rl_stuff_char buffer.
static IBUFFER: CVar<VecDeque<u8>> = CVar::new(VecDeque::new());
const IBUFFER_SIZE: usize = 512;

/// Microseconds to wait for "is more input available" (rl_set_keyboard_input_timeout).
pub static KEYBOARD_INPUT_TIMEOUT: CVar<c_int> = CVar::new(100_000);

/// Whole-call timeout (rl_set_timeout): duration and the deadline of the
/// current readline call.
static TIMEOUT_US: CVar<u64> = CVar::new(0);
static DEADLINE: CVar<Option<std::time::Instant>> = CVar::new(None);

pub fn set_state(bits: u64) {
    rl_readline_state.set(rl_readline_state.get() | bits as c_ulong);
}
pub fn unset_state(bits: u64) {
    rl_readline_state.set(rl_readline_state.get() & !(bits as c_ulong));
}
pub fn is_state(bits: u64) -> bool {
    rl_readline_state.get() & bits as c_ulong != 0
}

unsafe fn instream_fd(stream: *mut FILE) -> c_int {
    if stream.is_null() {
        0
    } else {
        libc::fileno(stream)
    }
}

/// Wait until `fd` is readable or `timeout_us` passes (negative: forever).
/// Returns >0 readable, 0 timeout, <0 error (errno set).
pub unsafe fn wait_readable(fd: c_int, timeout_us: i64) -> c_int {
    let mut fds: libc::fd_set = core::mem::zeroed();
    libc::FD_ZERO(&mut fds);
    libc::FD_SET(fd, &mut fds);
    let mut tv = libc::timeval {
        tv_sec: (timeout_us.max(0) / 1_000_000) as _,
        tv_usec: (timeout_us.max(0) % 1_000_000) as _,
    };
    let tvp = if timeout_us < 0 { null_mut() } else { &mut tv as *mut _ };
    libc::select(fd + 1, &mut fds, null_mut(), null_mut(), tvp)
}

/// Called at the start of readline(): arms the rl_set_timeout deadline.
pub fn start_timeout() {
    unset_state(RL_STATE_TIMEOUT);
    let us = TIMEOUT_US.get();
    DEADLINE.set(if us == 0 {
        None
    } else {
        Some(std::time::Instant::now() + std::time::Duration::from_micros(us))
    });
}

fn remaining_us() -> Option<i64> {
    DEADLINE.get().map(|d| {
        let now = std::time::Instant::now();
        if d <= now {
            0
        } else {
            (d - now).as_micros() as i64
        }
    })
}

/// A read was interrupted by a signal (or one is pending): fatal signals
/// end the read (Some(READERR/EOF)), the others are processed and the
/// application's signal event hook runs (None: read again).
unsafe fn interrupted() -> Option<c_int> {
    let sig = crate::signals::rl_pending_signal();
    if sig == libc::SIGTERM || sig == libc::SIGHUP {
        return Some(if is_state(RL_STATE_READCMD) { READERR } else { EOF });
    }
    crate::signals::rl_check_signals();
    if let Some(h) = crate::signals::rl_signal_event_hook.get() {
        h();
    }
    None
}

#[no_mangle]
pub unsafe extern "C" fn rl_getc(stream: *mut FILE) -> c_int {
    let fd = instream_fd(stream);
    loop {
        // a signal that arrived since the last read counts as an
        // interrupted read
        if crate::signals::rl_pending_signal() != 0 {
            if let Some(r) = interrupted() {
                return r;
            }
            continue;
        }
        if let Some(rem) = remaining_us() {
            let r = if rem == 0 { 0 } else { wait_readable(fd, rem) };
            if r == 0 {
                if let Some(h) = rl_timeout_event_hook.get() {
                    h();
                }
                set_state(RL_STATE_TIMEOUT);
                return READERR;
            }
            if r < 0 && *libc_errno() == libc::EINTR {
                if let Some(r) = interrupted() {
                    return r;
                }
                continue;
            }
        }
        let mut c: u8 = 0;
        let n = libc::read(fd, &mut c as *mut u8 as *mut c_void, 1);
        if n == 1 {
            return c as c_int;
        }
        if n == 0 {
            return EOF;
        }
        let e = *libc_errno();
        if e == libc::EAGAIN || e == libc::EWOULDBLOCK {
            // make the descriptor blocking again
            let fl = libc::fcntl(fd, libc::F_GETFL, 0);
            if fl >= 0 && fl & libc::O_NONBLOCK != 0 {
                libc::fcntl(fd, libc::F_SETFL, fl & !libc::O_NONBLOCK);
                continue;
            }
            return EOF;
        }
        if e == libc::EINTR {
            if let Some(r) = interrupted() {
                return r;
            }
            continue;
        }
        return if is_state(RL_STATE_READCMD) { READERR } else { EOF };
    }
}

pub unsafe fn libc_errno() -> *mut c_int {
    #[cfg(target_os = "macos")]
    {
        libc::__error()
    }
    #[cfg(not(target_os = "macos"))]
    {
        libc::__errno_location()
    }
}

/// Input queued inside the library (pushed keys, stuffed characters,
/// macro text): reading it never blocks.
pub fn internal_input_pending() -> bool {
    rl_pending_input.get() != 0 || !IBUFFER.get_mut().is_empty() || crate::kbdmacro::macro_input_pending()
}

/// Is input available without blocking (within `timeout_us`)?  Considers
/// pending/stuffed/macro input first, then the input hook or the stream.
pub unsafe fn input_available(timeout_us: i64) -> bool {
    if rl_pending_input.get() != 0 || !IBUFFER.get_mut().is_empty() || crate::kbdmacro::macro_input_pending() {
        return true;
    }
    if let Some(h) = rl_input_available_hook.get() {
        return h() != 0;
    }
    let fd = instream_fd(rl_instream.get());
    wait_readable(fd, timeout_us) > 0
}

/// Typeahead for self-insert: terminal input is waiting and nothing was
/// pushed back into the input (pushed-back keys are dispatched one by one).
pub unsafe fn typeahead_available() -> bool {
    !internal_input_pending() && input_available(0)
}

/// Read one byte through rl_getc_function (no pending input, no macros).
pub unsafe fn getc_raw() -> c_int {
    crate::callback::before_getc();
    let f = rl_getc_function.get().unwrap_or(rl_getc);
    let stream = rl_instream.get();
    f(stream)
}

/// The next key: pending input, stuffed characters, macro text, else the
/// terminal (calling rl_event_hook while waiting).  Records keys while a
/// keyboard macro is being defined.
pub unsafe fn read_key() -> c_int {
    let c;
    let p = rl_pending_input.get();
    if p != 0 {
        rl_pending_input.set(0);
        unset_state(RL_STATE_INPUTPENDING);
        c = p;
    } else if let Some(m) = crate::kbdmacro::next_macro_key() {
        c = m as c_int;
        // keys coming from a macro are not recorded again
        return c;
    } else if let Some(b) = IBUFFER.get_mut().pop_front() {
        c = b as c_int;
    } else {
        // with an event hook: call it while waiting for input (it is
        // called before the first check)
        loop {
            // the hook may have been unset meanwhile
            let Some(h) = rl_event_hook.get() else { break };
            h();
            if crate::readline::rl_done.get() != 0 {
                return 0;
            }
            if rl_pending_input.get() != 0 || !IBUFFER.get_mut().is_empty() {
                return read_key();
            }
            if input_available(KEYBOARD_INPUT_TIMEOUT.get() as i64) {
                break;
            }
        }
        c = getc_raw();
    }
    if c >= 0 && is_state(RL_STATE_MACRODEF) {
        crate::kbdmacro::record_key(c as u8);
    }
    c
}

#[no_mangle]
pub unsafe extern "C" fn rl_read_key() -> c_int {
    read_key()
}

#[no_mangle]
pub unsafe extern "C" fn rl_stuff_char(key: c_int) -> c_int {
    let b = IBUFFER.get_mut();
    if b.len() >= IBUFFER_SIZE - 1 {
        return 0;
    }
    if key == EOF {
        // a newline, and EOF as the pending input
        b.push_back(b'\n');
        rl_pending_input.set(EOF);
    } else {
        b.push_back(key as u8);
    }
    set_state(RL_STATE_INPUTPENDING);
    1
}

/// Push bytes to the front of the input (read before anything else).
pub fn unread_keys(keys: &[u8]) {
    let b = IBUFFER.get_mut();
    for &k in keys.iter().rev() {
        b.push_front(k);
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_execute_next(c: c_int) -> c_int {
    rl_pending_input.set(c);
    set_state(RL_STATE_INPUTPENDING);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_clear_pending_input() -> c_int {
    rl_pending_input.set(0);
    unset_state(RL_STATE_INPUTPENDING);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_set_keyboard_input_timeout(u: c_int) -> c_int {
    let old = KEYBOARD_INPUT_TIMEOUT.get();
    if u >= 0 {
        KEYBOARD_INPUT_TIMEOUT.set(u);
    }
    old
}

#[no_mangle]
pub unsafe extern "C" fn rl_set_timeout(secs: c_uint, usecs: c_uint) -> c_int {
    let us = secs as u64 * 1_000_000 + usecs as u64;
    TIMEOUT_US.set(us);
    if us == 0 {
        DEADLINE.set(None);
    } else {
        DEADLINE.set(Some(std::time::Instant::now() + std::time::Duration::from_micros(us)));
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_timeout_remaining(secs: *mut c_uint, usecs: *mut c_uint) -> c_int {
    let Some(rem) = remaining_us() else { return -1 };
    if !secs.is_null() {
        *secs = (rem / 1_000_000) as c_uint;
    }
    if !usecs.is_null() {
        *usecs = (rem % 1_000_000) as c_uint;
    }
    if rem == 0 {
        0
    } else {
        1
    }
}
