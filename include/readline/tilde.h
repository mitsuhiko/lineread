/* lineread: public interface of a GNU Readline 8.3 compatible library.
   Tilde expansion. */
#ifndef _TILDE_H_
#define _TILDE_H_

#ifdef __cplusplus
extern "C" {
#endif

typedef char *tilde_hook_func_t (char *);

extern tilde_hook_func_t *tilde_expansion_preexpansion_hook;
extern tilde_hook_func_t *tilde_expansion_failure_hook;

extern char **tilde_additional_prefixes;
extern char **tilde_additional_suffixes;

extern char *tilde_expand (const char *);
extern char *tilde_expand_word (const char *);
extern char *tilde_find_word (const char *, int, int *);

#ifdef __cplusplus
}
#endif

#endif /* _TILDE_H_ */
