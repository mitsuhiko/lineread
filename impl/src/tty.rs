//! Terminal modes: rl_prep_terminal / rl_deprep_terminal (termios), the
//! tty special characters and their default bindings, echoing.
//! Owner: signals/tty workstream.
use crate::input::{is_state, set_state, unset_state};
use crate::prelude::*;

#[no_mangle]
pub static rl_prep_term_function: CVar<Option<rl_vintfunc_t>> = CVar::new(Some(rl_prep_terminal));
#[no_mangle]
pub static rl_deprep_term_function: CVar<Option<rl_voidfunc_t>> = CVar::new(Some(rl_deprep_terminal));

/// Saved terminal settings (from the last prep).
static SAVED: CVar<Option<libc::termios>> = CVar::new(None);
/// Whether the terminal echoes (readline echoes itself while prepped).
pub static ECHOING: CVar<c_int> = CVar::new(1);
/// The terminal's ECHOCTL flag at the last prep (signal characters are
/// echoed as `^C` only then).
static ECHOCTL: CVar<bool> = CVar::new(false);

pub fn echoctl() -> bool {
    ECHOCTL.get()
}

/// The tty's special characters (from the last tcgetattr).
#[derive(Clone, Copy)]
pub struct Specials {
    pub erase: u8,
    pub kill: u8,
    pub werase: u8,
    pub lnext: u8,
    pub eof: u8,
    pub intr: u8,
    pub quit: u8,
    pub susp: u8,
}
pub static SPECIALS: CVar<Specials> = CVar::new(Specials {
    erase: 0x7f,
    kill: 0x15,
    werase: 0x17,
    lnext: 0x16,
    eof: 4,
    intr: 3,
    quit: 0x1c,
    susp: 0x1a,
});

unsafe fn tty_fd() -> c_int {
    let s = crate::readline::rl_instream.get();
    if s.is_null() {
        0
    } else {
        libc::fileno(s)
    }
}

fn vdisable() -> u8 {
    #[cfg(target_os = "macos")]
    {
        0xff
    }
    #[cfg(not(target_os = "macos"))]
    {
        0
    }
}

unsafe fn read_specials(t: &libc::termios) {
    let cc = |i: usize| t.c_cc[i];
    SPECIALS.set(Specials {
        erase: cc(libc::VERASE),
        kill: cc(libc::VKILL),
        werase: cc(libc::VWERASE),
        lnext: cc(libc::VLNEXT),
        eof: cc(libc::VEOF),
        intr: cc(libc::VINTR),
        quit: cc(libc::VQUIT),
        susp: cc(libc::VSUSP),
    });
}

#[no_mangle]
pub unsafe extern "C" fn rl_prep_terminal(meta_flag: c_int) {
    if is_state(RL_STATE_TERMPREPPED) {
        return;
    }
    let fd = tty_fd();
    let mut t: libc::termios = core::mem::zeroed();
    if libc::tcgetattr(fd, &mut t) != 0 {
        // not a terminal: nothing to prepare
        ECHOING.set(1);
        return;
    }
    SAVED.set(Some(t));
    set_state(RL_STATE_TTYCSAVED);
    read_specials(&t);
    ECHOING.set((t.c_lflag & libc::ECHO != 0) as c_int);
    ECHOCTL.set(t.c_lflag & libc::ECHOCTL != 0);
    let mut n = t;
    n.c_lflag &= !(libc::ICANON | libc::ECHO);
    n.c_iflag &= !(libc::ICRNL | libc::INLCR);
    if meta_flag != 0 {
        n.c_iflag &= !libc::ISTRIP;
    } else {
        n.c_iflag |= libc::ISTRIP;
    }
    n.c_cc[libc::VMIN] = 1;
    n.c_cc[libc::VTIME] = 0;
    // let ^V and ^O reach readline
    n.c_cc[libc::VLNEXT] = vdisable();
    #[cfg(target_os = "macos")]
    {
        n.c_cc[libc::VDSUSP] = vdisable();
    }
    n.c_cc[libc::VDISCARD] = vdisable();
    if crate::signals::rl_catch_signals.get() == 0 {
        // ISIG stays on: the application handles the signals
    }
    libc::tcsetattr(fd, libc::TCSADRAIN, &n);
    set_state(RL_STATE_TERMPREPPED);
    if crate::vars::enable_keypad.get() != 0 {
        crate::terminal::out_cap("smkx");
    }
    if crate::vars::enable_bracketed_paste.get() != 0 && !crate::terminal::term().dumb {
        crate::terminal::out(b"\x1b[?2004h");
    }
    crate::terminal::flush();
}

