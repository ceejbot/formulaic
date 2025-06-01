require "download_strategy"
require "utils/formatter"
require "utils/github"
require "system_command"

class GitHubCliDownloadStrategy < CurlDownloadStrategy
	require "utils/formatter"
	require "utils/github"
	require "system_command"

	def initialize(url, name, version, **meta)
	    super
	    # Extract owner and repo from the URL
	    # Example: https://github.com/ceejbot/formulaic/releases/download/main/formulaic-aarch64-apple-darwin.tar.gz
	    match_data = %r{^https?://github\.com/(?<owner>[^/]+)/(?<repo>[^/]+)/releases/download}.match(@url)
	    return unless match_data

	    @owner = match_data[:owner]
	    @repo = match_data[:repo]
	    @filename = File.basename(@url)
	end

	def fetch(timeout: nil)
		ohai "Downloading #{url} using GitHub CLI"
		if cached_location.exist?
		    puts "Already downloaded: #{cached_location}"
		else
			begin
			  	# Create the temporary directory
			  	temporary_path.dirname.mkpath

			  	# Use gh CLI to download the release asset
			  	system_command("gh", args: [
				   		"release", "download",
						"-R", "#{@owner}/#{@repo}",
				   		"--pattern", "#{@filename}",
						"-D", "#{temporary_path}"
				     ], print_stderr: true)
			rescue ErrorDuringExecution
        		raise GitHubCliDownloadStrategyError, "GitHub CLI download failed for: #{url}"
      		end
			cached_location.dirname.mkpath

			# Find the downloaded file in the temporary path
			downloaded_file = Dir["#{temporary_path}/*"].first

			if downloaded_file
				FileUtils.mv(downloaded_file, cached_location)
			else
				raise GitHubCliDownloadStrategyError, "Downloaded file not found in #{temporary_path}"
			end
		end

		symlink_location.dirname.mkpath
    	FileUtils.ln_s cached_location.relative_path_from(symlink_location.dirname), symlink_location, force: true
	end
end

class {{package}}Test < Formula
    desc "{{description}}"
    homepage "{{homepage}}"
    version "{{version}}"
    license "{{license}}"

{%- for asset in assets %}
    if OS.{{asset.os}}? && Hardware::CPU.{{asset.cpu}}?
        url    "{{asset.url}}", using: GitHubCliDownloadStrategy
        sha256 "{{asset.sha256}}"
    end
{%- endfor %}

    BINARY_ALIASES = {
        "aarch64-apple-darwin":     {},
        "x86_64-apple-darwin":      {},
        "x86_64-pc-windows-gnu":    {},
        "x86_64-unknown-linux-gnu": {},
    }.freeze

    def target_triple
        cpu = Hardware::CPU.arm? ? "aarch64" : "x86_64"
        os = OS.mac? ? "apple-darwin" : "unknown-linux-gnu"
        "#{cpu}-#{os}"
    end

    def install_binary_aliases!
        BINARY_ALIASES[target_triple.to_sym].each do |source, dests|
            dests.each do |dest|
            bin.install_symlink bin/source.to_s => dest
            end
        end
    end

    def install
{%- for asset in assets %}
        bin.install "{{ executable }}" if OS.{{ asset.os }}? && Hardware::CPU.{{ asset.cpu }}?
{%- endfor %}

        install_binary_aliases!
        doc_files = Dir["README.*", "readme.*", "LICENSE", "LICENSE.*", "CHANGELOG.*"]
        leftover_contents = Dir["*"] - doc_files
        pkgshare.install(*leftover_contents) unless leftover_contents.empty?
    end
end
