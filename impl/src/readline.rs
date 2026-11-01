//! Entry points: `readline()`, `rl_initialize()`, the per-line setup, the
//! key loop, accept-line, editing modes, readline state save/restore.
//! Owner: core (architect); changes here affect everyone.
use crate::buffer;
use crate::display;
use crate::input::{is_state, set_state, unset_state};
use crate::prelude::*;

#[no_mangle]
pub static rl_library_version: CVar<*const c_char> = CVar::new(cs!("8.3"));
#[no_mangle]
pub static rl_readline_version: CVar<c_int> = CVar::new(0x0803);
#[no_mangle]
pub static rl_gnu_readline_p: CVar<c_int> = CVar::new(1);
#[no_mangle]
pub static rl_readline_state: CVar<c_ulong> = CVar::new(0);
/// 1 = emacs, 0 = vi
#[no_mangle]
pub static rl_editing_mode: CVar<c_int> = CVar::new(1);
#[no_mangle]
pub static rl_readline_name: CVar<*const c_char> = CVar::new(cs!("other"));
#[no_mangle]
pub static rl_done: CVar<c_int> = CVar::new(0);
#[no_mangle]
pub static rl_eof_found: CVar<c_int> = CVar::new(0);
#[no_mangle]
pub static rl_instream: CVar<*mut FILE> = CVar::new(null_mut());
#[no_mangle]
pub static rl_outstream: CVar<*mut FILE> = CVar::new(null_mut());
#[no_mangle]
pub static rl_startup_hook: CVar<Option<rl_hook_func_t>> = CVar::new(None);
#[no_mangle]
pub static rl_pre_input_hook: CVar<Option<rl_hook_func_t>> = CVar::new(None);
#[no_mangle]
pub static rl_num_chars_to_read: CVar<c_int> = CVar::new(0);

pub const EMACS_MODE: c_int = 1;
pub const VI_MODE: c_int = 0;

/// Inside readline() / the callback interface (a line is being edited).
static IN_READLINE: CVar<bool> = CVar::new(false);
/// The current line ended with EOF / read error (readline returns NULL).
static LINE_EOF: CVar<bool> = CVar::new(false);

pub fn initialized() -> bool {
    is_state(RL_STATE_INITIALIZED)
}

pub fn in_readline() -> bool {
    IN_READLINE.get()
}

/// The output stream (stdout when unset).
pub fn outstream() -> *mut FILE {
    let o = rl_outstream.get();
    if o.is_null() {
        unsafe { crate::util::lr_stdout() }
    } else {
        o
    }
}

pub fn set_editing_mode_emacs() {
    rl_editing_mode.set(EMACS_MODE);
    unsafe { crate::keymap::rl_set_keymap(crate::keymap::emacs_std()) };
}

pub fn set_editing_mode_vi() {
    rl_editing_mode.set(VI_MODE);
    unsafe { crate::keymap::rl_set_keymap(crate::keymap::vi_insertion()) };
}

