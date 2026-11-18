PREFIX ?= $(HOME)/.local
BIN     = saturn-art
CARGO  ?= cargo

.PHONY: all build run install uninstall test snapshot screensaver screensaver-off clean

all: build

build:
	$(CARGO) build --release

run: build
	./target/release/$(BIN)

install: build
	install -Dm755 target/release/$(BIN) $(PREFIX)/bin/$(BIN)

uninstall:
	rm -f $(PREFIX)/bin/$(BIN)

test:
	$(CARGO) test --release

screensaver: install
	$(PREFIX)/bin/$(BIN) screensaver on

screensaver-off:
	$(PREFIX)/bin/$(BIN) screensaver off

snapshot: build
	./target/release/$(BIN) --snapshot preview.ppm --cols 240 --rows 70 --time 6

clean:
	$(CARGO) clean
