QMLLINT := /usr/lib/qt6/bin/qmllint
QML_FILES := Panel.qml Service.qml TorIcon.qml TorWordmark.qml SpeedGauge.qml

.PHONY: check check-root test build release shell screenshots qml-check hooks install install-dev reinstall doctor validate help

# Everything CI runs. Same script, so green here is green there.
check:
	.github/scripts/check.sh

# Under sudo the nftables ruleset is validated against the kernel, which is the
# check most worth having and the only one that needs root.
check-root:
	sudo .github/scripts/check.sh

test:
	cargo test --workspace --locked
	cargo build --locked
	python3 tests/daemon/scenarios.py target/debug/tormarchyd
	bash tests/test_decisions.sh
	python3 tests/test_qml_names.py

build:
	cargo build --locked

# What the release ships: static, so it runs on any x86_64 Omarchy.
release:
	cargo build --release --locked --target x86_64-unknown-linux-musl

# The widget under a real Qt engine, in every state, against Omarchy's shell.
# Needs PySide6; OMARCHY_PATH=~/src/omarchy to test against a checkout.
shell:
	.github/scripts/shell.sh

# The README's pictures and the marketplace preview, rendered from the QML.
# Needs PySide6, JetBrains Mono, and OMARCHY_PATH pointing at a checkout.
screenshots:
	python3 tests/qml/screenshots.py --omarchy "$${OMARCHY_PATH:-/usr/share/omarchy}"

# Needs Qt. qs.Commons and qs.Ui cannot resolve without a full Omarchy install,
# so most output is import noise -- but it still catches unused imports and
# genuine property mistakes, which is why it is worth running.
qml-check:
	$(QMLLINT) -I /usr/share/omarchy/shell $(QML_FILES) 2>&1 \
	  | grep -vE 'qs\.(Commons|Ui)|Failed to import|Unqualified access|unresolved-type|was not found|ComponentBehavior|Did you mean|^\s*\^|^---$$|^import |^$$' \
	  || true

# Auto-versioning lives in a pre-commit hook, so a fresh clone has to opt in.
hooks:
	git config core.hooksPath .githooks
	@echo "  pre-commit will now bump the patch version when source changes"

# The published release, verified, the way every user installs it.
install:
	sudo ./setup

# What you just built, unverified: for working on it.
install-dev: release
	sudo ./setup --local target/x86_64-unknown-linux-musl/release

reinstall:
	sudo tormarchy uninstall
	sudo ./setup
	omarchy-restart-shell

doctor:
	tormarchy doctor

validate: check test
	omarchy plugin validate .
	git diff --check

help:
	@echo "hooks        enable auto-versioning on commit"
	@echo "check        every CI check"
	@echo "check-root   the same, plus nft validation against the kernel"
	@echo "test         Rust tests, daemon scenarios, pinned decisions, names"
	@echo "build        debug build of the helper and the daemon"
	@echo "release      static release build"
	@echo "shell        load the widget in Qt in every state, and run its scenarios"
	@echo "screenshots  re-render the README pictures and preview.png"
	@echo "qml-check    qmllint, import noise filtered out"
	@echo "install      sudo ./setup: the verified release"
	@echo "install-dev  build, then install that build, unverified"
	@echo "reinstall    uninstall, setup, restart the shell"
	@echo "doctor       leak checks"
	@echo "validate     check + test + omarchy plugin validate"
