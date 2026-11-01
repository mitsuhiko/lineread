//! Readline (inputrc) variables: `set NAME VALUE`, `rl_variable_bind`,
//! `rl_variable_value`, `rl_variable_dumper`.
//!
//! Every variable has a crate-visible global here so other modules can read
//! the setting (`crate::vars::blink_matching_paren.get() != 0`).  Owner:
//! inputrc workstream.  Other modules only read these (or call
//! `rl_variable_bind`).
use crate::prelude::*;
use crate::util::{fflush, fwrite_bytes, ByteBufExt};
use core::ffi::CStr;

// ---- boolean variables (0/1)
pub static bind_tty_special_chars: CVar<c_int> = CVar::new(1);
pub static blink_matching_paren: CVar<c_int> = CVar::new(0);
pub static rl_byte_oriented: CVar<c_int> = CVar::new(0);
pub static colored_completion_prefix: CVar<c_int> = CVar::new(0);
pub static colored_stats: CVar<c_int> = CVar::new(0);
pub static completion_ignore_case: CVar<c_int> = CVar::new(0);
pub static completion_map_case: CVar<c_int> = CVar::new(0);
pub static convert_meta: CVar<c_int> = CVar::new(1);
pub static disable_completion: CVar<c_int> = CVar::new(0);
pub static echo_control_characters: CVar<c_int> = CVar::new(1);
pub static enable_active_region: CVar<c_int> = CVar::new(1);
pub static enable_bracketed_paste: CVar<c_int> = CVar::new(1);
pub static enable_keypad: CVar<c_int> = CVar::new(0);
pub static enable_meta_key: CVar<c_int> = CVar::new(1);
pub static expand_tilde: CVar<c_int> = CVar::new(0);
pub static force_meta_prefix: CVar<c_int> = CVar::new(0);
pub static history_preserve_point: CVar<c_int> = CVar::new(0);
pub static horizontal_scroll_mode: CVar<c_int> = CVar::new(0);
/// input-meta and its alias meta-flag
pub static input_meta: CVar<c_int> = CVar::new(0);
pub static mark_directories: CVar<c_int> = CVar::new(1);
pub static mark_modified_lines: CVar<c_int> = CVar::new(0);
pub static mark_symlinked_directories: CVar<c_int> = CVar::new(0);
pub static match_hidden_files: CVar<c_int> = CVar::new(1);
pub static menu_complete_display_prefix: CVar<c_int> = CVar::new(0);
pub static output_meta: CVar<c_int> = CVar::new(0);
pub static page_completions: CVar<c_int> = CVar::new(1);
pub static prefer_visible_bell: CVar<c_int> = CVar::new(1);
pub static print_completions_horizontally: CVar<c_int> = CVar::new(0);
pub static revert_all_at_newline: CVar<c_int> = CVar::new(0);
pub static search_ignore_case: CVar<c_int> = CVar::new(0);
pub static show_all_if_ambiguous: CVar<c_int> = CVar::new(0);
pub static show_all_if_unmodified: CVar<c_int> = CVar::new(0);
pub static show_mode_in_prompt: CVar<c_int> = CVar::new(0);
pub static skip_completed_text: CVar<c_int> = CVar::new(0);
pub static visible_stats: CVar<c_int> = CVar::new(0);

/// Set when the application/init file changed the variable (locale based
/// defaults of the meta variables are applied only to untouched ones).
pub static meta_vars_touched: CVar<c_int> = CVar::new(0);

// ---- valued variables
pub const BELL_NONE: c_int = 0;
pub const BELL_VISIBLE: c_int = 1;
pub const BELL_AUDIBLE: c_int = 2;
pub static bell_style: CVar<c_int> = CVar::new(BELL_AUDIBLE);
/// comment-begin (None = default "#")
pub static comment_begin: CVar<Option<Vec<u8>>> = CVar::new(None);
pub static completion_display_width: CVar<c_int> = CVar::new(-1);
pub static completion_prefix_display_length: CVar<c_int> = CVar::new(0);
/// emacs-mode-string etc.: None = default; stored translated (real bytes)
pub static emacs_mode_string: CVar<Option<Vec<u8>>> = CVar::new(None);
pub static vi_cmd_mode_string: CVar<Option<Vec<u8>>> = CVar::new(None);
pub static vi_ins_mode_string: CVar<Option<Vec<u8>>> = CVar::new(None);
/// isearch-terminators as given (untranslated); None = default
pub static isearch_terminators: CVar<Option<Vec<u8>>> = CVar::new(None);
pub static keyseq_timeout: CVar<c_int> = CVar::new(500);
/// active-region colors; None until rl_initialize sets the defaults
pub static active_region_start_color: CVar<Option<Vec<u8>>> = CVar::new(None);
pub static active_region_end_color: CVar<Option<Vec<u8>>> = CVar::new(None);

