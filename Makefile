.PHONY: dev mount umount prepare publish pre-publish check clean npm-build

dev:
	mkdir -p /tmp/agentignore
	cargo run -- mount ./test-project/ /tmp/agentignore

umount:
	fusermount3 -u /tmp/agentignore

# Publishing preparation
prepare: check pre-publish
	@echo "✅ Ready to publish!"
	@echo "Run 'make publish' to publish to crates.io"

pre-publish:
	@echo "📦 Running pre-publish checks..."
	cargo fmt --check
	cargo clippy -- -D warnings
	cargo test
	cargo doc --no-deps
	@echo "✅ All checks passed"

check:
	@echo "🔍 Checking Cargo.toml metadata..."
	@cargo verify-project > /dev/null && echo "✅ Project valid"
	@echo "📋 Package contents:"
	cargo package --list
	@echo ""
	@echo "🧪 Dry run publish:"
	cargo publish --dry-run

publish: prepare
	@echo "🚀 Publishing to crates.io..."
	@echo "Make sure you're logged in with 'cargo login'"
	cargo publish

clean:
	cargo clean
	@if [ -d "/tmp/agentignore" ]; then \
		echo "Cleaning up mount point..."; \
		fusermount3 -u /tmp/agentignore 2>/dev/null || true; \
	fi

npm-build:
	cargo clean
	rm -rf npm/
	cargo build --release --target aarch64-unknown-linux-gnu
	cargo build --release --target x86_64-unknown-linux-gnu
	cargo npm generate
