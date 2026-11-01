# Build the test driver and conformance programs.
#
#   make [IMPL=ref|cov|asan|rust]      build build/<platform>/<impl>/rl-driver
#   make conformance [IMPL=...]        run the header/ABI conformance check
#
# IMPL selects what the driver links against:
#   ref   GNU readline reference, plain build   ($(REF)/build-plain, or
#         $(REF_BIN)/<platform>/lib/plain without the source tree)
#   cov   GNU readline reference, coverage build ($(REF)/build-cov, source only)
#   asan  GNU readline reference, ASan+UBSan build ($(REF)/build-asan, or
#         $(REF_BIN)/<platform>/lib/asan)
#   rust  the Rust implementation ($(RUST_LIB), built with cargo from impl/)
#   rust-asan  the same with the driver under ASan+UBSan

REF ?= $(abspath $(or $(LINEREAD_REF),../lineread-ref))
# compiled reference without source (ref/export-bin.sh), used when the
# reference build directories are absent or REF_FROM_BIN=1
REF_BIN ?= $(abspath $(or $(LINEREAD_REF_BIN),../lineread-ref-bin))
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
# reference library: source build if present, else the exported binaries
ref_lib = $(if $(and $(wildcard $(REF)/build-$(1)$(REFSUF)/libreadline.a),$(if $(REF_FROM_BIN),,1)),$(REF)/build-$(1)$(REFSUF)/libreadline.a,$(REF_BIN)/$(PLAT)/lib/$(1)/libreadline.a)

ifeq ($(IMPL),ref)
  CFLAGS := $(CFLAGS_BASE) -O1
  IMPL_LIBS := $(call ref_lib,plain)
else ifeq ($(IMPL),cov)
  CFLAGS := $(CFLAGS_BASE) -O0
  LDFLAGS += -fprofile-instr-generate
  IMPL_LIBS := $(REF)/build-cov$(REFSUF)/libreadline.a
else ifeq ($(IMPL),asan)
  CFLAGS := $(CFLAGS_BASE) -O1 -fno-omit-frame-pointer -fsanitize=address,undefined
  LDFLAGS += -fsanitize=address,undefined
  IMPL_LIBS := $(call ref_lib,asan)
else ifeq ($(IMPL),rust)
  CFLAGS := $(CFLAGS_BASE) -O1
  IMPL_LIBS := $(RUST_LIB)
  RUST_BUILD := 1
else ifeq ($(IMPL),rust-asan)
  # driver under ASan+UBSan (catches ownership bugs at the C ABI: memory
  # freed by the driver must come from malloc), Rust library unchanged
  CFLAGS := $(CFLAGS_BASE) -O1 -fno-omit-frame-pointer -fsanitize=address,undefined
  LDFLAGS += -fsanitize=address,undefined
  IMPL_LIBS := $(RUST_LIB)
  RUST_BUILD := 1
else
  $(error unknown IMPL=$(IMPL))
endif

ifdef RUST_BUILD
  ifeq ($(UNAME_S),linux)
    LDLIBS += -lpthread -ldl -lm
  endif
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

# The Rust implementation: impl/ is a cargo package whose staticlib is
# liblineread.a (docs/IMPLEMENTING.md).  cargo decides what to rebuild.
ifdef RUST_BUILD
ifeq ($(RUST_LIB),impl/target/release/liblineread.a)
$(RUST_LIB): rust-lib
endif
endif
rust-lib:
	@test -f impl/Cargo.toml || { echo "impl/Cargo.toml missing: see docs/IMPLEMENTING.md" >&2; exit 1; }
	cargo build --release --manifest-path impl/Cargo.toml

# header/ABI conformance against conformance/abi.<platform>.json
conformance:
	python3 conformance/check.py --impl $(IMPL)

print-libs:
	@echo $(IMPL_LIBS)

clean:
	rm -rf build

.PHONY: all clean conformance print-libs rust-lib
