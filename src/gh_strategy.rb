require "download_strategy"

class GitHubCliDownloadStrategy < GitHubArtifactDownloadStrategy
	require "utils/formatter"
	require "utils/github"
	require "system_command"

	sig { params(url: String, name: String, version: T.nilable(Version), meta: T.untyped).void }
	def initialize(url, name, version, **meta)
	    super
	    match_data = %r{^https?://github\.com/(?<org>[^/]+)/(?<repo>[^/]+)\.git$}.match(@url)
	    return unless match_data

	    @org = T.let(match_data[:user], T.nilable(String))
	    @repo = T.let(match_data[:repo], T.nilable(String))
	end

	def gh_command
		"gh release download -R #{@org}/#{@repo} --pattern #{@filepath} -O #{temporary_path}"
	end

	sig { override.params(timeout: T.any(Float, Integer, NilClass)).void }
	def fetch(timeout: nil)
		ohai "Downloading #{url}"
		if cached_location.exist?
		    puts "Already downloaded: #{cached_location}"
		else
			begin
			  	stdout, _, status = system_command("gh", args: [
				   		"release", "download",
						 "-R", "#{@org}/#{@repo}",
				   		"--pattern", "#{@filepath}",
						"-O", "#{temporary_path}/#{resolved_basename}"
				     ], print_stderr: false)
			rescue ErrorDuringExecution
        		raise GitHubCliDownloadStrategy, url
      		end
			cached_location.dirname.mkpath
		   	temporary_path.rename(cached_location.to_s)
		end

		symlink_location.dirname.mkpath
    	FileUtils.ln_s cached_location.relative_path_from(symlink_location.dirname), symlink_location, force: true
	end

	sig { returns(String) }
	def resolved_basename
		"artifact.tgz"
	end
end

class {{package}}Test < Formula
    desc "{{description}}"
    homepage "{{homepage}}"
    version "{{version}}"
    license "{{license}}"

{%- for asset in assets %}
    if OS.{{asset.os}}? && Hardware::CPU.{{asset.cpu}}?
        url    "{{asset.url}}", :using => GitHubCliDownloadStrategy
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
