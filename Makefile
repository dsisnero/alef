ALEF     ?= $(abspath target/debug/alef)
XBURG    ?= $(abspath ../../xberg-io)

.PHONY: all build setup \
        regen-crystal \

# Build alef binary from source
build:
	cargo build -p alef

# Install development dependencies
setup:
	rustup update stable
	cargo install cargo-deny cargo-machete cargo-sort || true

# Regenerate all bindings for all xberg-io repositories
# (Crystal + FFI + full generate for workspace integrity)
regen-crystal: build
	@echo "=== regenerating crawlberg ================="
	cd $(XBURG)/crawlberg && $(ALEF) generate 2>&1 | tail -3
	cd $(XBURG)/crawlberg && $(ALEF) e2e generate --lang crystal 2>&1 | tail -3
	@echo "=== regenerating html-to-markdown ========="
	cd $(XBURG)/html-to-markdown && $(ALEF) generate 2>&1 | tail -3
	cd $(XBURG)/html-to-markdown && $(ALEF) e2e generate --lang crystal 2>&1 | tail -3
	@echo "=== regenerating liter-llm ================"
	cd $(XBURG)/liter-llm && $(ALEF) generate 2>&1 | tail -3
	cd $(XBURG)/liter-llm && $(ALEF) e2e generate --lang crystal 2>&1 | tail -3
	@echo "=== regenerating tree-sitter-language-pack ="
	cd $(XBURG)/tree-sitter-language-pack && $(ALEF) generate 2>&1 | tail -3
	cd $(XBURG)/tree-sitter-language-pack && $(ALEF) e2e generate --lang crystal 2>&1 | tail -3
	@echo "=== regenerating xberg ===================="
	cd $(XBURG)/xberg && $(ALEF) generate 2>&1 | tail -3
	cd $(XBURG)/xberg && $(ALEF) e2e generate --lang crystal 2>&1 | tail -3
	@echo "=== all bindings regenerated ======"

# Regenerate all bindings for a single repo
regen-crystal-%: build
	cd $(XBURG)/$* && $(ALEF) generate 2>&1 | tail -3
	cd $(XBURG)/$* && $(ALEF) e2e generate --lang crystal 2>&1 | tail -3
