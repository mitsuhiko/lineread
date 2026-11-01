//! Signal handling: readline's handlers for INT, QUIT, TERM, HUP, ALRM,
//! TSTP, TTIN, TTOU (rl_catch_signals) and WINCH (rl_catch_sigwinch).
//!
//! The handler only records the signal; it is processed at the next safe
//! point (`rl_check_signals`: while reading input, after a command): the
//! line state is cleaned up (SIGINT), the signal character echoed, the
//! terminal restored and the application's handler run by re-sending the
//! signal, then readline's terminal settings and handlers are restored.
//! Owner: bindings & input workstream.
use crate::input::{is_state, set_state, unset_state};
use crate::prelude::*;
use core::sync::atomic::{AtomicI32, Ordering};

#[no_mangle]
pub static rl_catch_signals: CVar<c_int> = CVar::new(1);
#[no_mangle]
pub static rl_catch_sigwinch: CVar<c_int> = CVar::new(1);
#[no_mangle]
pub static rl_change_environment: CVar<c_int> = CVar::new(1);
#[no_mangle]
pub static rl_persistent_signal_handlers: CVar<c_int> = CVar::new(0);
#[no_mangle]
pub static rl_signal_event_hook: CVar<Option<rl_hook_func_t>> = CVar::new(None);

/// The signal caught by readline's handler and not yet processed.
static CAUGHT: AtomicI32 = AtomicI32::new(0);

/// Number of signals (other than SIGWINCH) processed so far; lets long
/// running commands (the completion pager) notice an interruption.
pub static SIGNALS_HANDLED: CVar<u64> = CVar::new(0);

const CAUGHT_SIGNALS: [c_int; 8] = [
    libc::SIGINT,
    libc::SIGTERM,
    libc::SIGHUP,
    libc::SIGQUIT,
    libc::SIGALRM,
    libc::SIGTSTP,
    libc::SIGTTIN,
    libc::SIGTTOU,
];

const NSIG: usize = 65;

/// The application's dispositions saved when ours were installed.
static OLD: CVar<[Option<libc::sigaction>; NSIG]> = CVar::new([None; NSIG]);
static SIGNALS_SET: CVar<bool> = CVar::new(false);
static SIGWINCH_SET: CVar<bool> = CVar::new(false);
/// Processing a signal right now (no recursion).
static HANDLING: CVar<bool> = CVar::new(false);

extern "C" fn handler(sig: c_int) {
    CAUGHT.store(sig, Ordering::SeqCst);
}

unsafe fn install(sig: c_int) {
    let mut sa: libc::sigaction = core::mem::zeroed();
    sa.sa_sigaction = handler as extern "C" fn(c_int) as usize;
    sa.sa_flags = 0;
    libc::sigemptyset(&mut sa.sa_mask);
    let mut old: libc::sigaction = core::mem::zeroed();
    libc::sigaction(sig, &sa, &mut old);
    if old.sa_sigaction == libc::SIG_IGN {
        // signals the application ignores stay ignored
        libc::sigaction(sig, &old, null_mut());
    }
    OLD.get_mut()[sig as usize] = Some(old);
}