pub fn comment_begin_bytes() -> Vec<u8> {
    comment_begin.get_mut().clone().unwrap_or_else(|| b"#".to_vec())
}
pub fn emacs_mode_bytes() -> Vec<u8> {
    emacs_mode_string.get_mut().clone().unwrap_or_else(|| b"@".to_vec())
}
pub fn vi_cmd_mode_bytes() -> Vec<u8> {
    vi_cmd_mode_string.get_mut().clone().unwrap_or_else(|| b"(cmd)".to_vec())
}
pub fn vi_ins_mode_bytes() -> Vec<u8> {
    vi_ins_mode_string.get_mut().clone().unwrap_or_else(|| b"(ins)".to_vec())
}

struct BoolVar {
    name: &'static CStr,
    var: &'static CVar<c_int>,
}

macro_rules! b {
    ($n:literal, $v:ident) => {
        BoolVar { name: $n, var: &$v }
    };
}

/// Alphabetical, as the dumper lists them.
static BOOL_VARS: &[BoolVar] = &[
    b!(c"bind-tty-special-chars", bind_tty_special_chars),
    b!(c"blink-matching-paren", blink_matching_paren),
    b!(c"byte-oriented", rl_byte_oriented),
    b!(c"colored-completion-prefix", colored_completion_prefix),
    b!(c"colored-stats", colored_stats),
    b!(c"completion-ignore-case", completion_ignore_case),
    b!(c"completion-map-case", completion_map_case),
    b!(c"convert-meta", convert_meta),
    b!(c"disable-completion", disable_completion),
    b!(c"echo-control-characters", echo_control_characters),
    b!(c"enable-active-region", enable_active_region),
    b!(c"enable-bracketed-paste", enable_bracketed_paste),
    b!(c"enable-keypad", enable_keypad),
    b!(c"enable-meta-key", enable_meta_key),
    b!(c"expand-tilde", expand_tilde),
    b!(c"force-meta-prefix", force_meta_prefix),
    b!(c"history-preserve-point", history_preserve_point),
    b!(c"horizontal-scroll-mode", horizontal_scroll_mode),
    b!(c"input-meta", input_meta),
    b!(c"mark-directories", mark_directories),
    b!(c"mark-modified-lines", mark_modified_lines),
    b!(c"mark-symlinked-directories", mark_symlinked_directories),
    b!(c"match-hidden-files", match_hidden_files),
    b!(c"menu-complete-display-prefix", menu_complete_display_prefix),
    b!(c"meta-flag", input_meta),
    b!(c"output-meta", output_meta),
    b!(c"page-completions", page_completions),
    b!(c"prefer-visible-bell", prefer_visible_bell),
    b!(c"print-completions-horizontally", print_completions_horizontally),
    b!(c"revert-all-at-newline", revert_all_at_newline),
    b!(c"search-ignore-case", search_ignore_case),
    b!(c"show-all-if-ambiguous", show_all_if_ambiguous),
    b!(c"show-all-if-unmodified", show_all_if_unmodified),
    b!(c"show-mode-in-prompt", show_mode_in_prompt),
    b!(c"skip-completed-text", skip_completed_text),
    b!(c"visible-stats", visible_stats),
];

/// Alphabetical names of the valued variables.
static STRING_VARS: &[&CStr] = &[
    c"active-region-end-color",
    c"active-region-start-color",
    c"bell-style",
    c"comment-begin",
    c"completion-display-width",
    c"completion-prefix-display-length",
    c"completion-query-items",
    c"editing-mode",
    c"emacs-mode-string",
    c"history-size",
    c"isearch-terminators",
    c"keymap",
    c"keyseq-timeout",
    c"vi-cmd-mode-string",
    c"vi-ins-mode-string",
];

fn find_bool(name: &[u8]) -> Option<&'static BoolVar> {
    BOOL_VARS.iter().find(|v| v.name.to_bytes().eq_ignore_ascii_case(name))
}

fn find_string(name: &[u8]) -> Option<&'static CStr> {
    STRING_VARS.iter().copied().find(|v| v.to_bytes().eq_ignore_ascii_case(name))
}

/// Is `name` a known variable (case-insensitive)?
pub fn is_variable(name: &[u8]) -> bool {
    find_bool(name).is_some() || find_string(name).is_some()
}

/// Boolean value parsing: empty/NULL, "on" (any case) and "1" are on.
pub fn parse_bool(value: Option<&[u8]>) -> c_int {
    match value {
        None => 1,
        Some(v) if v.is_empty() => 1,
        Some(v) => (v.eq_ignore_ascii_case(b"on") || v == b"1") as c_int,
    }
}

