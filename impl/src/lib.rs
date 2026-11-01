//! lineread: a clean-room implementation of the GNU Readline 8.3 C API/ABI
//! (readline, history and tilde).  See docs/ARCHITECTURE.md.
#![allow(non_upper_case_globals, non_camel_case_types, non_snake_case)]
#![allow(clippy::missing_safety_doc, clippy::not_unsafe_ptr_arg_deref)]
#![allow(unused_unsafe)]

#[macro_use]
pub mod util;
pub mod cvar;
pub mod prelude;
pub mod types;
pub mod mb;

// history library (independent of the editor)
pub mod history;
pub mod tilde;

// key bindings, functions, init file, variables
pub mod bind;
pub mod funmap;
pub mod funmap_table;
pub mod inputrc;
pub mod keymap;
pub mod vars;

// input and dispatch
pub mod dispatch;
pub mod input;
pub mod kbdmacro;
pub mod numarg;

// the line and editing commands
pub mod buffer;
pub mod commands;
pub mod histcmds;
pub mod kill;
pub mod search;
pub mod undo;
pub mod vi;
pub mod complete;

// terminal, display, signals, entry points
pub mod callback;
pub mod display;
pub mod readline;
pub mod signals;
pub mod terminal;
pub mod tty;
