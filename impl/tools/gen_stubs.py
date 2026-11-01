#!/usr/bin/env python3
"""One-shot generator used to bootstrap impl/src: emits stub definitions for
every public function/variable of conformance/abi.<platform>.json into the
module that owns it (see docs/ARCHITECTURE.md).  Already applied; kept for
reference.  Run: python3 impl/tools/gen_stubs.py OUTDIR"""
import json, os, re, sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
abi = json.load(open(os.path.join(ROOT, "conformance/abi.darwin-arm64.json")))
surf = abi["surface"]

OWN = {
"history/mod.rs": """using_history history_get_history_state history_set_history_state add_history
 add_history_time remove_history remove_history_range alloc_history_entry copy_history_entry
 free_history_entry replace_history_entry clear_history stifle_history unstifle_history
 history_is_stifled history_list where_history current_history history_get history_get_time
 history_total_bytes history_set_pos previous_history next_history
 history_base history_length history_max_entries history_offset max_input_history""",
"history/search.rs": "history_search history_search_prefix history_search_pos",
"history/file.rs": """read_history read_history_range write_history append_history history_truncate_file
 history_lines_read_from_file history_lines_written_to_file history_write_timestamps
 history_multiline_entries history_file_version""",
"history/expand.rs": """history_expand history_arg_extract get_history_event history_tokenize
 history_expansion_char history_subst_char history_word_delimiters history_comment_char
 history_no_expand_chars history_search_delimiter_chars history_quotes_inhibit_expansion
 history_quoting_state history_inhibit_expansion_function""",
"tilde.rs": """tilde_expand tilde_expand_word tilde_find_word tilde_expansion_preexpansion_hook
 tilde_expansion_failure_hook tilde_additional_prefixes tilde_additional_suffixes""",
"keymap/mod.rs": """rl_make_bare_keymap rl_empty_keymap rl_copy_keymap rl_make_keymap rl_discard_keymap
 rl_free_keymap rl_get_keymap_by_name rl_get_keymap_name rl_set_keymap rl_get_keymap
 rl_set_keymap_name rl_set_keymap_from_edit_mode rl_get_keymap_name_from_edit_mode
 emacs_standard_keymap emacs_meta_keymap emacs_ctlx_keymap vi_insertion_keymap vi_movement_keymap""",
"funmap.rs": """funmap rl_add_funmap_entry rl_funmap_names rl_initialize_funmap rl_named_function
 rl_list_funmap_names""",
"bind.rs": """rl_add_defun rl_bind_key rl_bind_key_in_map rl_unbind_key rl_unbind_key_in_map
 rl_bind_key_if_unbound rl_bind_key_if_unbound_in_map rl_unbind_function_in_map
 rl_unbind_command_in_map rl_bind_keyseq rl_bind_keyseq_in_map rl_bind_keyseq_if_unbound
 rl_bind_keyseq_if_unbound_in_map rl_generic_bind rl_set_key rl_macro_bind rl_translate_keyseq
 rl_untranslate_keyseq rl_function_of_keyseq rl_function_of_keyseq_len rl_trim_arg_from_keyseq
 rl_invoking_keyseqs_in_map rl_invoking_keyseqs rl_print_keybinding rl_function_dumper
 rl_macro_dumper rl_dump_functions rl_dump_macros rl_binding_keymap rl_macro_display_hook""",
"inputrc.rs": "rl_read_init_file rl_parse_and_bind rl_re_read_init_file",
"vars.rs": "rl_variable_bind rl_variable_value rl_variable_dumper rl_dump_variables",
"input.rs": """rl_getc rl_read_key rl_stuff_char rl_execute_next rl_clear_pending_input
 rl_set_keyboard_input_timeout rl_set_timeout rl_timeout_remaining rl_getc_function
 rl_pending_input rl_event_hook rl_timeout_event_hook rl_input_available_hook""",
"kbdmacro.rs": """rl_start_kbd_macro rl_end_kbd_macro rl_call_last_kbd_macro rl_print_last_kbd_macro
 rl_push_macro_input rl_executing_macro""",
"dispatch.rs": """rl_executing_keymap rl_executing_key rl_executing_keyseq rl_key_sequence_length
 rl_dispatching rl_last_func rl_do_lowercase_version rl_arrow_keys rl_skip_csi_sequence""",
"numarg.rs": "rl_digit_argument rl_universal_argument rl_discard_argument rl_explicit_arg rl_numeric_arg",
"readline.rs": """readline rl_initialize rl_save_state rl_restore_state rl_newline rl_free
 rl_library_version rl_readline_version rl_gnu_readline_p rl_readline_state rl_editing_mode
 rl_readline_name rl_done rl_eof_found rl_instream rl_outstream rl_startup_hook rl_pre_input_hook
 rl_num_chars_to_read rl_emacs_editing_mode""",
"buffer.rs": """rl_line_buffer rl_point rl_end rl_mark rl_insert_text rl_delete_text
 rl_extend_line_buffer rl_copy_text rl_replace_line""",
"undo.rs": """rl_undo_list rl_add_undo rl_free_undo_list rl_do_undo rl_begin_undo_group
 rl_end_undo_group rl_modifying rl_undo_command rl_revert_line""",
"kill.rs": """rl_kill_text rl_kill_word rl_backward_kill_word rl_kill_line rl_backward_kill_line
 rl_kill_full_line rl_unix_word_rubout rl_unix_filename_rubout rl_unix_line_discard
 rl_copy_region_to_kill rl_kill_region rl_copy_forward_word rl_copy_backward_word rl_yank
 rl_yank_pop rl_yank_nth_arg rl_yank_last_arg""",
"commands/movement.rs": """rl_forward_byte rl_forward_char rl_forward rl_backward_byte rl_backward_char
 rl_backward rl_beg_of_line rl_end_of_line rl_forward_word rl_backward_word""",
"commands/text.rs": """rl_insert rl_quoted_insert rl_tab_insert rl_rubout rl_delete rl_rubout_or_delete
 rl_delete_horizontal_space rl_insert_comment rl_upcase_word rl_downcase_word rl_capitalize_word
 rl_transpose_words rl_transpose_chars rl_char_search rl_backward_char_search rl_set_mark
 rl_exchange_point_and_mark rl_overwrite_mode rl_insert_close rl_set_paren_blink_timeout
 rl_alphabetic rl_bracketed_paste_begin rl_tilde_expand rl_keep_mark_active rl_activate_mark
 rl_deactivate_mark rl_mark_active_p rl_insert_mode""",
"commands/misc.rs": """rl_abort rl_tty_status rl_execute_named_command rl_restart_output rl_stop_output""",
"histcmds.rs": """rl_beginning_of_history rl_end_of_history rl_get_next_history rl_get_previous_history
 rl_operate_and_get_next rl_fetch_history rl_maybe_save_line rl_maybe_unsave_line
 rl_maybe_replace_line rl_clear_history""",
"search.rs": """rl_reverse_search_history rl_forward_search_history rl_noninc_forward_search
 rl_noninc_reverse_search rl_noninc_forward_search_again rl_noninc_reverse_search_again
 rl_history_search_forward rl_history_search_backward rl_history_substr_search_forward
 rl_history_substr_search_backward""",
"complete/mod.rs": """rl_complete rl_possible_completions rl_insert_completions rl_old_menu_complete
 rl_menu_complete rl_backward_menu_complete rl_export_completions rl_delete_or_show_completions
 rl_complete_internal rl_display_match_list rl_completion_matches rl_username_completion_function
 rl_filename_completion_function rl_completion_mode rl_reparse_colors
 rl_completion_entry_function rl_menu_completion_entry_function rl_ignore_some_completions_function
 rl_attempted_completion_function rl_basic_word_break_characters rl_completer_word_break_characters
 rl_completion_word_break_hook rl_completer_quote_characters rl_basic_quote_characters
 rl_filename_quote_characters rl_special_prefixes rl_directory_completion_hook
 rl_directory_rewrite_hook rl_filename_stat_hook rl_filename_rewrite_hook rl_completion_rewrite_hook
 rl_completion_display_matches_hook rl_filename_completion_desired rl_filename_quoting_desired
 rl_full_quoting_desired rl_filename_quoting_function rl_filename_dequoting_function
 rl_char_is_quoted_p rl_attempted_completion_over rl_completion_type rl_completion_invoking_key
 rl_completion_query_items rl_completion_append_character rl_completion_suppress_append
 rl_completion_quote_character rl_completion_found_quote rl_completion_suppress_quote
 rl_sort_completion_matches rl_completion_mark_symlink_dirs rl_ignore_completion_duplicates
 rl_inhibit_completion""",
"vi.rs": "@rl_vi_ rl_vi_editing_mode",
"display.rs": """rl_redisplay rl_on_new_line rl_on_new_line_with_prompt rl_forced_update_display
 rl_clear_visible_line rl_clear_message rl_reset_line_state rl_show_char rl_character_len
 rl_redraw_prompt_last_line rl_save_prompt rl_restore_prompt rl_set_prompt rl_expand_prompt
 rl_refresh_line rl_clear_screen rl_clear_display rl_previous_screen_line rl_next_screen_line
 rl_prompt rl_display_prompt rl_redisplay_function rl_already_prompted rl_erase_empty_line rl_message""",
"terminal.rs": """rl_reset_terminal rl_resize_terminal rl_set_screen_size rl_get_screen_size
 rl_reset_screen_size rl_get_termcap rl_ding rl_crlf rl_terminal_name rl_prefer_env_winsize""",
"tty.rs": """rl_prep_terminal rl_deprep_terminal rl_tty_set_default_bindings rl_tty_unset_default_bindings
 rl_tty_set_echoing rl_prep_term_function rl_deprep_term_function""",
"signals.rs": """rl_set_signals rl_clear_signals rl_cleanup_after_signal rl_reset_after_signal
 rl_free_line_state rl_pending_signal rl_check_signals rl_echo_signal_char rl_catch_signals
 rl_catch_sigwinch rl_change_environment rl_persistent_signal_handlers rl_signal_event_hook""",
"callback.rs": "rl_callback_handler_install rl_callback_read_char rl_callback_handler_remove rl_callback_sigcleanup",
}

