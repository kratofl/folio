# Everyday commands for Folio. `make` lists them.
#
# Works the same in the main checkout and in any git worktree. What belongs
# to one checkout (build output, scratch library, screenshots) stays per
# checkout. What's shared (fixtures, the Zed reference checkout) is found next
# to the main checkout, wherever the worktree lives.

# Rust installs cargo here, and not every terminal has it on PATH. The
# exported PATH reaches scripts, but make 3.81 (macOS) looks commands up in
# its own PATH, so recipes name cargo directly.
export PATH := $(HOME)/.cargo/bin:$(PATH)
CARGO := $(firstword $(wildcard $(HOME)/.cargo/bin/cargo) cargo)

# The main checkout, also when run from a worktree.
export MAIN := $(abspath $(shell git rev-parse --path-format=absolute --git-common-dir)/..)
export FIXTURES ?= $(abspath $(MAIN)/../folio-fixtures/2024)
export ZED_REF ?= $(abspath $(MAIN)/.refs/zed)
export GPUI_REV := $(shell sed -n 's/^gpui = .*rev = "\([0-9a-f]*\)".*/\1/p' Cargo.toml)

# Named after the checkout's folder, so worktrees never share a library.
CHECKOUT := $(notdir $(CURDIR))
SCRATCH ?= /tmp/folio-scratch/$(CHECKOUT)
SHOTS ?= /tmp/folio-shots/$(CHECKOUT)
# A copy-on-write clone: instant, and it takes no space until files change.
CLONE := $(if $(filter Darwin,$(shell uname)),cp -cR,cp -R --reflink=auto)

# `make run LIB=<folder>` opens a library. Empty opens the welcome screen.
LIB ?=
# `make screenshot KEYS="cmd-2 enter" SHOT=review` (see AGENTS.md).
KEYS ?=
SHOT ?= screenshot

.DEFAULT_GOAL := help
.PHONY: help paths setup test lint check run scratch run-scratch screenshot bundle

help: ## List the commands
	@grep -E '^[a-z-]+:.*## ' $(MAKEFILE_LIST) | awk -F':.*## ' '{printf "  make %-12s %s\n", $$1, $$2}'

paths: ## Show where the shared fixtures and Zed reference are
	@echo "MAIN=$(MAIN)"
	@echo "FIXTURES=$(FIXTURES)"
	@echo "ZED_REF=$(ZED_REF)"

setup: ## Prepare this checkout: reuse the main build, fetch crates, check references
	@scripts/setup.sh

test: ## Run the core tests
	$(CARGO) test -p folio-core

lint: ## Clippy on both crates, the screenshot harness included
	$(CARGO) clippy --release -p folio-core -p folio --all-targets -- -D warnings
	$(CARGO) clippy --release -p folio --features screenshot -- -D warnings

check: test lint ## Tests and lint, what to run before calling work done

run: ## Run the app (LIB=<folder> opens a library)
	$(CARGO) run --release -p folio -- $(LIB)

scratch: ## Make a fresh scratch library from the fixtures, for marking and moving
	@case "$(SCRATCH)" in /tmp/*) ;; *) echo "SCRATCH must be under /tmp, it gets deleted"; exit 1;; esac
	@test -d "$(FIXTURES)" || { echo "No fixtures at $(FIXTURES)"; exit 1; }
	rm -rf "$(SCRATCH)"
	mkdir -p "$(SCRATCH)"
	$(CLONE) "$(FIXTURES)" "$(SCRATCH)/"
	@echo "Scratch library: $(SCRATCH)/$(notdir $(FIXTURES))"

run-scratch: scratch ## Run the app on a fresh scratch library
	$(CARGO) run --release -p folio -- "$(SCRATCH)/$(notdir $(FIXTURES))"

screenshot: ## Render the app to a PNG (LIB, KEYS, SHOT), on the scratch library by default
	$(CARGO) build --release -p folio --features screenshot
	mkdir -p "$(SHOTS)"
	FOLIO_SCREENSHOT="$(SHOTS)/$(SHOT).png" FOLIO_KEYS="$(KEYS)" target/release/folio $(or $(LIB),"$(SCRATCH)/$(notdir $(FIXTURES))")

bundle: ## Build target/release/Folio.app (macOS)
	scripts/bundle-macos.sh
