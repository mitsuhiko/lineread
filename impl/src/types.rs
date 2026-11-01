//! `#[repr(C)]` mirrors of the public types in include/readline/*.h.
#![allow(non_camel_case_types, non_snake_case)]

use core::ffi::{c_char, c_int, c_uint, c_void};
pub use libc::{time_t, FILE};

// ---- function pointer types (rltypedefs.h).  A C `T *` maps to Option<T>.
pub type rl_command_func_t = unsafe extern "C" fn(c_int, c_int) -> c_int;
pub type rl_compentry_func_t = unsafe extern "C" fn(*const c_char, c_int) -> *mut c_char;
pub type rl_completion_func_t =
    unsafe extern "C" fn(*const c_char, c_int, c_int) -> *mut *mut c_char;
pub type rl_quote_func_t = unsafe extern "C" fn(*mut c_char, c_int, *mut c_char) -> *mut c_char;
pub type rl_dequote_func_t = unsafe extern "C" fn(*mut c_char, c_int) -> *mut c_char;
pub type rl_compignore_func_t = unsafe extern "C" fn(*mut *mut c_char) -> c_int;
pub type rl_compdisp_func_t = unsafe extern "C" fn(*mut *mut c_char, c_int, c_int);
pub type rl_macro_print_func_t =
    unsafe extern "C" fn(*const c_char, *const c_char, c_int, *const c_char);
pub type rl_hook_func_t = unsafe extern "C" fn() -> c_int;
pub type rl_getc_func_t = unsafe extern "C" fn(*mut FILE) -> c_int;
pub type rl_linebuf_func_t = unsafe extern "C" fn(*mut c_char, c_int) -> c_int;
pub type rl_intfunc_t = unsafe extern "C" fn(c_int) -> c_int;
pub type rl_icpfunc_t = unsafe extern "C" fn(*mut c_char) -> c_int;
pub type rl_icppfunc_t = unsafe extern "C" fn(*mut *mut c_char) -> c_int;
pub type rl_voidfunc_t = unsafe extern "C" fn();
pub type rl_vintfunc_t = unsafe extern "C" fn(c_int);
pub type rl_vcpfunc_t = unsafe extern "C" fn(*mut c_char);
pub type rl_vcppfunc_t = unsafe extern "C" fn(*mut *mut c_char);
pub type rl_cpvfunc_t = unsafe extern "C" fn() -> *mut c_char;
pub type rl_cpifunc_t = unsafe extern "C" fn(c_int) -> *mut c_char;
pub type rl_cpcpfunc_t = unsafe extern "C" fn(*mut c_char) -> *mut c_char;
pub type rl_cpcppfunc_t = unsafe extern "C" fn(*mut *mut c_char) -> *mut c_char;
pub type tilde_hook_func_t = unsafe extern "C" fn(*mut c_char) -> *mut c_char;

/// A bindable command (non-null).
pub type Command = rl_command_func_t;

// ---- keymaps.h
pub const KEYMAP_SIZE: usize = 257;
pub const ANYOTHERKEY: usize = KEYMAP_SIZE - 1;
pub const ISFUNC: c_char = 0;
pub const ISKMAP: c_char = 1;
pub const ISMACR: c_char = 2;