unsafe fn restore(sig: c_int) {
    if let Some(old) = OLD.get_mut()[sig as usize].take() {
        libc::sigaction(sig, &old, null_mut());
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_set_signals() -> c_int {
    if rl_catch_signals.get() != 0 && !SIGNALS_SET.get() {
        for s in CAUGHT_SIGNALS {
            install(s);
        }
        SIGNALS_SET.set(true);
    }
    if rl_catch_sigwinch.get() != 0 && !SIGWINCH_SET.get() {
        install(libc::SIGWINCH);
        SIGWINCH_SET.set(true);
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_clear_signals() -> c_int {
    if SIGNALS_SET.get() {
        for s in CAUGHT_SIGNALS {
            restore(s);
        }
        SIGNALS_SET.set(false);
    }
    if SIGWINCH_SET.get() {
        restore(libc::SIGWINCH);
        SIGWINCH_SET.set(false);
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_pending_signal() -> c_int {
    CAUGHT.load(Ordering::SeqCst)
}

#[no_mangle]
pub unsafe extern "C" fn rl_check_signals() {
    let sig = CAUGHT.load(Ordering::SeqCst);
    if sig != 0 && !HANDLING.get() {
        handle_signal(sig);
    }
}

/// Re-send `sig` to the process with the application's disposition in
/// place (readline's handlers are cleared at this point).
unsafe fn resend(sig: c_int) {
    let mut set: libc::sigset_t = core::mem::zeroed();
    let mut old: libc::sigset_t = core::mem::zeroed();
    libc::sigemptyset(&mut set);
    libc::sigaddset(&mut set, sig);
    libc::sigprocmask(libc::SIG_UNBLOCK, &set, &mut old);
    libc::kill(libc::getpid(), sig);
    libc::sigprocmask(libc::SIG_SETMASK, &old, null_mut());
}

unsafe fn handle_signal(sig: c_int) {
    HANDLING.set(true);
    CAUGHT.store(0, Ordering::SeqCst);
    set_state(RL_STATE_SIGHANDLER);
    if sig == libc::SIGWINCH {
        rl_resize_after_winch();
        // the application's handler, if any
        if let Some(old) = OLD.get_mut()[sig as usize] {
            let h = old.sa_sigaction;
            if h != libc::SIG_IGN && h != libc::SIG_DFL {
                let f: extern "C" fn(c_int) = core::mem::transmute(h);
                f(sig);
            }
        }
        // the signal is not re-sent: tell the application here
        if let Some(h) = rl_signal_event_hook.get() {
            h();
        }
    } else {
        SIGNALS_HANDLED.set(SIGNALS_HANDLED.get() + 1);
        if sig == libc::SIGINT {
            rl_free_line_state();
            // a pending vi motion is forgotten too (observed)
            unset_state(RL_STATE_VIMOTION);
        }
        rl_echo_signal_char(sig);
        rl_cleanup_after_signal();
        resend(sig);
        rl_reset_after_signal();
    }
    unset_state(RL_STATE_SIGHANDLER);
    HANDLING.set(false);
}

unsafe fn rl_resize_after_winch() {
    crate::terminal::rl_resize_terminal();
}

/// Echo the tty character that generates `sig` (`^C`) when the terminal
/// echoes control characters and echo-control-characters is on.  The
/// cursor stays where the display expects it.
#[no_mangle]
pub unsafe extern "C" fn rl_echo_signal_char(sig: c_int) {
    if crate::vars::echo_control_characters.get() == 0 || !crate::tty::echoctl() {
        return;
    }
    let sp = crate::tty::SPECIALS.get();
    let c = match sig {
        libc::SIGINT => sp.intr,
        libc::SIGQUIT => sp.quit,
        libc::SIGTSTP => sp.susp,
        _ => return,
    };
    let mut v = Vec::new();
    if c < 0x20 || c == 0x7f {
        v.push(b'^');
        v.push(if c == 0x7f { b'?' } else { c + 64 });
    } else {
        v.push(c);
    }
    let out = crate::readline::outstream();
    crate::terminal::flush();
    crate::util::fwrite_bytes(out, &v);
    crate::util::fflush(out);
}

#[no_mangle]
pub unsafe extern "C" fn rl_cleanup_after_signal() {
    crate::terminal::flush();
    crate::tty::deprep();
    crate::input::rl_clear_pending_input();
    rl_clear_signals();
}

#[no_mangle]
pub unsafe extern "C" fn rl_reset_after_signal() {
    crate::tty::prep();
    rl_set_signals();
}

/// Forget the partial state of the current line: undo list, numeric
/// argument, keyboard macro being defined.
#[no_mangle]
pub unsafe extern "C" fn rl_free_line_state() {
    crate::undo::rl_free_undo_list();
    // the argument's value is gone (the state flag stays until the
    // argument loop ends)
    crate::numarg::rl_numeric_arg.set(1);
    crate::numarg::rl_explicit_arg.set(0);
    crate::numarg::ARG_SIGN.set(1);
    // a message such as "(arg: 3) " goes away (at the next redisplay)
    crate::display::drop_message();
    if is_state(RL_STATE_MACRODEF) {
        unset_state(RL_STATE_MACRODEF);
    }
}