/// Leading integer like atoi.
pub fn atoi(v: &[u8]) -> c_int {
    let mut i = 0;
    while i < v.len() && (v[i] == b' ' || v[i] == b'\t') {
        i += 1;
    }
    let mut neg = false;
    if i < v.len() && (v[i] == b'-' || v[i] == b'+') {
        neg = v[i] == b'-';
        i += 1;
    }
    let mut n: i64 = 0;
    while i < v.len() && v[i].is_ascii_digit() {
        n = (n * 10 + (v[i] - b'0') as i64).min(i32::MAX as i64 + 1);
        i += 1;
    }
    (if neg { -n } else { n }) as c_int
}

/// Side effects of changing a boolean variable.
fn bool_changed(name: &[u8]) {
    let n = name.to_ascii_lowercase();
    match &n[..] {
        b"input-meta" | b"meta-flag" | b"convert-meta" | b"output-meta" => {
            meta_vars_touched.set(1);
        }
        b"show-mode-in-prompt" => crate::display::mode_string_changed(),
        _ => {}
    }
}

/// Error reporting for variable assignments (goes through the init file
/// parser so messages carry file/line context when parsing a file).
fn report(msg: &[u8]) {
    crate::inputrc::report_error(msg);
}

/// Set a valued variable.  Returns 0 or nonzero on a bad value.
fn set_string_var(name: &CStr, value: &[u8]) -> c_int {
    use crate::keymap;
    match name.to_bytes() {
        b"bell-style" => {
            let v = value.to_ascii_lowercase();
            let s = match &v[..] {
                b"none" | b"off" => BELL_NONE,
                b"visible" => BELL_VISIBLE,
                b"audible" | b"on" => BELL_AUDIBLE,
                _ => return 1,
            };
            bell_style.set(s);
        }
        b"comment-begin" => comment_begin.set(Some(crate::bind::translate_keyseq_bytes(value).0)),
        b"completion-display-width" => completion_display_width.set(atoi(value)),
        b"completion-prefix-display-length" => {
            completion_prefix_display_length.set(atoi(value).max(0))
        }
        b"completion-query-items" => {
            crate::complete::rl_completion_query_items.set(if value.is_empty() {
                100
            } else {
                atoi(value).max(0)
            })
        }
        b"editing-mode" => {
            let v = value.to_ascii_lowercase();
            if v.starts_with(b"vi") {
                crate::readline::set_editing_mode_vi();
            } else if v.starts_with(b"emacs") {
                crate::readline::set_editing_mode_emacs();
            } else {
                return 1;
            }
        }
        b"emacs-mode-string" => {
            emacs_mode_string.set(Some(crate::bind::translate_keyseq_bytes(value).0));
            crate::display::mode_string_changed();
        }
        b"vi-cmd-mode-string" => {
            vi_cmd_mode_string.set(Some(crate::bind::translate_keyseq_bytes(value).0));
            crate::display::mode_string_changed();
        }
        b"vi-ins-mode-string" => {
            vi_ins_mode_string.set(Some(crate::bind::translate_keyseq_bytes(value).0));
            crate::display::mode_string_changed();
        }
        b"active-region-start-color" => {
            active_region_start_color.set(Some(crate::bind::translate_keyseq_bytes(value).0))
        }
        b"active-region-end-color" => {
            active_region_end_color.set(Some(crate::bind::translate_keyseq_bytes(value).0))
        }
        b"history-size" => {
            let n = if value.is_empty() { 0 } else { atoi(value) };
            unsafe {
                if n < 0 {
                    crate::history::unstifle_history();
                } else {
                    crate::history::stifle_history(n);
                }
            }
        }
        b"isearch-terminators" => isearch_terminators.set(Some(value.to_vec())),
        b"keymap" => {
            let km = keymap::keymap_by_name(value);
            if km.is_null() {
                return 1;
            }
            unsafe { keymap::rl_set_keymap(km) };
        }
        b"keyseq-timeout" => keyseq_timeout.set(atoi(value).max(0)),
        _ => return 1,
    }
    0
}