#[no_mangle]
pub unsafe extern "C" fn rl_deprep_terminal() {
    if !is_state(RL_STATE_TERMPREPPED) {
        return;
    }
    if crate::vars::enable_bracketed_paste.get() != 0 && !crate::terminal::term().dumb {
        crate::terminal::out(b"\x1b[?2004l\r");
        // the carriage return moved the cursor to the first column
        crate::display::SCREEN.get_mut().cur_col = 0;
    }
    if crate::vars::enable_keypad.get() != 0 {
        crate::terminal::out_cap("rmkx");
    }
    crate::terminal::flush();
    if let Some(t) = SAVED.get() {
        libc::tcsetattr(tty_fd(), libc::TCSADRAIN, &t);
    }
    unset_state(RL_STATE_TERMPREPPED);
}

/// Call the (possibly application supplied) prep function.
pub unsafe fn prep() {
    if let Some(f) = rl_prep_term_function.get() {
        f(crate::vars::input_meta.get());
    }
}

/// Call the (possibly application supplied) deprep function.
pub unsafe fn deprep() {
    if let Some(f) = rl_deprep_term_function.get() {
        f();
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_tty_set_default_bindings(map: Keymap) {
    if map.is_null() || crate::vars::bind_tty_special_chars.get() == 0 {
        return;
    }
    let mut t: libc::termios = core::mem::zeroed();
    if libc::tcgetattr(tty_fd(), &mut t) == 0 {
        read_specials(&t);
    }
    let s = SPECIALS.get();
    let vi = map == crate::keymap::vi_insertion() || map == crate::keymap::vi_movement();
    let bind = |k: u8, f: rl_command_func_t| {
        if k != vdisable() && k != 0 {
            let e = map.add(k as usize);
            if (*e).type_ == ISFUNC {
                *e = KEYMAP_ENTRY::func(f);
            }
        }
    };
    bind(s.erase, crate::commands::text::rl_rubout);
    bind(s.kill, crate::kill::rl_unix_line_discard);
    if vi {
        bind(s.werase, crate::vi::rl_vi_unix_word_rubout);
    } else {
        bind(s.werase, crate::kill::rl_unix_word_rubout);
    }
    bind(s.lnext, crate::commands::text::rl_quoted_insert);
}

#[no_mangle]
pub unsafe extern "C" fn rl_tty_unset_default_bindings(map: Keymap) {
    // only after the terminal settings were saved
    if map.is_null() || !is_state(RL_STATE_TTYCSAVED) {
        return;
    }
    let s = SPECIALS.get();
    for k in [s.erase, s.kill, s.werase, s.lnext] {
        if k != vdisable() && k != 0 {
            let e = map.add(k as usize);
            if (*e).type_ == ISFUNC {
                *e = KEYMAP_ENTRY::func(crate::commands::text::rl_insert);
            }
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_tty_set_echoing(v: c_int) -> c_int {
    ECHOING.replace(v)
}

/// Temporarily turn the terminal's signal characters off (`false`) or back
/// on (`true`), e.g. while quoted-insert reads its character.  Only acts
/// while the terminal is prepped.  Returns whether they were on before.
pub unsafe fn set_tty_signals(on: bool) -> bool {
    if !is_state(RL_STATE_TERMPREPPED) {
        return false;
    }
    let fd = tty_fd();
    let mut t: libc::termios = core::mem::zeroed();
    if libc::tcgetattr(fd, &mut t) != 0 {
        return false;
    }
    let was = t.c_lflag & libc::ISIG != 0;
    if on {
        t.c_lflag |= libc::ISIG;
    } else {
        t.c_lflag &= !libc::ISIG;
    }
    libc::tcsetattr(fd, libc::TCSADRAIN, &t);
    was
}
