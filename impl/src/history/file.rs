//! GENERATED STUBS — see docs/ARCHITECTURE.md

use crate::prelude::*;

#[no_mangle]
pub unsafe extern "C" fn append_history(_a0: c_int, _a1: *const c_char) -> c_int {
    todo_stub!("append_history");
    0
}

#[no_mangle]
pub static history_file_version: CVar<c_int> = CVar::new(1);

#[no_mangle]
pub static history_lines_read_from_file: CVar<c_int> = CVar::new(0);

#[no_mangle]
pub static history_lines_written_to_file: CVar<c_int> = CVar::new(0);

#[no_mangle]
pub static history_multiline_entries: CVar<c_int> = CVar::new(0);

#[no_mangle]
pub unsafe extern "C" fn history_truncate_file(_a0: *const c_char, _a1: c_int) -> c_int {
    todo_stub!("history_truncate_file");
    0
}

#[no_mangle]
pub static history_write_timestamps: CVar<c_int> = CVar::new(0);

#[no_mangle]
pub unsafe extern "C" fn read_history(_a0: *const c_char) -> c_int {
    todo_stub!("read_history");
    0
}

#[no_mangle]
pub unsafe extern "C" fn read_history_range(_a0: *const c_char, _a1: c_int, _a2: c_int) -> c_int {
    todo_stub!("read_history_range");
    0
}

#[no_mangle]
pub unsafe extern "C" fn write_history(_a0: *const c_char) -> c_int {
    todo_stub!("write_history");
    0
}