/// Current value of a valued variable, None if unset.
fn string_var_value(name: &CStr) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    match name.to_bytes() {
        b"bell-style" => {
            out.push_str(match bell_style.get() {
                BELL_NONE => "none",
                BELL_VISIBLE => "visible",
                _ => "audible",
            });
        }
        b"comment-begin" => out = comment_begin_bytes(),
        b"completion-display-width" => out.push_int(completion_display_width.get() as i64),
        b"completion-prefix-display-length" => {
            out.push_int(completion_prefix_display_length.get() as i64)
        }
        b"completion-query-items" => {
            out.push_int(crate::complete::rl_completion_query_items.get() as i64)
        }
        b"editing-mode" => {
            out.push_str(if crate::readline::rl_editing_mode.get() == 0 { "vi" } else { "emacs" })
        }
        b"emacs-mode-string" => out = emacs_mode_bytes(),
        b"vi-cmd-mode-string" => out = vi_cmd_mode_bytes(),
        b"vi-ins-mode-string" => out = vi_ins_mode_bytes(),
        b"active-region-start-color" => return active_region_start_color.get_mut().clone(),
        b"active-region-end-color" => return active_region_end_color.get_mut().clone(),
        b"history-size" => {
            let n = unsafe {
                if crate::history::history_is_stifled() != 0 {
                    crate::history::history_max_entries.get()
                } else {
                    -1
                }
            };
            out.push_int(n as i64)
        }
        b"isearch-terminators" => return isearch_terminators.get_mut().clone(),
        b"keymap" => {
            let name = crate::keymap::keymap_name(crate::keymap::current());
            return name.map(|n| n.to_vec());
        }
        b"keyseq-timeout" => out.push_int(keyseq_timeout.get() as i64),
        _ => return None,
    }
    Some(out)
}

/// `set NAME VALUE` semantics shared by `rl_variable_bind` and the init
/// file parser.  Returns 0 on success (also for unknown names, which are
/// reported), nonzero when the value was rejected.
pub fn variable_bind(name: &[u8], value: Option<&[u8]>) -> c_int {
    if let Some(v) = find_bool(name) {
        v.var.set(parse_bool(value));
        bool_changed(name);
        return 0;
    }
    if let Some(n) = find_string(name) {
        let val = value.unwrap_or(b"");
        let r = set_string_var(n, val);
        if r != 0 {
            let mut m = Vec::new();
            m.extend_from_slice(n.to_bytes());
            m.push_str(": could not set value to `");
            m.extend_from_slice(val);
            m.push(b'\'');
            report(&m);
        }
        return r;
    }
    let mut m = name.to_vec();
    m.push_str(": unknown variable name");
    report(&m);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rl_variable_bind(name: *const c_char, value: *const c_char) -> c_int {
    variable_bind(cstr(name), cstr_opt(value))
}

/// Value of a variable as bytes (`on`/`off` for booleans).
pub fn variable_value(name: &[u8]) -> Option<Vec<u8>> {
    if let Some(v) = find_bool(name) {
        return Some(if v.var.get() != 0 { b"on".to_vec() } else { b"off".to_vec() });
    }
    find_string(name).and_then(string_var_value)
}

#[no_mangle]
pub unsafe extern "C" fn rl_variable_value(name: *const c_char) -> *mut c_char {
    // the result points to storage owned by the library (not freed by the
    // caller); keep the last value alive
    static LAST: CVar<Vec<u8>> = CVar::new(Vec::new());
    match variable_value(cstr(name)) {
        Some(mut v) => {
            v.push(0);
            let l = LAST.get_mut();
            *l = v;
            l.as_mut_ptr() as *mut c_char
        }
        None => null_mut(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_variable_dumper(readable: c_int) {
    let out = crate::readline::outstream();
    let mut buf = Vec::new();
    for v in BOOL_VARS {
        let val = if v.var.get() != 0 { "on" } else { "off" };
        line(&mut buf, v.name.to_bytes(), val.as_bytes(), readable);
    }
    for n in STRING_VARS {
        if let Some(val) = string_var_value(n) {
            line(&mut buf, n.to_bytes(), &val, readable);
        }
    }
    fwrite_bytes(out, &buf);
    fflush(out);
}

fn line(buf: &mut Vec<u8>, name: &[u8], val: &[u8], readable: c_int) {
    if readable != 0 {
        buf.push_str("set ");
        buf.extend_from_slice(name);
        buf.push(b' ');
        buf.extend_from_slice(val);
        buf.push(b'\n');
    } else {
        buf.extend_from_slice(name);
        buf.push_str(" is set to `");
        buf.extend_from_slice(val);
        buf.push_str("'\n");
    }
}

#[no_mangle]
pub unsafe extern "C" fn rl_dump_variables(_count: c_int, _key: c_int) -> c_int {
    let readable = crate::numarg::rl_explicit_arg.get();
    crate::display::output_before_listing();
    rl_variable_dumper(readable);
    crate::display::after_listing();
    0
}

/// Apply the locale dependent defaults (8-bit input/output) at init time.
pub fn apply_locale_defaults() {
    if meta_vars_touched.get() != 0 {
        return;
    }
    if crate::mb::locale_is_8bit() {
        input_meta.set(1);
        output_meta.set(1);
        convert_meta.set(0);
    }
}
