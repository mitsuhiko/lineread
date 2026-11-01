/* lineread: public interface of a GNU Readline 8.3 compatible library.
   Keymaps. */
#ifndef _KEYMAPS_H_
#define _KEYMAPS_H_

#ifdef __cplusplus
extern "C" {
#endif

#ifdef READLINE_LIBRARY
#  include "rlstdc.h"
#  include "chardefs.h"
#  include "rltypedefs.h"
#else
#  include <readline/rlstdc.h>
#  include <readline/chardefs.h>
#  include <readline/rltypedefs.h>
#endif

/* One slot of a keymap.  TYPE is ISFUNC, ISKMAP or ISMACR; FUNCTION holds a
   command, a Keymap (cast) or a macro string (cast) accordingly. */
typedef struct _keymap_entry {
  char type;
  rl_command_func_t *function;
} KEYMAP_ENTRY;

/* 256 byte values plus one slot for "any other key". */
#define KEYMAP_SIZE 257
#define ANYOTHERKEY KEYMAP_SIZE-1

typedef KEYMAP_ENTRY KEYMAP_ENTRY_ARRAY[KEYMAP_SIZE];
typedef KEYMAP_ENTRY *Keymap;

#define ISFUNC 0
#define ISKMAP 1
#define ISMACR 2

extern KEYMAP_ENTRY_ARRAY emacs_standard_keymap, emacs_meta_keymap, emacs_ctlx_keymap;
extern KEYMAP_ENTRY_ARRAY vi_insertion_keymap, vi_movement_keymap;

extern Keymap rl_make_bare_keymap (void);
extern Keymap rl_copy_keymap (Keymap);
extern Keymap rl_make_keymap (void);
extern void rl_discard_keymap (Keymap);

extern Keymap rl_get_keymap_by_name (const char *);
extern Keymap rl_get_keymap (void);
extern void rl_set_keymap (Keymap);
extern int rl_set_keymap_name (const char *, Keymap);

#ifdef __cplusplus
}
#endif

#endif /* _KEYMAPS_H_ */
