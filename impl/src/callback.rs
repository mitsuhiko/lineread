//! The alternate (callback) interface.
//!
//! Design: the ordinary blocking editing loop (readline.rs line_setup /
//! line_step / line_teardown) runs on a coroutine (its own stack, same
//! thread; c/shim.c, ucontext).  Whenever it needs a key and none is
//! available it yields back to `rl_callback_read_char`, which returns to
//! the application.  So every command can be written in blocking style
//! (read keys with rl_read_key) and still works in callback mode, with the
//! state flags (MULTIKEY, ISEARCH, NUMERICARG, ...) visible between calls.
//! The line handler always runs on the application's stack.
//! Owner: callback workstream.
use crate::input::{self, is_state, set_state, unset_state};
use crate::prelude::*;

#[repr(C)]
pub struct LrCoro {
    _private: [u8; 0],
}

extern "C" {
    fn lr_coro_new(stack: usize, f: unsafe extern "C" fn(*mut c_void), arg: *mut c_void) -> *mut LrCoro;
    fn lr_coro_resume(c: *mut LrCoro);
    fn lr_coro_yield(c: *mut LrCoro);
    fn lr_coro_done(c: *mut LrCoro) -> c_int;
}

const STACK_SIZE: usize = 8 << 20;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum WState {
    /// suspended waiting for input
    Waiting,
    /// a line is finished (LINE holds it)
    LineDone,
    /// not started / finished
    Idle,
}

static HANDLER: CVar<Option<rl_vcpfunc_t>> = CVar::new(None);
static CORO: CVar<*mut LrCoro> = CVar::new(null_mut());
/// Abandoned coroutines (their stacks stay allocated: frames on them may
/// own memory, and leak checkers scan them).
static GRAVEYARD: CVar<Vec<*mut LrCoro>> = CVar::new(Vec::new());
/// Running on the coroutine right now.
static ON_WORKER: CVar<bool> = CVar::new(false);
static STATE: CVar<WState> = CVar::new(WState::Idle);
/// Reads allowed without checking for available input in this call.
static BUDGET: CVar<c_int> = CVar::new(0);
static LINE: CVar<*mut c_char> = CVar::new(null_mut());

/// Yield to rl_callback_read_char (we are on the worker).
unsafe fn suspend(st: WState) {
    STATE.set(st);
    ON_WORKER.set(false);
    lr_coro_yield(CORO.get());
    ON_WORKER.set(true);
}

/// Before reading the next top-level key: give control back to the
/// application unless input is already there.
pub fn wait_for_key() {
    unsafe {
        if !ON_WORKER.get() {
            return;
        }
        if BUDGET.get() > 0 || input::internal_input_pending() {
            return;
        }
        suspend(WState::Waiting);
    }
}

/// Called before every rl_getc_function call: inside a command that needs
/// more input, suspend until the next rl_callback_read_char.
pub fn before_getc() {
    unsafe {
        if !ON_WORKER.get() {
            return;
        }
        if BUDGET.get() > 0 {
            BUDGET.set(BUDGET.get() - 1);
            return;
        }
        if input::input_available(0) {
            return;
        }
        suspend(WState::Waiting);
        BUDGET.set(BUDGET.get() - 1);
    }
}

/// Is the callback interface active (a handler installed)?
pub fn active() -> bool {
    HANDLER.get().is_some()
}

unsafe extern "C" fn worker(_arg: *mut c_void) {
    ON_WORKER.set(true);
    loop {
        crate::readline::line_setup();
        loop {
            wait_for_key();
            if !crate::readline::line_step() {
                break;
            }
        }
        let line = crate::readline::line_teardown();
        LINE.set(line);
        suspend(WState::LineDone);
        // next line
        crate::readline::rl_initialize();
        crate::tty::prep();
    }
}

unsafe fn abandon() {
    let c = CORO.replace(null_mut());
    if !c.is_null() {
        GRAVEYARD.get_mut().push(c);
    }
    STATE.set(WState::Idle);
}

unsafe fn start_worker() {
    abandon();
    let c = lr_coro_new(STACK_SIZE, worker, null_mut());
    if c.is_null() {
        return;
    }
    CORO.set(c);
    // run the line setup (prompt) up to the first read
    BUDGET.set(0);
    resume();
}

unsafe fn resume() {
    let c = CORO.get();
    if c.is_null() {
        return;
    }
    ON_WORKER.set(true);
    lr_coro_resume(c);
    ON_WORKER.set(false);
    if lr_coro_done(c) != 0 {
        STATE.set(WState::Idle);
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_callback_handler_install(prompt: *const c_char, handler: Option<rl_vcpfunc_t>) {
    crate::display::rl_set_prompt(prompt);
    set_state(RL_STATE_CALLBACK);
    HANDLER.set(handler);
    crate::readline::rl_initialize();
    crate::tty::prep();
    start_worker();
}

#[no_mangle]
pub unsafe extern "C" fn rl_callback_read_char() {
    if HANDLER.get().is_none() || CORO.get().is_null() {
        let m = b"readline: readline_callback_read_char() called with no handler!\n";
        crate::util::fwrite_bytes(crate::util::lr_stderr(), m);
        return;
    }
    BUDGET.set(1);
    let my = CORO.get();
    resume();
    BUDGET.set(0);
    if CORO.get() != my {
        return;
    }
    if STATE.get() == WState::LineDone {
        let line = LINE.replace(null_mut());
        crate::tty::deprep();
        if let Some(h) = HANDLER.get() {
            h(line);
        } else {
            cfree(line);
        }
        // handler still installed (and not reinstalled): prepare the next line
        if HANDLER.get().is_some() && CORO.get() == my {
            BUDGET.set(0);
            resume();
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_callback_handler_remove() {
    HANDLER.set(None);
    unset_state(RL_STATE_CALLBACK);
    abandon();
    crate::tty::deprep();
}

#[no_mangle]
pub unsafe extern "C" fn rl_callback_sigcleanup() {
    if !is_state(RL_STATE_CALLBACK) {
        return;
    }
    unset_state(RL_STATE_ISEARCH | RL_STATE_NSEARCH | RL_STATE_MULTIKEY | RL_STATE_NUMERICARG | RL_STATE_VIMOTION | RL_STATE_CHARSEARCH | RL_STATE_MOREINPUT);
    // restart the line from scratch on the next call
    if !CORO.get().is_null() && STATE.get() == WState::Waiting {
        abandon();
        let c = lr_coro_new(STACK_SIZE, worker_resume_line, null_mut());
        CORO.set(c);
        BUDGET.set(0);
        resume();
    }
}

/// Worker for a line that is already set up (after rl_callback_sigcleanup).
unsafe extern "C" fn worker_resume_line(_arg: *mut c_void) {
    ON_WORKER.set(true);
    let mut first = true;
    loop {
        if !first {
            crate::readline::line_setup();
        }
        first = false;
        loop {
            wait_for_key();
            if !crate::readline::line_step() {
                break;
            }
        }
        let line = crate::readline::line_teardown();
        LINE.set(line);
        suspend(WState::LineDone);
        crate::readline::rl_initialize();
        crate::tty::prep();
    }
}
