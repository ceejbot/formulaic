# formulaic

`formulaic` is a cli that reads a manifest for a Rust program plus GitHub release information, and generates a homebrew formula for downloading assets for it. It is intended to be run in a GitHub action that's generating the release, though it can also be run locally after a release has been created. It is yet another tool in a long series of tools that solve extremely specific problems that nobody else has.

## Usage

Create a GitHub personal access token with _read_ access to the repository you're creating formulas for. Give it _write_ access to your Homebrew tap repo if you're also using this token in a workflow that updates the tap. Export that token in the env var `GITHUB_ACCESS_TOKEN`. Then invoke the tool with the location of the `Cargo.toml` manifest for the thing whose tap you want to update.

`formulaic` writes a single file to the working directory in which it is invoked, then outputs the name of that file to `stdout`. The file is named `{executable}.rb`, for the first bin target it finds in the cargo manifest.

If you set the `--gh-cli-strategy`, the tool will generate a formula file with an embedded custom download strategy that uses the `gh` [github command-line tool](https://cli.github.com). You can use an authenticated `gh` to download release artifacts from private repos, assuming you can tap the private repo to begin with.

```text
Usage: formulaic [OPTIONS] [MANIFEST]

Arguments:
  [MANIFEST]
          path to the Cargo.toml file for the installable binary
          [default: ./Cargo.toml]

Options:
  -g, --gh-cli-strategy
          Use the `gh` cli download strategy; useful for private tap repos
  -, --local
          If you have no repo-reading API permissions, we'll use only local data
  -h, --help
          Print help (see a summary with '-h')
  -V, --version
          Print version
```

## Examples

`formulaic` self-publishes to brew in [its release workflow](./.github/workflows/release.yml). Another working example is in the [justfile](./.justfile).

## Limitations

The GitHub CLI download strategy is not an official homebrew strategy, but instead my best take on what one should be. It's undoubtedly less bomb-proof than one the official project would write.

I should probably make this iterate through all discovered bins in a manifest. I only had examples with single bin targets. I also haven't tested this at all with workspaces. The manifest-reading crate, [cargo_toml](https://lib.rs/crates/cargo_toml), should be doing a good job handling them, however.

## LICENSE

This code is licensed via [the Parity Public License.](https://paritylicense.com) This license requires people who build on top of this source code to share their work with the community, too. See the license text for details.