owner = {}
prefixes = []
for mod, names in OWN.items():
    for n in names.split():
        if n.startswith("@"):
            prefixes.append((n[1:], mod))
        else:
            assert n not in owner, n
            owner[n] = mod

def owner_of(n):
    if n in owner: return owner[n]
    for p, m in prefixes:
        if n.startswith(p): return m
    raise SystemExit("no owner for " + n)

TY = {
    "int": "c_int", "void": "()", "char *": "*mut c_char", "const char *": "*const c_char",
    "char **": "*mut *mut c_char", "const char **": "*mut *const c_char", "Keymap": "Keymap",
    "HIST_ENTRY *": "*mut HIST_ENTRY", "HIST_ENTRY **": "*mut *mut HIST_ENTRY",
    "HISTORY_STATE *": "*mut HISTORY_STATE", "histdata_t": "histdata_t", "FILE *": "*mut FILE",
    "size_t": "usize", "time_t": "time_t", "int *": "*mut c_int", "unsigned int": "c_uint",
    "unsigned int *": "*mut c_uint", "struct readline_state *": "*mut readline_state",
    "enum undo_code": "undo_code", "void *": "*mut c_void", "char": "c_char",
    "unsigned long": "c_ulong", "UNDO_LIST *": "*mut UNDO_LIST", "FUNMAP **": "*mut *mut FUNMAP",
    "KEYMAP_ENTRY_ARRAY": "KEYMAP_ENTRY_ARRAY",
}
def ty(t):
    t = t.strip()
    m = re.match(r"^(\w+) \*$", t)
    if m and (m.group(1).endswith("_t") and m.group(1) != "histdata_t"):
        return f"Option<{m.group(1)}>"
    return TY[t]