/// One-time initialisation (`readline_initialize_everything`).
unsafe fn initialize_everything() {
    if rl_instream.get().is_null() {
        rl_instream.set(crate::util::lr_stdin());
    }
    if rl_outstream.get().is_null() {
        rl_outstream.set(crate::util::lr_stdout());
    }
    buffer::ensure(0);
    crate::terminal::init_terminal(cstr_opt(crate::terminal::rl_terminal_name.get()));
    crate::funmap::rl_initialize_funmap();
    crate::vars::apply_locale_defaults();
    if crate::vars::bind_tty_special_chars.get() != 0 {
        crate::tty::rl_tty_set_default_bindings(crate::keymap::current());
    }
    crate::inputrc::read_startup_init_file();
    // the edit mode decides the keymap, whatever `set keymap` said
    crate::keymap::rl_set_keymap_from_edit_mode();
    crate::keymap::bind_default_keyseqs();
    crate::complete::colors::init();
    crate::terminal::init_file_read();
    if crate::complete::rl_completer_word_break_characters.get().is_null() {
        crate::complete::rl_completer_word_break_characters.set(crate::complete::rl_basic_word_break_characters.get());
    }
    if crate::vars::active_region_start_color.get_mut().is_none() {
        crate::vars::active_region_start_color.set(Some(b"\x1b[7m".to_vec()));
    }
    if crate::vars::active_region_end_color.get_mut().is_none() {
        crate::vars::active_region_end_color.set(Some(b"\x1b[27m".to_vec()));
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_initialize() -> c_int {
    if !initialized() {
        set_state(RL_STATE_INITIALIZING);
        initialize_everything();
        unset_state(RL_STATE_INITIALIZING);
        set_state(RL_STATE_INITIALIZED);
    }
    // per-line state
    buffer::ensure(0);
    buffer::rl_point.set(0);
    buffer::rl_end.set(0);
    buffer::rl_mark.set(0);
    *buffer::rl_line_buffer.get() = 0;
    rl_done.set(0);
    unset_state(RL_STATE_DONE | RL_STATE_EOF | RL_STATE_TIMEOUT);
    crate::numarg::reset();
    // (rl_executing_keyseq keeps the last sequence until a key is read)
    crate::commands::text::reset_per_line();
    crate::history::using_history();
    if rl_display_prompt_is_null() {
        display::rl_display_prompt.set(cs_mut!(""));
    }
    0
}

fn rl_display_prompt_is_null() -> bool {
    display::rl_display_prompt.get().is_null()
}

/// Per-line setup: hooks, initial display.
pub unsafe fn line_setup() {
    LAST_KEY.set(0);
    rl_done.set(0);
    rl_eof_found.set(0);
    IN_READLINE.set(true);
    crate::terminal::line_setup();
    crate::dispatch::rl_last_func.set(None);
    crate::histcmds::line_started();
    if let Some(h) = rl_startup_hook.get() {
        h();
    }
    if rl_editing_mode.get() == VI_MODE {
        crate::vi::start_line_in_vi();
    }
    if rl_already_prompted() {
        display::rl_on_new_line_with_prompt();
    } else {
        display::print_prompt_prefix();
        display::rl_on_new_line();
        display::redisplay();
    }
    if let Some(h) = rl_pre_input_hook.get() {
        h();
    }
}

fn rl_already_prompted() -> bool {
    display::rl_already_prompted.get() != 0
}

/// Read and dispatch one key.  Returns false when the line is finished.
pub unsafe fn line_step() -> bool {
    // a new command: the numeric argument of the previous one is gone
    // (it stays visible between commands, like the reference)
    crate::numarg::reset();
    crate::dispatch::keyseq_reset();
    set_state(RL_STATE_READCMD);
    let c = crate::input::rl_read_key();
    if is_state(RL_STATE_TIMEOUT) && (c == EOF || c == READERR) {
        // rl_set_timeout expired: the line is abandoned as it is
        handle_timeout();
        return false;
    }
    unset_state(RL_STATE_READCMD);
    let mut c = c;
    if c == EOF && buffer::rl_end.get() > 0 && !is_state(RL_STATE_TERMPREPPED) {
        // end of a file without a final newline: the partial line is
        // accepted
        c = b'\n' as c_int;
    }
    if c == EOF || c == READERR {
        handle_eof(c);
        return false;
    }
    let last = LAST_KEY.replace(c);
    // the terminal's EOF character on an empty line ends the input before
    // any key binding is consulted
    if c == crate::tty::SPECIALS.get().eof as c_int && last != c && buffer::rl_end.get() == 0 {
        handle_eof(EOF);
        return false;
    }
    if crate::commands::misc::catch_abort(&mut || {
        crate::dispatch::dispatch_key(c, crate::keymap::current());
    }) {
        // the command was aborted (rl_abort and friends)
        crate::dispatch::rl_last_func.set(None);
        crate::dispatch::reset_command_stack();
    }
    unset_state(RL_STATE_MULTIKEY);
    after_command();
    crate::signals::rl_check_signals();
    if rl_done.get() == 0 {
        let n = rl_num_chars_to_read.get();
        if n > 0 && crate::mb::count_chars(buffer::line()) >= n as usize {
            rl_done.set(1);
            set_state(RL_STATE_DONE);
            display::redisplay();
            display::update_final();
            return false;
        }
        display::redisplay();
        return true;
    }
    false
}

/// End the line as if EOF was read (delete-char on an empty line).
pub unsafe fn signal_eof() {
    handle_eof(EOF);
}

/// Bookkeeping after every command dispatched by the key loop.
pub fn after_command() {
    crate::commands::text::after_command();
    crate::kill::after_command();
    crate::vi::after_command();
}

/// The previous key read by the key loop in this line.
static LAST_KEY: CVar<c_int> = CVar::new(0);

/// rl_set_timeout expired while reading a key: readline returns NULL,
/// the line buffer and the screen stay as they are.
unsafe fn handle_timeout() {
    crate::dispatch::rl_last_func.set(None);
    LINE_EOF.set(true);
    rl_eof_found.set(1);
    rl_done.set(1);
    set_state(RL_STATE_DONE | RL_STATE_EOF);
}

/// EOF (or a read error) while reading a key.
unsafe fn handle_eof(c: c_int) {
    LINE_EOF.set(true);
    rl_eof_found.set((c == EOF) as c_int);
    buffer::rl_end.set(0);
    buffer::ensure(0);
    *buffer::rl_line_buffer.get() = 0;
    rl_done.set(1);
    set_state(RL_STATE_DONE | RL_STATE_EOF);
    // (the cursor stays; restoring the terminal moves to the next line)
    crate::terminal::flush();
}

/// Finish the line: the result (malloc'd) or NULL at EOF.
pub unsafe fn line_teardown() -> *mut c_char {
    IN_READLINE.set(false);
    crate::commands::text::rl_deactivate_mark();
    crate::histcmds::line_finished();
    let r = if LINE_EOF.get() { null_mut() } else { c_strdup(buffer::line()) };
    LINE_EOF.set(false);
    crate::undo::rl_free_undo_list();
    r
}

/// The blocking editing loop for one line.
pub unsafe fn readline_internal() -> *mut c_char {
    line_setup();
    while line_step() {}
    line_teardown()
}

#[no_mangle]
pub unsafe extern "C" fn readline(prompt: *const c_char) -> *mut c_char {
    // an EOF pushed as pending input (rl_stuff_char(EOF), rl_execute_next)
    // ends the call before anything happens
    if crate::input::rl_pending_input.get() == EOF {
        crate::input::rl_clear_pending_input();
        return null_mut();
    }
    display::rl_set_prompt(prompt);
    rl_initialize();
    crate::input::start_timeout();
    crate::tty::prep();
    crate::signals::rl_set_signals();
    let r = readline_internal();
    // a signal that ended the read is processed before the terminal is
    // restored
    crate::signals::rl_check_signals();
    crate::tty::deprep();
    crate::signals::rl_clear_signals();
    r
}

#[no_mangle]
pub unsafe extern "C" fn rl_newline(_count: c_int, _key: c_int) -> c_int {
    rl_done.set(1);
    set_state(RL_STATE_DONE);
    if rl_editing_mode.get() == VI_MODE {
        crate::vi::newline_hook();
    }
    // (no redisplay: the line is already up to date)
    if display::rl_erase_empty_line.get() != 0 && buffer::rl_point.get() == 0 && buffer::rl_end.get() == 0 {
        // an empty line is not finished with a newline: the next prompt
        // replaces it
        crate::terminal::flush();
        return 0;
    }
    display::update_final();
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_emacs_editing_mode(_count: c_int, _key: c_int) -> c_int {
    set_editing_mode_emacs();
    unset_state(RL_STATE_VICMDONCE);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_free(p: *mut c_void) {
    libc::free(p)
}

#[no_mangle]
pub unsafe extern "C" fn rl_save_state(sp: *mut readline_state) -> c_int {
    if sp.is_null() {
        return -1;
    }
    let s = &mut *sp;
    s.point = buffer::rl_point.get();
    s.end = buffer::rl_end.get();
    s.mark = buffer::rl_mark.get();
    s.buffer = buffer::rl_line_buffer.get();
    s.buflen = buffer::BUFFER_LEN.get() as c_int;
    s.ul = crate::undo::rl_undo_list.get();
    s.prompt = display::rl_prompt.get();
    s.rlstate = rl_readline_state.get() as c_int;
    s.done = rl_done.get();
    s.kmap = crate::keymap::current();
    s.lastfunc = crate::dispatch::rl_last_func.get();
    s.insmode = crate::commands::text::rl_insert_mode.get();
    s.edmode = rl_editing_mode.get();
    s.kseq = crate::dispatch::rl_executing_keyseq.get();
    s.kseqlen = crate::dispatch::rl_key_sequence_length.get();
    s.inf = rl_instream.get();
    s.outf = rl_outstream.get();
    s.pendingin = crate::input::rl_pending_input.get();
    s.macro_ = crate::kbdmacro::rl_executing_macro.get();
    s.catchsigs = crate::signals::rl_catch_signals.get();
    s.catchsigwinch = crate::signals::rl_catch_sigwinch.get();
    s.entryfunc = crate::complete::rl_completion_entry_function.get();
    s.menuentryfunc = crate::complete::rl_menu_completion_entry_function.get();
    s.ignorefunc = crate::complete::rl_ignore_some_completions_function.get();
    s.attemptfunc = crate::complete::rl_attempted_completion_function.get();
    s.wordbreakchars = crate::complete::rl_completer_word_break_characters.get();
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_restore_state(sp: *mut readline_state) -> c_int {
    if sp.is_null() {
        return -1;
    }
    let s = &*sp;
    buffer::rl_point.set(s.point);
    buffer::rl_end.set(s.end);
    buffer::rl_mark.set(s.mark);
    buffer::rl_line_buffer.set(s.buffer);
    buffer::BUFFER_LEN.set(s.buflen.max(0) as usize);
    crate::undo::rl_undo_list.set(s.ul);
    display::rl_prompt.set(s.prompt);
    rl_readline_state.set(s.rlstate as c_uint as c_ulong);
    rl_done.set(s.done);
    crate::keymap::CURRENT.set(s.kmap);
    crate::dispatch::rl_last_func.set(s.lastfunc);
    crate::commands::text::rl_insert_mode.set(s.insmode);
    rl_editing_mode.set(s.edmode);
    crate::dispatch::rl_executing_keyseq.set(s.kseq);
    crate::dispatch::rl_key_sequence_length.set(s.kseqlen);
    rl_instream.set(s.inf);
    rl_outstream.set(s.outf);
    crate::input::rl_pending_input.set(s.pendingin);
    crate::kbdmacro::rl_executing_macro.set(s.macro_);
    crate::signals::rl_catch_signals.set(s.catchsigs);
    crate::signals::rl_catch_sigwinch.set(s.catchsigwinch);
    crate::complete::rl_completion_entry_function.set(s.entryfunc);
    crate::complete::rl_menu_completion_entry_function.set(s.menuentryfunc);
    crate::complete::rl_ignore_some_completions_function.set(s.ignorefunc);
    crate::complete::rl_attempted_completion_function.set(s.attemptfunc);
    crate::complete::rl_completer_word_break_characters.set(s.wordbreakchars);
    display::rl_display_prompt.set(if s.prompt.is_null() { cs_mut!("") } else { s.prompt });
    0
}
