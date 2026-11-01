# Build the test driver and conformance programs.
#
#   make [IMPL=ref|cov|asan|rust]      build build/<platform>/<impl>/rl-driver
#   make conformance [IMPL=...]        build the ABI check program
#
# IMPL selects what the driver links against:
#   ref   GNU readline reference, plain build   ($(REF)/build-plain)
#   cov   GNU readline reference, coverage build ($(REF)/build-cov)
#   asan  GNU readline reference, ASan+UBSan build ($(REF)/build-asan)
#   rust  the Rust implementation ($(RUST_LIB))

REF ?= $(abspath ../lineread-ref)
IMPL ?= ref
CC ?= cc
ifeq ($(origin CC),default)
CC := clang
endif

UNAME_S := $(shell uname -s | tr A-Z a-z)
UNAME_M := $(shell uname -m)
PLAT := $(UNAME_S)-$(UNAME_M)
ifeq ($(UNAME_S),darwin)
REFSUF :=
else
REFSUF := -$(PLAT)
endif

B := build/$(PLAT)/$(IMPL)
TERMLIB ?= -lncurses
RUST_LIB ?= impl/target/release/liblineread.a

CFLAGS_BASE := -std=c11 -g -Wall -Wextra -Wno-unused-parameter -Iinclude -I$(B)
LDLIBS :=
ifeq ($(IMPL),ref)
  CFLAGS := $(CFLAGS_BASE) -O1
  IMPL_LIBS := $(REF)/build-plain$(REFSUF)/libreadline.a
else ifeq ($(IMPL),cov)
  CFLAGS := $(CFLAGS_BASE) -O0
  LDFLAGS += -fprofile-instr-generate
  IMPL_LIBS := $(REF)/build-cov$(REFSUF)/libreadline.a
else ifeq ($(IMPL),asan)
  CFLAGS := $(CFLAGS_BASE) -O1 -fno-omit-frame-pointer -fsanitize=address,undefined
  LDFLAGS += -fsanitize=address,undefined
  IMPL_LIBS := $(REF)/build-asan$(REFSUF)/libreadline.a
else ifeq ($(IMPL),rust)
  CFLAGS := $(CFLAGS_BASE) -O1
  IMPL_LIBS := $(RUST_LIB)
  ifeq ($(UNAME_S),linux)
    LDLIBS += -lpthread -ldl -lm
  endif
else
  $(error unknown IMPL=$(IMPL))
endif

HEADERS := $(wildcard include/readline/*.h)

all: $(B)/rl-driver

$(B)/bindable.inc: $(HEADERS)
	@mkdir -p $(B)
	$(CC) -E -P -Iinclude -include stdio.h include/readline/readline.h | \
	  sed -n 's/^extern int \(rl_[A-Za-z_]*\) *(int, *int);$$/  API(\1, "i:ii"),/p' > $@

$(B)/rl-driver.o: driver/rl-driver.c $(B)/bindable.inc $(HEADERS)
	$(CC) $(CFLAGS) -c -o $@ driver/rl-driver.c

$(B)/rl-driver: $(B)/rl-driver.o $(IMPL_LIBS)
	$(CC) $(LDFLAGS) -o $@ $(B)/rl-driver.o $(IMPL_LIBS) $(TERMLIB) $(LDLIBS)

conformance: $(B)/abi-check $(B)/symbols-check

$(B)/abi-check: conformance/abi_check.c $(HEADERS)
	@mkdir -p $(B)
	$(CC) $(CFLAGS_BASE) -o $@ conformance/abi_check.c

$(B)/symbols-check: conformance/symbols_check.c $(IMPL_LIBS)
	@mkdir -p $(B)
	$(CC) $(CFLAGS_BASE) $(filter -fsanitize%,$(CFLAGS)) $(LDFLAGS) -o $@ conformance/symbols_check.c $(IMPL_LIBS) $(TERMLIB) $(LDLIBS)

clean:
	rm -rf build

.PHONY: all clean conformance
