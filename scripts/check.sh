#!/bin/sh
# Full check of the whole app. Run by the git hooks (.githooks) and by CI; any warning fails it.
# Usage: scripts/check.sh   (from anywhere)
set -eu
cd "$(dirname "$0")/.."

step() { printf '\n==> %s\n' "$*"; }

step "Rust: format"
cargo fmt --check
step "Rust: lint"
cargo clippy --all-targets --locked -- -D warnings
step "Rust: tests"
cargo test --locked
step "Rust: docs"
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --document-private-items --locked
step "Rust: release library"
RUSTFLAGS="-D warnings" cargo build --release --locked

step "Swift: tests"
(cd mac && swift test -Xswiftc -warnings-as-errors)
step "App bundle"
mac/build-app.sh
codesign --verify --strict mac/build/SoundMixer.app

printf '\nAll checks passed.\n'
