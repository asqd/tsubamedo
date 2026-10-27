DESKTOP := apps/desktop
APP := target/release/bundle/macos/TsubameDo.app

.PHONY: test lint check cli install deps dev bridge app run icon release screens

# Rust tests: request files, variables, sending against local servers, the viewer, the session.
test:
	cargo test --workspace

lint:
	cargo clippy --workspace --all-targets -- -D warnings

# Rust tests, clippy and a TypeScript type check of the interface.
check: test lint deps
	cd $(DESKTOP) && npm run typecheck

# Release build of the CLI: target/release/tsubamedo.
cli:
	cargo build --release -p tsubamedo-cli

# Puts tsubamedo into ~/.cargo/bin.
install:
	cargo install --path core/tsubamedo-cli

deps:
	cd $(DESKTOP) && [ -d node_modules ] || npm install

# The app with hot reload of the interface.
dev: deps
	cd $(DESKTOP) && npx tauri dev

# Dev only: the interface in a browser against the real core, http://localhost:1430/bridge.html.
bridge: deps
	./scripts/bridge.sh

# Release build of TsubameDo.app (unsigned, ad-hoc).
app: deps
	cd $(DESKTOP) && npx tauri build --bundles app

run: app
	open $(APP)

# What a release gives for this system, with SHA-256, into build/release: on a Mac the DMGs and
# the CLI for Apple Silicon and Intel. See docs/RELEASE.md.
release: deps
	./scripts/release.sh

# Screenshots for the download page (site/assets/shots), light and dark; needs `make bridge`.
screens:
	./design/screens/make.sh

# Redraws the icon from design/make-icon.swift and regenerates the app's icon set.
icon: deps
	swift design/make-icon.swift design/icon_1024.png
	cd $(DESKTOP) && npx tauri icon ../../design/icon_1024.png -o src-tauri/icons
	rm -rf $(DESKTOP)/src-tauri/icons/android $(DESKTOP)/src-tauri/icons/ios
