BIN ?= $(HOME)/bin/domux
SRC := $(wildcard *.go)
OUT := domux

.PHONY: all build install uninstall test vet run switcher todo clean

all: build

build: $(OUT)

$(OUT): $(SRC)
	go build -o $(OUT) .

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
