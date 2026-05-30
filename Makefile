.PHONY: dev mount umount prepare publish pre-publish check clean

dev:
	mkdir -p /tmp/agentfs
	cargo run -- mount ./test-project/ /tmp/agentfs

umount:
	fusermount3 -u /tmp/agentfs

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
	@if [ -d "/tmp/agentfs" ]; then \
		echo "Cleaning up mount point..."; \
		fusermount3 -u /tmp/agentfs 2>/dev/null || true; \
	fi