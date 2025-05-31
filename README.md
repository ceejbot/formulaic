# formulaic

`formulaic` is a cli that reads a manifest for a Rust program plus GitHub release information, and generates a homebrew formula for downloading assets for it. It is intended to be run in a GitHub action that's generating the release, though it can also be run locally after a release has been created. Yet another tool in a long series of tools that solve extremely specific problems that nobody else has.

## Usage


Here's an example of local use as part of a hand-run release workflow. This script would be run in your Rust tool repo clone.

```bash
#!/usr/bin/env bash
set -e

tag=$(git describe --tags --abbrev=0)
release_url=$(gh release create "$tag" --generate-notes)

for target in "aarch64-apple-darwin" "x86_64-apple-darwin"; do
		cargo +stable build --release --target $target
		tar czf {{ BINNAME }}-$target.tar.gz --strip-components=2  target/$target/release/{{ BINNAME }}
		gh release upload "$tag" "{{ BINNAME }}-$target.tar.gz"
		sha256sum {{ BINNAME }}-$target.tar.gz > {{ BINNAME }}-"$target".tar.gz.sha256
		gh release upload "$tag" "{{ BINNAME }}-$target.tar.gz.sha256"
done

formula_file=$(formulaic ./Cargo.toml)
mv $formula_file /path/to/tap/repo/Formula/
cd /path/to/tap/repo/ || exit
git commit Formula/$(basename $formula_file) -m "$(basename -s .rb $formula_file) release $tag"
```

The Cargo manifest in the target directory must include a repo url _or_ the tool must be running in a GitHub action so it can determine which repo it's acting on.

## LICENSE

This code is licensed via [the Parity Public License.](https://paritylicense.com) This license requires people who build on top of this source code to share their work with the community, too. See the license text for details.
