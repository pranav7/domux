BIN ?= $(HOME)/bin/domux
SRC := $(wildcard *.go)
OUT := domux
VERSION ?= dev

.PHONY: all build install uninstall test vet run switcher todo clean release-local

all: build

build: $(OUT)

$(OUT): $(SRC)
	go build -ldflags "-X main.version=$(VERSION)" -o $(OUT) .

install: build
	@mkdir -p $(dir $(BIN))
	@if [ -e $(BIN) ] && [ ! -L $(BIN) ]; then \
		mv $(BIN) $(BIN).pre-symlink.bak; \
		echo "backed up existing $(BIN) -> $(BIN).pre-symlink.bak"; \
	fi
	ln -sfn $(CURDIR)/$(OUT) $(BIN)
	@echo "linked $(BIN) -> $(CURDIR)/$(OUT)"

uninstall:
	@if [ -L $(BIN) ]; then rm $(BIN) && echo "removed symlink $(BIN)"; fi

test:
	go test ./...

vet:
	go vet ./...

run: build
	./$(OUT)

switcher: build
	./$(OUT) switcher

todo: build
	./$(OUT) todo

clean:
	rm -f $(OUT)
	rm -rf dist

release-local:
	@mkdir -p dist
	@for goarch in arm64 amd64; do \
		v_body=$$(echo "$(VERSION)" | sed 's/^v//'); \
		outdir="dist/darwin_$$goarch"; \
		mkdir -p $$outdir; \
		echo "==> building darwin/$$goarch"; \
		GOOS=darwin GOARCH=$$goarch CGO_ENABLED=0 \
			go build -trimpath -ldflags "-s -w -X main.version=$(VERSION)" -o $$outdir/$(OUT) . || exit 1; \
		tar -czf dist/domux_$${v_body}_darwin_$${goarch}.tar.gz -C $$outdir $(OUT); \
	done
	@cd dist && shasum -a 256 domux_*.tar.gz > SHA256SUMS
	@echo "==> dist/"
	@ls -1 dist/
