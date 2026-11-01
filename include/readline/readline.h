/* lineread: public interface of a GNU Readline 8.3 compatible library.
   Line editing, key binding, completion and display. */
#ifndef _READLINE_H_
#define _READLINE_H_

#ifdef __cplusplus
extern "C" {
#endif

#ifdef READLINE_LIBRARY
#  include "rlstdc.h"
#  include "rltypedefs.h"
#  include "keymaps.h"
#  include "tilde.h"
#else
#  include <readline/rlstdc.h>
#  include <readline/rltypedefs.h>
#  include <readline/keymaps.h>
#  include <readline/tilde.h>
#endif

#define RL_READLINE_VERSION 0x0803
#define RL_VERSION_MAJOR 8
#define RL_VERSION_MINOR 3

/* ------------------------------------------------------------------ */
/* Undo list                                                           */

enum undo_code { UNDO_DELETE, UNDO_INSERT, UNDO_BEGIN, UNDO_END };

typedef struct undo_list {
  struct undo_list *next;
  int start, end;
  char *text;
  enum undo_code what;
} UNDO_LIST;

extern UNDO_LIST *rl_undo_list;

/* ------------------------------------------------------------------ */
/* Function name map                                                   */

typedef struct _funmap {
  const char *name;
  rl_command_func_t *function;
} FUNMAP;

extern FUNMAP **funmap;

/* ------------------------------------------------------------------ */
/* Bindable commands                                                   */

/* Numeric arguments. */
extern int rl_digit_argument (int, int);
extern int rl_universal_argument (int, int);

/* Movement. */
extern int rl_forward_byte (int, int);
extern int rl_forward_char (int, int);
extern int rl_forward (int, int);
extern int rl_backward_byte (int, int);
extern int rl_backward_char (int, int);
extern int rl_backward (int, int);
extern int rl_beg_of_line (int, int);
extern int rl_end_of_line (int, int);
extern int rl_forward_word (int, int);
extern int rl_backward_word (int, int);
extern int rl_refresh_line (int, int);
extern int rl_clear_screen (int, int);
extern int rl_clear_display (int, int);
extern int rl_skip_csi_sequence (int, int);
extern int rl_arrow_keys (int, int);
extern int rl_previous_screen_line (int, int);
extern int rl_next_screen_line (int, int);

/* Inserting and deleting text. */
extern int rl_insert (int, int);
extern int rl_quoted_insert (int, int);
extern int rl_tab_insert (int, int);
extern int rl_newline (int, int);
extern int rl_do_lowercase_version (int, int);
extern int rl_rubout (int, int);
extern int rl_delete (int, int);
extern int rl_rubout_or_delete (int, int);
extern int rl_delete_horizontal_space (int, int);
extern int rl_delete_or_show_completions (int, int);
extern int rl_insert_comment (int, int);

/* Changing case and transposing. */
extern int rl_upcase_word (int, int);
extern int rl_downcase_word (int, int);
extern int rl_capitalize_word (int, int);
extern int rl_transpose_words (int, int);
extern int rl_transpose_chars (int, int);

/* Character search. */
extern int rl_char_search (int, int);
extern int rl_backward_char_search (int, int);

/* History movement. */
extern int rl_beginning_of_history (int, int);
extern int rl_end_of_history (int, int);
extern int rl_get_next_history (int, int);
extern int rl_get_previous_history (int, int);
extern int rl_operate_and_get_next (int, int);
extern int rl_fetch_history (int, int);

/* Mark and region. */
extern int rl_set_mark (int, int);
extern int rl_exchange_point_and_mark (int, int);

/* Editing mode. */
extern int rl_vi_editing_mode (int, int);
extern int rl_emacs_editing_mode (int, int);

/* Insert/overwrite. */
extern int rl_overwrite_mode (int, int);

/* Init file and dumping. */
extern int rl_re_read_init_file (int, int);
extern int rl_dump_functions (int, int);
extern int rl_dump_macros (int, int);
extern int rl_dump_variables (int, int);

/* Completion. */
extern int rl_complete (int, int);
extern int rl_possible_completions (int, int);
extern int rl_insert_completions (int, int);
extern int rl_old_menu_complete (int, int);
extern int rl_menu_complete (int, int);
extern int rl_backward_menu_complete (int, int);
extern int rl_export_completions (int, int);

/* Killing and yanking. */
extern int rl_kill_word (int, int);
extern int rl_backward_kill_word (int, int);
extern int rl_kill_line (int, int);
extern int rl_backward_kill_line (int, int);
extern int rl_kill_full_line (int, int);
extern int rl_unix_word_rubout (int, int);
extern int rl_unix_filename_rubout (int, int);
extern int rl_unix_line_discard (int, int);
extern int rl_copy_region_to_kill (int, int);
extern int rl_kill_region (int, int);
extern int rl_copy_forward_word (int, int);
extern int rl_copy_backward_word (int, int);
extern int rl_yank (int, int);
extern int rl_yank_pop (int, int);
extern int rl_yank_nth_arg (int, int);
extern int rl_yank_last_arg (int, int);
extern int rl_bracketed_paste_begin (int, int);
#ifdef _WIN32
extern int rl_paste_from_clipboard (int, int);
#endif

/* Incremental search. */
extern int rl_reverse_search_history (int, int);
extern int rl_forward_search_history (int, int);

/* Keyboard macros. */
extern int rl_start_kbd_macro (int, int);
extern int rl_end_kbd_macro (int, int);
extern int rl_call_last_kbd_macro (int, int);
extern int rl_print_last_kbd_macro (int, int);

/* Undo. */
extern int rl_revert_line (int, int);
extern int rl_undo_command (int, int);

/* Miscellaneous. */
extern int rl_tilde_expand (int, int);
extern int rl_restart_output (int, int);
extern int rl_stop_output (int, int);
extern int rl_abort (int, int);
extern int rl_tty_status (int, int);
extern int rl_execute_named_command (int, int);

/* Non-incremental and prefix/substring history search. */
extern int rl_history_search_forward (int, int);
extern int rl_history_search_backward (int, int);
extern int rl_history_substr_search_forward (int, int);
extern int rl_history_substr_search_backward (int, int);
extern int rl_noninc_forward_search (int, int);
extern int rl_noninc_reverse_search (int, int);
extern int rl_noninc_forward_search_again (int, int);
extern int rl_noninc_reverse_search_again (int, int);

/* Parenthesis matching. */
extern int rl_insert_close (int, int);
/* Callback (alternate) interface. */
extern void rl_callback_handler_install (const char *, rl_vcpfunc_t *);
extern void rl_callback_read_char (void);
extern void rl_callback_handler_remove (void);
extern void rl_callback_sigcleanup (void);

/* vi mode. */
extern int rl_vi_redo (int, int);
extern int rl_vi_undo (int, int);
extern int rl_vi_yank_arg (int, int);
extern int rl_vi_fetch_history (int, int);
extern int rl_vi_search_again (int, int);
extern int rl_vi_search (int, int);
extern int rl_vi_complete (int, int);
extern int rl_vi_tilde_expand (int, int);
extern int rl_vi_prev_word (int, int);
extern int rl_vi_next_word (int, int);
extern int rl_vi_end_word (int, int);
extern int rl_vi_insert_beg (int, int);
extern int rl_vi_append_mode (int, int);
extern int rl_vi_append_eol (int, int);
extern int rl_vi_eof_maybe (int, int);
extern int rl_vi_insertion_mode (int, int);
extern int rl_vi_insert_mode (int, int);
extern int rl_vi_movement_mode (int, int);
extern int rl_vi_arg_digit (int, int);
extern int rl_vi_change_case (int, int);
extern int rl_vi_put (int, int);
extern int rl_vi_column (int, int);
extern int rl_vi_delete_to (int, int);
extern int rl_vi_change_to (int, int);
extern int rl_vi_yank_to (int, int);
extern int rl_vi_yank_pop (int, int);
extern int rl_vi_rubout (int, int);
extern int rl_vi_delete (int, int);
extern int rl_vi_back_to_indent (int, int);
extern int rl_vi_unix_word_rubout (int, int);
extern int rl_vi_first_print (int, int);
extern int rl_vi_char_search (int, int);
extern int rl_vi_match (int, int);
extern int rl_vi_change_char (int, int);
extern int rl_vi_subst (int, int);
extern int rl_vi_overstrike (int, int);
extern int rl_vi_overstrike_delete (int, int);
extern int rl_vi_replace (int, int);
extern int rl_vi_set_mark (int, int);
extern int rl_vi_goto_mark (int, int);
extern int rl_vi_check (void);
extern int rl_vi_domove (int, int *);
extern int rl_vi_bracktype (int);
extern void rl_vi_start_inserting (int, int, int);
extern int rl_vi_fWord (int, int);
extern int rl_vi_bWord (int, int);
extern int rl_vi_eWord (int, int);
extern int rl_vi_fword (int, int);
extern int rl_vi_bword (int, int);
extern int rl_vi_eword (int, int);
/* ------------------------------------------------------------------ */
/* Application entry points                                            */

extern char *readline (const char *);

extern int rl_set_prompt (const char *);
extern int rl_expand_prompt (char *);

extern int rl_initialize (void);

extern int rl_discard_argument (void);

/* Binding keys and functions. */
extern int rl_add_defun (const char *, rl_command_func_t *, int);
extern int rl_bind_key (int, rl_command_func_t *);
extern int rl_bind_key_in_map (int, rl_command_func_t *, Keymap);
extern int rl_unbind_key (int);
extern int rl_unbind_key_in_map (int, Keymap);
extern int rl_bind_key_if_unbound (int, rl_command_func_t *);
extern int rl_bind_key_if_unbound_in_map (int, rl_command_func_t *, Keymap);
extern int rl_unbind_function_in_map (rl_command_func_t *, Keymap);
extern int rl_unbind_command_in_map (const char *, Keymap);
extern int rl_bind_keyseq (const char *, rl_command_func_t *);
extern int rl_bind_keyseq_in_map (const char *, rl_command_func_t *, Keymap);
extern int rl_bind_keyseq_if_unbound (const char *, rl_command_func_t *);
extern int rl_bind_keyseq_if_unbound_in_map (const char *, rl_command_func_t *, Keymap);
extern int rl_generic_bind (int, const char *, char *, Keymap);

extern char *rl_variable_value (const char *);
extern int rl_variable_bind (const char *, const char *);

/* Older names for binding key sequences. */
extern int rl_set_key (const char *, rl_command_func_t *, Keymap);
extern int rl_macro_bind (const char *, const char *, Keymap);

extern int rl_translate_keyseq (const char *, char *, int *);
extern char *rl_untranslate_keyseq (int);

extern rl_command_func_t *rl_named_function (const char *);
extern rl_command_func_t *rl_function_of_keyseq (const char *, Keymap, int *);
extern rl_command_func_t *rl_function_of_keyseq_len (const char *, size_t, Keymap, int *);
extern int rl_trim_arg_from_keyseq (const char *, size_t, Keymap);

extern void rl_list_funmap_names (void);
extern char **rl_invoking_keyseqs_in_map (rl_command_func_t *, Keymap);
extern char **rl_invoking_keyseqs (rl_command_func_t *);

extern void rl_print_keybinding (const char *, Keymap, int);
extern void rl_function_dumper (int);
extern void rl_macro_dumper (int);
extern void rl_variable_dumper (int);

extern int rl_read_init_file (const char *);
extern int rl_parse_and_bind (char *);

/* Keymaps. */
extern Keymap rl_make_bare_keymap (void);
extern int rl_empty_keymap (Keymap);
extern Keymap rl_copy_keymap (Keymap);
extern Keymap rl_make_keymap (void);
extern void rl_discard_keymap (Keymap);
extern void rl_free_keymap (Keymap);

extern Keymap rl_get_keymap_by_name (const char *);
extern char *rl_get_keymap_name (Keymap);
extern void rl_set_keymap (Keymap);
extern Keymap rl_get_keymap (void);
extern int rl_set_keymap_name (const char *, Keymap);
extern void rl_set_keymap_from_edit_mode (void);
extern char *rl_get_keymap_name_from_edit_mode (void);

/* Function map. */
extern int rl_add_funmap_entry (const char *, rl_command_func_t *);
extern const char **rl_funmap_names (void);
extern void rl_initialize_funmap (void);

/* Macros. */
extern void rl_push_macro_input (char *);

/* Undo. */
extern void rl_add_undo (enum undo_code, int, int, char *);
extern void rl_free_undo_list (void);
extern int rl_do_undo (void);
extern int rl_begin_undo_group (void);
extern int rl_end_undo_group (void);
extern int rl_modifying (int, int);

/* Redisplay. */
extern void rl_redisplay (void);
extern int rl_on_new_line (void);
extern int rl_on_new_line_with_prompt (void);
extern int rl_forced_update_display (void);
extern int rl_clear_visible_line (void);
extern int rl_clear_message (void);
extern int rl_reset_line_state (void);
extern int rl_crlf (void);

/* Active mark (region highlighting). */
extern void rl_keep_mark_active (void);
extern void rl_activate_mark (void);
extern void rl_deactivate_mark (void);
extern int rl_mark_active_p (void);

extern int rl_message (const char *, ...) __attribute__((__format__ (printf, 1, 2)));

extern int rl_show_char (int);
extern int rl_character_len (int, int);
extern void rl_redraw_prompt_last_line (void);

extern void rl_save_prompt (void);
extern void rl_restore_prompt (void);

/* Modifying the line buffer. */
extern void rl_replace_line (const char *, int);
extern int rl_insert_text (const char *);
extern int rl_delete_text (int, int);
extern int rl_kill_text (int, int);
extern char *rl_copy_text (int, int);

/* Terminal handling. */
extern void rl_prep_terminal (int);
extern void rl_deprep_terminal (void);
extern void rl_tty_set_default_bindings (Keymap);
extern void rl_tty_unset_default_bindings (Keymap);
extern int rl_tty_set_echoing (int);

extern int rl_reset_terminal (const char *);
extern void rl_resize_terminal (void);
extern void rl_set_screen_size (int, int);
extern void rl_get_screen_size (int *, int *);
extern void rl_reset_screen_size (void);

extern char *rl_get_termcap (const char *);
extern void rl_reparse_colors (void);

/* Character input. */
extern int rl_stuff_char (int);
extern int rl_execute_next (int);
extern int rl_clear_pending_input (void);
extern int rl_read_key (void);
extern int rl_getc (FILE *);
extern int rl_set_keyboard_input_timeout (int);

/* Whole-call timeouts. */
extern int rl_set_timeout (unsigned int, unsigned int);
extern int rl_timeout_remaining (unsigned int *, unsigned int *);

#undef rl_clear_timeout
#define rl_clear_timeout() rl_set_timeout (0, 0)

/* Utilities. */
extern void rl_extend_line_buffer (int);
extern int rl_ding (void);
extern int rl_alphabetic (int);
extern void rl_free (void *);

/* Signals. */
extern int rl_set_signals (void);
extern int rl_clear_signals (void);
extern void rl_cleanup_after_signal (void);
extern void rl_reset_after_signal (void);
extern void rl_free_line_state (void);

extern int rl_pending_signal (void);
extern void rl_check_signals (void);

extern void rl_echo_signal_char (int);

extern int rl_set_paren_blink_timeout (int);

/* History helpers living in libreadline. */
extern void rl_clear_history (void);
extern int rl_maybe_save_line (void);
extern int rl_maybe_unsave_line (void);
extern int rl_maybe_replace_line (void);

/* Completion. */
extern int rl_complete_internal (int);
extern void rl_display_match_list (char **, int, int);

extern char **rl_completion_matches (const char *, rl_compentry_func_t *);
extern char *rl_username_completion_function (const char *, int);
extern char *rl_filename_completion_function (const char *, int);

extern int rl_completion_mode (rl_command_func_t *);

/* ------------------------------------------------------------------ */
/* Variables                                                           */

extern const char *rl_library_version;
extern int rl_readline_version;
extern int rl_gnu_readline_p;

extern unsigned long rl_readline_state;

extern int rl_editing_mode;
extern int rl_insert_mode;

extern const char *rl_readline_name;

extern char *rl_prompt;
extern char *rl_display_prompt;

extern char *rl_line_buffer;
extern int rl_point;
extern int rl_end;
extern int rl_mark;

extern int rl_done;
extern int rl_eof_found;

extern int rl_pending_input;
extern int rl_dispatching;
extern int rl_explicit_arg;
extern int rl_numeric_arg;

extern rl_command_func_t *rl_last_func;

extern const char *rl_terminal_name;
extern FILE *rl_instream;
extern FILE *rl_outstream;
extern int rl_prefer_env_winsize;

/* Hooks. */
extern rl_hook_func_t *rl_startup_hook;
extern rl_hook_func_t *rl_pre_input_hook;
extern rl_hook_func_t *rl_event_hook;
extern rl_hook_func_t *rl_signal_event_hook;
extern rl_hook_func_t *rl_timeout_event_hook;
extern rl_hook_func_t *rl_input_available_hook;

extern rl_getc_func_t *rl_getc_function;
extern rl_voidfunc_t *rl_redisplay_function;
extern rl_vintfunc_t *rl_prep_term_function;
extern rl_voidfunc_t *rl_deprep_term_function;

extern rl_macro_print_func_t *rl_macro_display_hook;

/* Dispatch state. */
extern Keymap rl_executing_keymap;
extern Keymap rl_binding_keymap;
extern int rl_executing_key;
extern char *rl_executing_keyseq;
extern int rl_key_sequence_length;

extern int rl_erase_empty_line;
extern int rl_already_prompted;
extern int rl_num_chars_to_read;
extern char *rl_executing_macro;

/* Signal handling control. */
extern int rl_catch_signals;
extern int rl_catch_sigwinch;
extern int rl_change_environment;

/* Completion control. */
extern rl_compentry_func_t *rl_completion_entry_function;
extern rl_compentry_func_t *rl_menu_completion_entry_function;
extern rl_compignore_func_t *rl_ignore_some_completions_function;
extern rl_completion_func_t *rl_attempted_completion_function;

extern const char *rl_basic_word_break_characters;
extern const char *rl_completer_word_break_characters;
extern rl_cpvfunc_t *rl_completion_word_break_hook;
extern const char *rl_completer_quote_characters;
extern const char *rl_basic_quote_characters;
extern const char *rl_filename_quote_characters;
extern const char *rl_special_prefixes;

extern rl_icppfunc_t *rl_directory_completion_hook;
extern rl_icppfunc_t *rl_directory_rewrite_hook;
extern rl_icppfunc_t *rl_filename_stat_hook;
extern rl_dequote_func_t *rl_filename_rewrite_hook;
extern rl_dequote_func_t *rl_completion_rewrite_hook;

#define rl_symbolic_link_hook rl_directory_completion_hook

extern rl_compdisp_func_t *rl_completion_display_matches_hook;

extern int rl_filename_completion_desired;
extern int rl_filename_quoting_desired;
extern int rl_full_quoting_desired;
extern rl_quote_func_t *rl_filename_quoting_function;
extern rl_dequote_func_t *rl_filename_dequoting_function;
extern rl_linebuf_func_t *rl_char_is_quoted_p;

extern int rl_attempted_completion_over;
extern int rl_completion_type;
extern int rl_completion_invoking_key;
extern int rl_completion_query_items;
extern int rl_completion_append_character;
extern int rl_completion_suppress_append;
extern int rl_completion_quote_character;
extern int rl_completion_found_quote;
extern int rl_completion_suppress_quote;
extern int rl_sort_completion_matches;
extern int rl_completion_mark_symlink_dirs;
extern int rl_ignore_completion_duplicates;
extern int rl_inhibit_completion;

extern int rl_persistent_signal_handlers;

/* ------------------------------------------------------------------ */
/* Constants                                                           */

#define READERR (-2)

#define RL_PROMPT_START_IGNORE '\001'
#define RL_PROMPT_END_IGNORE '\002'

#define NO_MATCH 0
#define SINGLE_MATCH 1
#define MULT_MATCH 2

/* Bits of rl_readline_state. */
#define RL_STATE_NONE          0x0000000
#define RL_STATE_INITIALIZING  0x00000001
#define RL_STATE_INITIALIZED   0x00000002
#define RL_STATE_TERMPREPPED   0x00000004
#define RL_STATE_READCMD       0x00000008
#define RL_STATE_METANEXT      0x00000010
#define RL_STATE_DISPATCHING   0x00000020
#define RL_STATE_MOREINPUT     0x00000040
#define RL_STATE_ISEARCH       0x00000080
#define RL_STATE_NSEARCH       0x00000100
#define RL_STATE_SEARCH        0x00000200
#define RL_STATE_NUMERICARG    0x00000400
#define RL_STATE_MACROINPUT    0x00000800
#define RL_STATE_MACRODEF      0x00001000
#define RL_STATE_OVERWRITE     0x00002000
#define RL_STATE_COMPLETING    0x00004000
#define RL_STATE_SIGHANDLER    0x00008000
#define RL_STATE_UNDOING       0x00010000
#define RL_STATE_INPUTPENDING  0x00020000
#define RL_STATE_TTYCSAVED     0x00040000
#define RL_STATE_CALLBACK      0x00080000
#define RL_STATE_VIMOTION      0x00100000
#define RL_STATE_MULTIKEY      0x00200000
#define RL_STATE_VICMDONCE     0x00400000
#define RL_STATE_CHARSEARCH    0x00800000
#define RL_STATE_REDISPLAYING  0x01000000
#define RL_STATE_DONE          0x02000000
#define RL_STATE_TIMEOUT       0x04000000
#define RL_STATE_EOF           0x08000000
#define RL_STATE_READSTR       0x10000000

#define RL_SETSTATE(x)   (rl_readline_state |= (x))
#define RL_UNSETSTATE(x) (rl_readline_state &= ~(x))
#define RL_ISSTATE(x)    (rl_readline_state & (x))

/* Snapshot used by rl_save_state/rl_restore_state. */
struct readline_state {
  int point;
  int end;
  int mark;
  int buflen;
  char *buffer;
  UNDO_LIST *ul;
  char *prompt;

  int rlstate;
  int done;
  Keymap kmap;

  rl_command_func_t *lastfunc;
  int insmode;
  int edmode;
  char *kseq;
  int kseqlen;

  int pendingin;
  FILE *inf;
  FILE *outf;
  char *macro;

  int catchsigs;
  int catchsigwinch;

  rl_compentry_func_t *entryfunc;
  rl_compentry_func_t *menuentryfunc;
  rl_compignore_func_t *ignorefunc;
  rl_completion_func_t *attemptfunc;
  const char *wordbreakchars;

  char reserved[64];
};

extern int rl_save_state (struct readline_state *);
extern int rl_restore_state (struct readline_state *);

#ifdef __cplusplus
}
#endif

#endif /* _READLINE_H_ */
