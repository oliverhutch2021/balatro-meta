# Balatro Meta. Run from the repo root (Ubuntu / WSL2 on the PC, or the cloud).
.PHONY: help test test-rust rng-fixtures rng-stress fmt lint

CARGO  ?= cargo
LUAJIT ?= luajit
STRESS ?= 40

help:
	@echo "make test          all tests"
	@echo "make rng-fixtures  regenerate the LuaJIT reference fixture"
	@echo "make rng-stress    compare the RNG with LuaJIT on ~$(STRESS)x more cases"
	@echo "make fmt / lint    format / check formatting and clippy"

test: test-rust

test-rust:
	$(CARGO) test --workspace

rng-fixtures:
	$(LUAJIT) reference/rng_reference.lua > tests/fixtures/rng/reference.tsv

rng-stress:
	$(LUAJIT) reference/rng_reference.lua $(STRESS) > /tmp/bm_rng_stress.tsv
	BM_RNG_FIXTURE=/tmp/bm_rng_stress.tsv $(CARGO) test -p bm-rng --test luajit_fixtures --release

fmt:
	$(CARGO) fmt --all

lint:
	$(CARGO) fmt --all -- --check
	$(CARGO) clippy --workspace --all-targets -- -D warnings
