/* lineread: public interface of a GNU Readline 8.3 compatible library.
   Character classification and key constant helpers. */
#ifndef _CHARDEFS_H_
#define _CHARDEFS_H_

#include <ctype.h>

#ifdef HAVE_CONFIG_H
#  ifdef HAVE_STRING_H
#    include <string.h>
#  endif
#  ifdef HAVE_STRINGS_H
#    include <strings.h>
#  endif
#else
#  include <string.h>
#endif

#ifndef whitespace
#define whitespace(c) (((c) == ' ') || ((c) == '\t'))
#endif

#define control_character_threshold 0x020
#define control_character_mask 0x1f
#define meta_character_threshold 0x07f
#define control_character_bit 0x40
#define meta_character_bit 0x080
#define largest_char 255

#define CTRL_CHAR(c) ((c) < control_character_threshold && (((c) & 0x80) == 0))
#define META_CHAR(c) ((unsigned char)(c) > meta_character_threshold && (unsigned char)(c) <= largest_char)

#undef CTRL
#undef UNCTRL
#define CTRL(c) ((c) & control_character_mask)
#define META(c) ((c) | meta_character_bit)
#define UNMETA(c) ((c) & (~meta_character_bit))
#define UNCTRL(c) _rl_to_upper(((c)|control_character_bit))

#ifndef UCHAR_MAX
#  define UCHAR_MAX 255
#endif
#ifndef CHAR_MAX
#  define CHAR_MAX 127
#endif

#if defined(HAVE_STDLIB_H) && defined(HAVE_STRING_H)
#  define IN_CTYPE_DOMAIN(c) 1
#  define NON_NEGATIVE(c) 1
#else
#  define IN_CTYPE_DOMAIN(c) ((c) >= 0 && (c) <= CHAR_MAX)
#  define NON_NEGATIVE(c) ((unsigned char)(c) == (c))
#endif

#if !defined(isxdigit) && !defined(HAVE_ISXDIGIT) && !defined(__cplusplus)
#  define isxdigit(c) (isdigit((unsigned char)(c)) || ((c) >= 'a' && (c) <= 'f') || ((c) >= 'A' && (c) <= 'F'))
#endif

#undef ISPRINT
#define ISALNUM(c) (IN_CTYPE_DOMAIN (c) && isalnum ((unsigned char)c))
#define ISALPHA(c) (IN_CTYPE_DOMAIN (c) && isalpha ((unsigned char)c))
#define ISDIGIT(c) (IN_CTYPE_DOMAIN (c) && isdigit ((unsigned char)c))
#define ISLOWER(c) (IN_CTYPE_DOMAIN (c) && islower ((unsigned char)c))
#define ISPRINT(c) (IN_CTYPE_DOMAIN (c) && isprint ((unsigned char)c))
#define ISUPPER(c) (IN_CTYPE_DOMAIN (c) && isupper ((unsigned char)c))
#define ISXDIGIT(c) (IN_CTYPE_DOMAIN (c) && isxdigit ((unsigned char)c))

#define _rl_lowercase_p(c) (NON_NEGATIVE(c) && ISLOWER(c))
#define _rl_uppercase_p(c) (NON_NEGATIVE(c) && ISUPPER(c))
#define _rl_digit_p(c) ((c) >= '0' && (c) <= '9')
#define _rl_alphabetic_p(c) (NON_NEGATIVE(c) && ISALNUM(c))
#define _rl_pure_alphabetic(c) (NON_NEGATIVE(c) && ISALPHA(c))

#ifndef _rl_to_upper
#  define _rl_to_upper(c) (_rl_lowercase_p(c) ? toupper((unsigned char)(c)) : (c))
#  define _rl_to_lower(c) (_rl_uppercase_p(c) ? tolower((unsigned char)(c)) : (c))
#endif

#ifndef _rl_digit_value
#  define _rl_digit_value(x) ((x) - '0')
#endif

#ifndef _rl_isident
#  define _rl_isident(c) (ISALNUM(c) || (c) == '_')
#endif

#ifndef ISOCTAL
#  define ISOCTAL(c) ((c) >= '0' && (c) <= '7')
#endif
#define OCTVALUE(c) ((c) - '0')
#define HEXVALUE(c) \
  (((c) >= 'a' && (c) <= 'f') ? (c)-'a'+10 : \
   ((c) >= 'A' && (c) <= 'F') ? (c)-'A'+10 : (c)-'0')

#ifndef NEWLINE
#define NEWLINE '\n'
#endif
#ifndef RETURN
#define RETURN CTRL('M')
#endif
#ifndef RUBOUT
#define RUBOUT 0x7f
#endif
#ifndef TAB
#define TAB '\t'
#endif

#undef ABORT_CHAR
#define ABORT_CHAR CTRL('G')
#undef PAGE
#define PAGE CTRL('L')
#undef SPACE
#define SPACE ' '
#undef ESC
#define ESC CTRL('[')

#endif /* _CHARDEFS_H_ */