def neutral(rt):
    if rt == "()": return ""
    if rt in ("c_int", "c_uint", "usize", "time_t", "c_char", "c_ulong"): return "0"
    if rt.startswith("Option<"): return "None"
    return "core::ptr::null_mut()"

DEFAULTS = {
 "history_base": "1", "rl_numeric_arg": "1", "rl_editing_mode": "1", "rl_insert_mode": "1",
 "rl_catch_signals": "1", "rl_catch_sigwinch": "1", "rl_change_environment": "1",
 "rl_filename_quoting_desired": "1", "rl_completion_query_items": "100",
 "rl_completion_append_character": "b' ' as c_int", "rl_sort_completion_matches": "1",
 "rl_ignore_completion_duplicates": "1", "history_file_version": "1",
 "rl_readline_version": "0x0803", "rl_gnu_readline_p": "1",
 "history_expansion_char": "b'!' as c_char", "history_subst_char": "b'^' as c_char",
 "rl_library_version": 'cs!("8.3")', "rl_readline_name": 'cs!("other")',
 "history_word_delimiters": 'cs_mut!(" \\t\\n;&()|<>")',
 "history_no_expand_chars": 'cs_mut!(" \\t\\n\\r=")',
 "rl_basic_word_break_characters": 'cs!(" \\t\\n\\"\\\\\'`@$><=;|&{(")',
 "rl_basic_quote_characters": 'cs!("\\"\'")',
}

out = {}
for name, v in sorted(surf.items()):
    if v["kind"] not in ("FunctionDecl", "VarDecl"): continue
    if name == "tilde_find_word": continue
    mod = owner_of(name)
    if v["kind"] == "FunctionDecl":
        m = re.match(r"^(.*?)\s*\((.*)\)$", v["type"])
        rt, args = m.group(1), m.group(2)
        if name == "rl_message":
            continue  # variadic: c/shim.c
        argl = [] if args in ("void", "") else [a.strip() for a in args.split(",")]
        rr = ty(rt)
        params = ", ".join(f"_a{i}: {ty(a)}" for i, a in enumerate(argl))
        if name == "rl_vi_insertion_mode": pass
        ret = "" if rr == "()" else f" -> {rr}"
        body = f'    todo_stub!("{name}");\n' + (f"    {neutral(rr)}\n" if rr != "()" else "")
        out.setdefault(mod, []).append(
            f"#[no_mangle]\npub unsafe extern \"C\" fn {name}({params}){ret} {{\n{body}}}\n")
    else:
        rt = ty(v["type"])
        if rt == "KEYMAP_ENTRY_ARRAY":
            continue  # keymap/defaults.rs
        d = DEFAULTS.get(name, neutral(rt) or "0")
        out.setdefault(mod, []).append(f"#[no_mangle]\npub static {name}: CVar<{rt}> = CVar::new({d});\n")

outdir = sys.argv[1]
for mod, items in out.items():
    p = os.path.join(outdir, mod)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    with open(p, "w") as f:
        f.write("//! GENERATED STUBS — see docs/ARCHITECTURE.md\n\nuse crate::prelude::*;\n\n")
        f.write("\n".join(items))
    print(p, len(items))