/// The `function` slot of a keymap entry: a command, a keymap or a macro
/// string, depending on `type_`.  Same layout as a C pointer.
#[repr(C)]
#[derive(Clone, Copy)]
pub union KmData {
    pub func: Option<rl_command_func_t>,
    pub map: Keymap,
    pub macro_: *mut c_char,
    pub raw: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KEYMAP_ENTRY {
    pub type_: c_char,
    pub function: KmData,
}

impl KEYMAP_ENTRY {
    pub const EMPTY: KEYMAP_ENTRY = KEYMAP_ENTRY { type_: ISFUNC, function: KmData { func: None } };
    pub const fn func(f: rl_command_func_t) -> Self {
        KEYMAP_ENTRY { type_: ISFUNC, function: KmData { func: Some(f) } }
    }
    pub const fn keymap(m: Keymap) -> Self {
        KEYMAP_ENTRY { type_: ISKMAP, function: KmData { map: m } }
    }
    #[inline]
    pub fn is_null(&self) -> bool {
        unsafe { self.function.raw.is_null() }
    }
}

pub type KEYMAP_ENTRY_ARRAY = [KEYMAP_ENTRY; KEYMAP_SIZE];
pub type Keymap = *mut KEYMAP_ENTRY;

// ---- readline.h
pub type undo_code = c_uint;
pub const UNDO_DELETE: undo_code = 0;
pub const UNDO_INSERT: undo_code = 1;
pub const UNDO_BEGIN: undo_code = 2;
pub const UNDO_END: undo_code = 3;

#[repr(C)]
pub struct UNDO_LIST {
    pub next: *mut UNDO_LIST,
    pub start: c_int,
    pub end: c_int,
    pub text: *mut c_char,
    pub what: undo_code,
}

#[repr(C)]
pub struct FUNMAP {
    pub name: *const c_char,
    pub function: Option<rl_command_func_t>,
}

#[repr(C)]
pub struct readline_state {
    pub point: c_int,
    pub end: c_int,
    pub mark: c_int,
    pub buflen: c_int,
    pub buffer: *mut c_char,
    pub ul: *mut UNDO_LIST,
    pub prompt: *mut c_char,
    pub rlstate: c_int,
    pub done: c_int,
    pub kmap: Keymap,
    pub lastfunc: Option<rl_command_func_t>,
    pub insmode: c_int,
    pub edmode: c_int,
    pub kseq: *mut c_char,
    pub kseqlen: c_int,
    pub pendingin: c_int,
    pub inf: *mut FILE,
    pub outf: *mut FILE,
    pub macro_: *mut c_char,
    pub catchsigs: c_int,
    pub catchsigwinch: c_int,
    pub entryfunc: Option<rl_compentry_func_t>,
    pub menuentryfunc: Option<rl_compentry_func_t>,
    pub ignorefunc: Option<rl_compignore_func_t>,
    pub attemptfunc: Option<rl_completion_func_t>,
    pub wordbreakchars: *const c_char,
    pub reserved: [c_char; 64],
}

// ---- history.h
pub type histdata_t = *mut c_void;

#[repr(C)]
pub struct HIST_ENTRY {
    pub line: *mut c_char,
    pub timestamp: *mut c_char,
    pub data: histdata_t,
}

#[repr(C)]
pub struct HISTORY_STATE {
    pub entries: *mut *mut HIST_ENTRY,
    pub offset: c_int,
    pub length: c_int,
    pub size: c_int,
    pub flags: c_int,
}
pub const HS_STIFLED: c_int = 0x01;

// ---- rl_readline_state bits
pub const RL_STATE_NONE: u64 = 0;
pub const RL_STATE_INITIALIZING: u64 = 0x00000001;
pub const RL_STATE_INITIALIZED: u64 = 0x00000002;
pub const RL_STATE_TERMPREPPED: u64 = 0x00000004;
pub const RL_STATE_READCMD: u64 = 0x00000008;
pub const RL_STATE_METANEXT: u64 = 0x00000010;
pub const RL_STATE_DISPATCHING: u64 = 0x00000020;
pub const RL_STATE_MOREINPUT: u64 = 0x00000040;
pub const RL_STATE_ISEARCH: u64 = 0x00000080;
pub const RL_STATE_NSEARCH: u64 = 0x00000100;
pub const RL_STATE_SEARCH: u64 = 0x00000200;
pub const RL_STATE_NUMERICARG: u64 = 0x00000400;
pub const RL_STATE_MACROINPUT: u64 = 0x00000800;
pub const RL_STATE_MACRODEF: u64 = 0x00001000;
pub const RL_STATE_OVERWRITE: u64 = 0x00002000;
pub const RL_STATE_COMPLETING: u64 = 0x00004000;
pub const RL_STATE_SIGHANDLER: u64 = 0x00008000;
pub const RL_STATE_UNDOING: u64 = 0x00010000;
pub const RL_STATE_INPUTPENDING: u64 = 0x00020000;
pub const RL_STATE_TTYCSAVED: u64 = 0x00040000;
pub const RL_STATE_CALLBACK: u64 = 0x00080000;
pub const RL_STATE_VIMOTION: u64 = 0x00100000;
pub const RL_STATE_MULTIKEY: u64 = 0x00200000;
pub const RL_STATE_VICMDONCE: u64 = 0x00400000;
pub const RL_STATE_CHARSEARCH: u64 = 0x00800000;
pub const RL_STATE_REDISPLAYING: u64 = 0x01000000;
pub const RL_STATE_DONE: u64 = 0x02000000;
pub const RL_STATE_TIMEOUT: u64 = 0x04000000;
pub const RL_STATE_EOF: u64 = 0x08000000;
pub const RL_STATE_READSTR: u64 = 0x10000000;

pub const READERR: c_int = -2;
pub const RL_PROMPT_START_IGNORE: u8 = 1;
pub const RL_PROMPT_END_IGNORE: u8 = 2;
pub const NO_MATCH: c_int = 0;
pub const SINGLE_MATCH: c_int = 1;
pub const MULT_MATCH: c_int = 2;

pub const EOF: c_int = -1;
