# AppReestr build and install.
#   make                 build release binaries
#   make install         install into $(DESTDIR)$(PREFIX)
#   make test            run the test suite
#   make check           formatting, lints and tests (what CI runs)

PREFIX     ?= /usr
DESTDIR    ?=
LIBDIR     ?= $(PREFIX)/lib
DATADIR    ?= $(PREFIX)/share
CARGO      ?= cargo
# Cargo's target directory (CARGO_TARGET_DIR when set).
TARGET     ?= $(or $(CARGO_TARGET_DIR),target)
APP_ID     := io.github.appreestr.AppReestr
HELPER     := $(LIBDIR)/appreestr/appreestr-helper
LINGUAS    := $(shell cat po/LINGUAS)

export APPREESTR_HELPER_PATH := $(HELPER)
export APPREESTR_LOCALEDIR   := $(DATADIR)/locale

all: build

build:
	$(CARGO) build --release --locked -p appreestr -p appreestr-helper
	@for l in $(LINGUAS); do msgfmt -o $(TARGET)/release/$$l.mo po/$$l.po || exit 1; done

test:
	$(CARGO) test --workspace --locked

check:
	$(CARGO) fmt --all --check
	$(CARGO) clippy --workspace --all-targets --locked -- -D warnings
	$(CARGO) test --workspace --locked
	desktop-file-validate data/$(APP_ID).desktop
	appstreamcli validate --no-net --explain data/$(APP_ID).metainfo.xml || true

install:
	install -Dm755 $(TARGET)/release/appreestr        "$(DESTDIR)$(PREFIX)/bin/appreestr"
	install -Dm755 $(TARGET)/release/appreestr-helper "$(DESTDIR)$(HELPER)"
	install -Dm644 data/$(APP_ID).desktop          "$(DESTDIR)$(DATADIR)/applications/$(APP_ID).desktop"
	install -Dm644 data/$(APP_ID).metainfo.xml     "$(DESTDIR)$(DATADIR)/metainfo/$(APP_ID).metainfo.xml"
	install -Dm644 data/icons/hicolor/scalable/apps/$(APP_ID).svg \
		"$(DESTDIR)$(DATADIR)/icons/hicolor/scalable/apps/$(APP_ID).svg"
	install -Dm644 data/icons/hicolor/symbolic/apps/$(APP_ID)-symbolic.svg \
		"$(DESTDIR)$(DATADIR)/icons/hicolor/symbolic/apps/$(APP_ID)-symbolic.svg"
	install -d "$(DESTDIR)$(DATADIR)/polkit-1/actions"
	sed 's|/usr/lib/appreestr/appreestr-helper|$(HELPER)|' data/$(APP_ID).policy \
		> "$(DESTDIR)$(DATADIR)/polkit-1/actions/$(APP_ID).policy"
	@for l in $(LINGUAS); do \
		install -Dm644 $(TARGET)/release/$$l.mo "$(DESTDIR)$(DATADIR)/locale/$$l/LC_MESSAGES/appreestr.mo"; \
	done
	install -Dm644 LICENSE "$(DESTDIR)$(DATADIR)/licenses/appreestr/LICENSE"

uninstall:
	rm -f "$(DESTDIR)$(PREFIX)/bin/appreestr" "$(DESTDIR)$(HELPER)"
	rm -f "$(DESTDIR)$(DATADIR)/applications/$(APP_ID).desktop" "$(DESTDIR)$(DATADIR)/metainfo/$(APP_ID).metainfo.xml"
	rm -f "$(DESTDIR)$(DATADIR)/icons/hicolor/scalable/apps/$(APP_ID).svg"
	rm -f "$(DESTDIR)$(DATADIR)/icons/hicolor/symbolic/apps/$(APP_ID)-symbolic.svg"
	rm -f "$(DESTDIR)$(DATADIR)/polkit-1/actions/$(APP_ID).policy"
	@for l in $(LINGUAS); do rm -f "$(DESTDIR)$(DATADIR)/locale/$$l/LC_MESSAGES/appreestr.mo"; done

pot:
	tools/extract-strings.py
	@for l in $(LINGUAS); do msgmerge --update --backup=none po/$$l.po po/appreestr.pot; done

.PHONY: all build test check install uninstall pot
