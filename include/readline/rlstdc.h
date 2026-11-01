/* lineread: public interface of a GNU Readline 8.3 compatible library.
   Compiler compatibility helpers. */
#ifndef _RL_STDC_H_
#define _RL_STDC_H_

#ifndef PARAMS
#  if defined(__STDC__) || defined(__GNUC__) || defined(__cplusplus)
#    define PARAMS(protos) protos
#  else
#    define PARAMS(protos) ()
#  endif
#endif

#ifndef __attribute__
#  if __GNUC__ < 2 || (__GNUC__ == 2 && __GNUC_MINOR__ < 8)
#    define __attribute__(x)
#  endif
#endif

#endif /* _RL_STDC_H_ */
