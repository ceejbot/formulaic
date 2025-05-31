BINNAME := "formulaic"
RELATIVE_TAP_PATH := "../../homebrew-tap/"

_help:
	just -l

# Run all tests using nextest.
test:
	cargo nextest run

# Run the same checks we run in CI. Requires nightly.
ci: test
	cargo clippy
	cargo +nightly fmt

# Ask for clippy's opinion.
lint:
	cargo clippy --fix
	cargo +nightly fmt

# Install required tools
setup:
	brew tap ceejbot/tap
	brew install fzf tomato semver-bump cargo-nextest
	rustup install nightly

# Tag a new version for release.
tag BUMP:
	#!/usr/bin/env bash
	set -e
	current=$(tomato get package.version Cargo.toml)
	version=$(echo "$current" | semver-bump {{BUMP}})
	tomato set package.version "$version" Cargo.toml &> /dev/null
	cargo generate-lockfile
	git commit Cargo.toml Cargo.lock -m "v${version}"
	git tag "v${version}"
	echo "Release tagged for version v${version}"

# Release formulaic by hand using the local build of itself.
release:
	#!/usr/bin/env bash
	set -e
	mkdir -p dist
	cd dist
	tag=$(git describe --tags --abbrev=0)
	release_url=$(gh release create "$tag" --generate-notes)
	for target in "aarch64-apple-darwin" "x86_64-apple-darwin"; do
		cargo +stable build --release --target $target
		tar czf {{ BINNAME }}-$target.tar.gz --strip-components=2  target/$target/release/{{ BINNAME }}
		gh release upload "$tag" "{{ BINNAME }}-$target.tar.gz"
		sha256sum {{ BINNAME }}-$target.tar.gz > {{ BINNAME }}-"$target".tar.gz.sha256
		gh release upload "$tag" "{{ BINNAME }}-$target.tar.gz.sha256"
	done
	formula_file=$(cargo run -- ../Cargo.toml)
	cd ..
	mv dist/$formula_file {{RELATIVE_TAP_PATH}}/Formula/
	cd {{RELATIVE_TAP_PATH}} || exit
	git add Formula/$(basename $formula_file)
	git commit -m "$(basename -s .rb $formula_file) $tag"
