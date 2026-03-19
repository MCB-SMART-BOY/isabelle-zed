SHELL := /bin/bash
.ONESHELL:

XTASK := cargo run -p isabelle-zed-xtask --

.PHONY: bridge-build bridge-test bridge-clippy \
	 adapter-test \
	 lsp-build lsp-test lsp-clippy \
	 zed-build zed-check \
	 zed-official-check \
	 build-isabelle-grammar \
	 doctor \
	 release-build \
	 install-local \
	 install-zed-native \
	 uninstall-zed-native \
	 install-zed-shortcuts \
	 uninstall-zed-shortcuts \
	 release-package \
	 bridge-mock-up bridge-mock-down \
	 mock-bridge mock-bridge-adapter mock-adapter mock-send mock-lsp-e2e \
	 native-lsp-smoke spawn-e2e-ndjson

bridge-build:
	cargo build -p isabelle-bridge

bridge-test:
	cargo test -p isabelle-bridge

bridge-clippy:
	cargo clippy -p isabelle-bridge -- -D warnings

adapter-test:
	cargo test -p isabelle-bridge process::real_adapter_tests

lsp-build:
	cargo build -p isabelle-zed-lsp

lsp-test:
	cargo test -p isabelle-zed-lsp

lsp-clippy:
	cargo clippy -p isabelle-zed-lsp -- -D warnings

zed-build:
	cargo build -p isabelle-zed-extension --target wasm32-wasip2

zed-check:
	cargo check -p isabelle-zed-extension

zed-official-check:
	$(XTASK) zed-official-check

build-isabelle-grammar:
	$(XTASK) build-isabelle-grammar

doctor:
	$(XTASK) doctor

release-build:
	$(XTASK) release-build

install-local:
	$(XTASK) install-local

install-zed-native:
	$(XTASK) install-zed-native

uninstall-zed-native:
	$(XTASK) uninstall-zed-native

install-zed-shortcuts:
	$(XTASK) install-zed-shortcuts

uninstall-zed-shortcuts:
	$(XTASK) uninstall-zed-shortcuts

release-package:
	$(XTASK) release-package

bridge-mock-up:
	$(XTASK) bridge-mock-up /tmp/isabelle.sock

bridge-mock-down:
	$(XTASK) bridge-mock-down /tmp/isabelle.sock

mock-bridge:
	cargo run -p isabelle-bridge -- --mock --socket /tmp/isabelle.sock

mock-bridge-adapter:
	cargo run -p isabelle-bridge -- --socket /tmp/isabelle.sock --adapter-socket 127.0.0.1:9011

mock-adapter:
	cargo run -p isabelle-bridge -- --real-adapter

mock-send:
	$(XTASK) mock-send /tmp/isabelle.sock

mock-lsp-e2e:
	$(XTASK) mock-lsp-e2e

native-lsp-smoke:
	$(XTASK) native-lsp-smoke

spawn-e2e-ndjson:
	$(XTASK) spawn-e2e-ndjson
