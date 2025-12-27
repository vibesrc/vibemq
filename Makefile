# VibeMQ Makefile
# Build, test, and run conformance tests

.PHONY: all build build-release test test-unit test-integration test-conformance \
        conformance conformance-v3 conformance-v5 conformance-ci \
        broker-start broker-stop run clean help install-conformance

# Configuration
BROKER_ADDR ?= localhost
BROKER_PORT ?= 1883
CONFORMANCE_VERSION := 0.1.0
CONFORMANCE_URL := https://github.com/vibesrc/mqttconformance/releases/download/v$(CONFORMANCE_VERSION)/mqtt-conformance-x86_64-unknown-linux-musl.tar.gz
CONFORMANCE_BIN := ./bin/mqtt-conformance

# Default target
all: build test

# Build targets
build:
	cargo build

build-release:
	cargo build --release

# Test targets (cargo tests)
test: test-unit test-integration test-conformance

test-unit:
	cargo test --lib

test-integration:
	cargo test --test integration

test-conformance:
	cargo test --test conformance

# Install mqtt-conformance binary
install-conformance: $(CONFORMANCE_BIN)

$(CONFORMANCE_BIN):
	@mkdir -p ./bin
	@echo "Downloading mqtt-conformance $(CONFORMANCE_VERSION)..."
	@curl -sL $(CONFORMANCE_URL) | tar -xz -C ./bin
	@chmod +x $(CONFORMANCE_BIN)
	@echo "mqtt-conformance installed to $(CONFORMANCE_BIN)"

# External conformance tests using mqtt-conformance
# These require a running broker at BROKER_ADDR:BROKER_PORT
conformance: conformance-v3 conformance-v5

conformance-v3: $(CONFORMANCE_BIN)
	@echo "Running MQTT v3.1.1 conformance tests..."
	$(CONFORMANCE_BIN) run -H $(BROKER_ADDR) -p $(BROKER_PORT) -v 3 --parallel

conformance-v5: $(CONFORMANCE_BIN)
	@echo "Running MQTT v5.0 conformance tests..."
	$(CONFORMANCE_BIN) run -H $(BROKER_ADDR) -p $(BROKER_PORT) -v 5 --parallel

# Run specific section (usage: make conformance-section SECTION="connect" VERSION=3)
conformance-section: $(CONFORMANCE_BIN)
	$(CONFORMANCE_BIN) run -H $(BROKER_ADDR) -p $(BROKER_PORT) -v $(VERSION) --section $(SECTION) --parallel

# List available tests
conformance-list: $(CONFORMANCE_BIN)
	$(CONFORMANCE_BIN) list -v 3
	$(CONFORMANCE_BIN) list -v 5

# CI targets for running conformance tests
conformance-ci: build-release $(CONFORMANCE_BIN) broker-start
	@sleep 1
	@$(MAKE) conformance BROKER_ADDR=127.0.0.1 BROKER_PORT=1883 || ($(MAKE) broker-stop && exit 1)
	@$(MAKE) broker-stop

broker-start:
	@echo "Starting broker..."
	@./target/release/vibemq -b 127.0.0.1:1883 & echo $$! > /tmp/vibemq.pid
	@sleep 2
	@echo "Broker started (PID: $$(cat /tmp/vibemq.pid))"

broker-stop:
	@echo "Stopping broker..."
	-@kill $$(cat /tmp/vibemq.pid 2>/dev/null) 2>/dev/null || true
	-@rm -f /tmp/vibemq.pid
	@echo "Broker stopped"

# Run the broker
run:
	cargo run -- -b 0.0.0.0:1883

run-release:
	cargo run --release -- -b 0.0.0.0:1883

# Clean
clean:
	cargo clean
	rm -rf ./bin

# Help
help:
	@echo "VibeMQ Makefile"
	@echo ""
	@echo "Build targets:"
	@echo "  build          - Debug build"
	@echo "  build-release  - Release build"
	@echo ""
	@echo "Test targets (cargo tests):"
	@echo "  test           - Run all cargo tests"
	@echo "  test-unit      - Run unit tests"
	@echo "  test-integration - Run integration tests"
	@echo "  test-conformance - Run conformance tests"
	@echo ""
	@echo "External conformance tests (requires running broker):"
	@echo "  conformance    - Run all conformance tests (v3 + v5)"
	@echo "  conformance-v3 - Run MQTT v3.1.1 tests"
	@echo "  conformance-v5 - Run MQTT v5.0 tests"
	@echo "  conformance-ci - Build, start broker, run tests, stop broker"
	@echo "  conformance-list - List all available tests"
	@echo ""
	@echo "  conformance-section VERSION=3 SECTION='connect'"
	@echo "                 - Run specific test section"
	@echo ""
	@echo "Other targets:"
	@echo "  install-conformance - Download mqtt-conformance binary"
	@echo "  run            - Run broker (debug)"
	@echo "  run-release    - Run broker (release)"
	@echo "  clean          - Clean build artifacts"
	@echo ""
	@echo "Configuration:"
	@echo "  BROKER_ADDR    - Broker address (default: localhost)"
	@echo "  BROKER_PORT    - Broker port (default: 1883)"
