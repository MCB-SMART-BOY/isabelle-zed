SHELL := /bin/bash
.ONESHELL:

.PHONY: bridge-build bridge-test bridge-clippy \
	 scala-test \
	 lsp-build lsp-test lsp-clippy \
	 zed-build zed-check \
	 doctor \
	 release-build \
	 install-local \
	 install-zed-native \
	 uninstall-zed-native \
	 release-package \
	 bridge-mock-up bridge-mock-down \
	 mock-bridge mock-bridge-adapter mock-adapter mock-send mock-lsp-e2e \
	 native-lsp-smoke

bridge-build:
	cargo build --manifest-path bridge/Cargo.toml

bridge-test:
	cargo test --manifest-path bridge/Cargo.toml

bridge-clippy:
	cargo clippy --manifest-path bridge/Cargo.toml -- -D warnings

scala-test:
	cd scala-adapter && sbt test

lsp-build:
	cargo build --manifest-path isabelle-lsp/Cargo.toml

lsp-test:
	cargo test --manifest-path isabelle-lsp/Cargo.toml

lsp-clippy:
	cargo clippy --manifest-path isabelle-lsp/Cargo.toml -- -D warnings

zed-build:
	cargo build --manifest-path zed-extension/Cargo.toml --target wasm32-wasip2

zed-check:
	cargo check --manifest-path zed-extension/Cargo.toml

doctor:
	./scripts/doctor.sh

release-build:
	./scripts/build_release.sh

install-local:
	./scripts/install_local.sh

install-zed-native:
	./scripts/install_zed_native.sh

uninstall-zed-native:
	./scripts/uninstall_zed_native.sh

release-package:
	./scripts/package_release.sh

bridge-mock-up:
	./scripts/bridge_mock_up.sh /tmp/isabelle.sock

bridge-mock-down:
	./scripts/bridge_mock_down.sh /tmp/isabelle.sock

mock-bridge:
	cargo run --manifest-path bridge/Cargo.toml -- --mock --socket /tmp/isabelle.sock

mock-bridge-adapter:
	cargo run --manifest-path bridge/Cargo.toml -- --socket /tmp/isabelle.sock --adapter-socket 127.0.0.1:9011

mock-adapter:
	cd scala-adapter && sbt "run --mock --socket=127.0.0.1:9011"

mock-send:
	python3 scripts/mock_send.py

mock-lsp-e2e:
	cargo build --manifest-path bridge/Cargo.toml
	cargo build --manifest-path isabelle-lsp/Cargo.toml
	cargo run --manifest-path bridge/Cargo.toml -- --mock --socket /tmp/isabelle.sock >/tmp/bridge-local.log 2>&1 &
	bridge_pid=$$!
	trap 'kill $$bridge_pid 2>/dev/null || true; rm -f /tmp/isabelle.sock' EXIT
	for i in $$(seq 1 80); do
	  if [ -S /tmp/isabelle.sock ]; then
	    break
	  fi
	  sleep 0.1
	done
	python3 scripts/mock_lsp_e2e.py

native-lsp-smoke:
	python3 scripts/native_lsp_smoke.py
