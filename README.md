# formulaic

`formulaic` generates Homebrew formula files from GitHub release assets. It supports Rust projects via `Cargo.toml` and any project via a `formulaic.toml` manifest. It's designed to run in a GitHub Actions release workflow, but it works just fine locally too.

## Installation

```
brew tap ceejbot/tap
brew install formulaic
```

Or build from source with `cargo build --release`.

## Quick start

1. Export a GitHub token as `GITHUB_ACCESS_TOKEN` or `GITHUB_TOKEN`. It needs _read_ access to the repo you're generating formulas for. Give it _write_ access to your Homebrew tap repo if your workflow updates the tap automatically.

2. Run `formulaic` in your project directory:

```
formulaic [OPTIONS] [MANIFEST]
```

That's it! It writes `{executable}.rb` to the current directory (or wherever `--output-dir` points) and prints the path to stdout. Use `--dry-run` to preview what it would generate.

## Manifest resolution

If you don't pass a manifest path, formulaic looks in the current directory for these files, in order:

1. `.config/formulaic.toml`
2. `.formulaic.toml`
3. `formulaic.toml`
4. `Cargo.toml`

The first one it finds wins.

## Options

| Flag                 | Short | Description                                                            |
| -------------------- | ----- | ---------------------------------------------------------------------- |
| `--gh-cli-strategy`  | `-g`  | Use the `gh` CLI download strategy (for private repos)                 |
| `--local`            | `-l`  | Use local data from a `dist/` directory instead of the GitHub API      |
| `--bin <NAME>`       |       | Generate a formula for a specific binary only                          |
| `--all`              |       | Generate formulas for all binaries (default when more than one exists) |
| `--output-dir <DIR>` | `-o`  | Write formula files to this directory                                  |
| `--dry-run`          |       | Preview formulas without writing files                                 |
| `--template <FILE>`  | `-t`  | Use a custom formula template (upon/Jinja2 syntax)                     |

## `formulaic.toml`

For non-Rust projects — or if you just want a simpler manifest — create a `formulaic.toml` with your project metadata:

```toml
name = "my-tool"
version = "1.0.0"
description = "What my tool does"
homepage = "https://github.com/owner/my-tool"
license = "MIT"
repository = "https://github.com/owner/my-tool"
```

Only `name` and `version` are required. Everything else is optional.

### Multiple binaries

If your tarball contains more than one executable, use the `[bins]` table to tell formulaic where to find them. Keys are the binary names that get installed; values are the paths inside the tarball:

```toml
name = "my-project"
version = "2.0.0"
repository = "https://github.com/owner/my-project"

[bins]
tool-a = "tool-a"
tool-b = "bin/tool-b"
```

When the name and path are the same (like `tool-a` above), the formula uses a simple `bin.install "tool-a"`. When they differ, it generates `bin.install "bin/tool-b" => "tool-b"` so Homebrew extracts the binary from a subdirectory and installs it under the right name.

If you omit `[bins]`, formulaic falls back to installing a single binary named after `name`.

### Caveats

Got something important to tell users after they install? Add a `caveats` field:

```toml
caveats = "Run `my-tool init` before first use."
```

This generates a `def caveats` block in the formula, which Homebrew displays after install.

### The `gh` CLI download strategy

Setting `gh-cli-strategy = true` in the manifest is equivalent to passing `--gh-cli-strategy` on the command line — either one enables it.

```toml
gh-cli-strategy = true
```

This generates a formula with an embedded download strategy that uses the [`gh` CLI tool](https://cli.github.com) to fetch release artifacts. It lets authenticated users install from private repos, assuming they can tap the repo to begin with. This isn't an official Homebrew strategy, but making the private repo approach work is the reason I wrote this tool in the first place.

## Asset naming conventions

Formulaic expects release tarballs named `{name}-{target}.tar.gz`, where `{target}` is one of:

- `aarch64-apple-darwin` (macOS ARM)
- `x86_64-apple-darwin` (macOS Intel)
- `universal-apple-darwin` (macOS fat binary — generates both ARM and Intel entries)
- `x86_64-unknown-linux-gnu` (Linux Intel)
- `aarch64-unknown-linux-gnu` (Linux ARM)

SHA256 digests can come from a `.sha256` sidecar file, a local tarball, or — as a last resort — by downloading the asset and hashing it.

## Cargo projects

For Rust projects, formulaic reads `Cargo.toml` directly to get package metadata. If your crate has multiple `[[bin]]` targets, use `--bin <NAME>` to generate a formula for just one, or `--all` to generate formulas for every binary (one `.rb` file each). When there's more than one binary and you don't specify, `--all` is the default.

## Examples

`formulaic` uses itself to publish to Homebrew! You can see it in action in the [release workflow](./.github/workflows/release.yml). There's also a [justfile](./.justfile) with a `just release` recipe for doing it by hand locally.

## License

[The Parity Public License.](https://paritylicense.com) This license requires people who build on top of this source code to share their work with the community, too. See the license text for details.
