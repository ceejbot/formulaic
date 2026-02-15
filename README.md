# formulaic

`formulaic` generates Homebrew formula files from GitHub release assets. It supports Rust projects via `Cargo.toml` and any project via a `formulaic.toml` manifest. It is intended to be run in a GitHub action that's generating a release, though it can also be run locally after a release has been created.

## Usage

Create a GitHub personal access token with _read_ access to the repository you're creating formulas for. Give it _write_ access to your Homebrew tap repo if you're also using this token in a workflow that updates the tap. Export that token as `GITHUB_ACCESS_TOKEN` or `GITHUB_TOKEN`.

```
formulaic [OPTIONS] [MANIFEST]
```

If no manifest path is given, formulaic searches the current directory in this order:

1. `.config/formulaic.toml`
2. `.formulaic.toml`
3. `formulaic.toml`
4. `Cargo.toml`

The tool writes `{executable}.rb` to the working directory (or `--output-dir`) and prints the path to stdout. Use `--dry-run` to preview without writing.

### Options

| Flag | Short | Description |
|------|-------|-------------|
| `--gh-cli-strategy` | `-g` | Use the `gh` CLI download strategy (useful for private repos) |
| `--local` | `-l` | Use only local data from a `dist/` directory instead of the GitHub API |
| `--bin <NAME>` | | Generate a formula for a specific binary only |
| `--all` | | Generate formulas for all binaries (default when more than one exists) |
| `--output-dir <DIR>` | `-o` | Write formula files to this directory |
| `--dry-run` | | Preview formulas without writing files |
| `--template <FILE>` | `-t` | Use a custom formula template (upon/Jinja2 syntax) |

## `formulaic.toml`

For non-Rust projects, create a `formulaic.toml` (or `.formulaic.toml`, or `.config/formulaic.toml`) with your project metadata:

```toml
name = "my-tool"
version = "1.0.0"
description = "What my tool does"
homepage = "https://github.com/owner/my-tool"
license = "MIT"
repository = "https://github.com/owner/my-tool"
gh-cli-strategy = true
```

Required fields: `name`, `version`. All others are optional.

Setting `gh-cli-strategy = true` in the manifest is equivalent to passing `--gh-cli-strategy` on the command line. The CLI flag and the manifest field are OR'd together — either one enables the strategy.

## The `gh` CLI download strategy

The `--gh-cli-strategy` flag generates a formula with an embedded download strategy that uses the [`gh` CLI tool](https://cli.github.com). This lets an authenticated `gh` download release artifacts from private repos, assuming you can tap the repo to begin with. This is not an official Homebrew strategy, but a best-effort implementation.

## Examples

`formulaic` self-publishes to brew in [its release workflow](./.github/workflows/release.yml). Another working example is in the [justfile](./.justfile).

## Limitations

Multi-binary support (`--bin`, `--all`) works for Cargo projects but is stubbed for generic manifests — a `formulaic.toml` project always produces a single formula. Workspace support is untested.

## License

This code is licensed via [the Parity Public License.](https://paritylicense.com) This license requires people who build on top of this source code to share their work with the community, too. See the license text for details.
