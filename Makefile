QMLLINT := /usr/lib/qt6/bin/qmllint
QML_FILES := Panel.qml Service.qml TorIcon.qml TorWordmark.qml SpeedGauge.qml

.PHONY: check check-root test qml-check hooks install reinstall doctor validate help

# Everything CI runs. Same script, so green here is green there.
check:
	.github/scripts/check.sh

# Under sudo the nftables ruleset is validated against the kernel, which is the
# check most worth having and the only one that needs root.
check-root:
	sudo .github/scripts/check.sh

test:
	python3 tests/test_qml_names.py
	bash tests/test_decisions.sh
	node tests/test_model.js


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

install:
	sudo ./tormarchy setup

reinstall:
	sudo ./tormarchy uninstall
	sudo ./tormarchy setup
	omarchy-restart-shell

doctor:
	./tormarchy doctor

validate: check test
	omarchy plugin validate .
	git diff --check

help:
	@echo "hooks        enable auto-versioning on commit"
	@echo "check        every CI check"
	@echo "check-root   the same, plus nft validation against the kernel"
	@echo "test         name collisions, pinned decisions, Model.js"
	@echo "qml-check    qmllint, import noise filtered out"
	@echo "install      sudo ./tormarchy setup"
	@echo "reinstall    uninstall, setup, restart the shell"
	@echo "doctor       leak checks"
	@echo "validate     check + test + omarchy plugin validate"
