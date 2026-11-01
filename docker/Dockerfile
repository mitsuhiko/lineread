# Linux environment for building the reference and running the suite.
FROM debian:bookworm
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      clang llvm libclang-rt-dev make git curl ca-certificates libncurses-dev ncurses-base ncurses-term \
      python3 python3-venv python3-dev unzip binutils locales \
    && rm -rf /var/lib/apt/lists/*
COPY requirements.txt /tmp/requirements.txt
RUN python3 -m venv /opt/venv && /opt/venv/bin/pip install -q -r /tmp/requirements.txt
ENV PATH=/opt/venv/bin:$PATH CC=clang
WORKDIR /work/lineread
