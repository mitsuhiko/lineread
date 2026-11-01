/* lineread: public interface of a GNU Readline 8.3 compatible library.
   Compile-time feature configuration. */
#ifndef _RLCONF_H_
#define _RLCONF_H_

#define VI_MODE
#define VISIBLE_STATS
#define COLOR_SUPPORT
#define HANDLE_SIGNALS
#define PREFIX_META_HACK
#define DISPLAY_TABS
#define READLINE_CALLBACKS

#define DEFAULT_INPUTRC "~/.inputrc"
#define SYS_INPUTRC "/etc/inputrc"

#define RL_COMMENT_BEGIN_DEFAULT "#"

#define RL_EMACS_MODESTR_DEFAULT "@"
#define RL_EMACS_MODESTR_DEFLEN 1
#define RL_VI_INS_MODESTR_DEFAULT "(ins)"
#define RL_VI_INS_MODESTR_DEFLEN 5
#define RL_VI_CMD_MODESTR_DEFAULT "(cmd)"
#define RL_VI_CMD_MODESTR_DEFLEN 5

#define RL_ANSI_TERM_DEFAULT 1

#endif /* _RLCONF_H_ */
