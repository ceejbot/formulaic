require "download_strategy"
require "utils/formatter"
require "utils/github"
require "system_command"

class GitHubCliDownloadStrategy < CurlDownloadStrategy
  def initialize(url, name, version, **meta)
    super
    # Extract owner and repo from the URL, e.g.
    # https://github.com/ceejbot/formulaic/releases/download/main/formulaic-aarch64-apple-darwin.tar.gz
    match_data = %r{^https?://github\.com/(?<owner>[^/]+)/(?<repo>[^/]+)/releases/download/(?<tag>[^/]+)/}.match(@url)
    return unless match_data

    @owner = match_data[:owner]
    @repo = match_data[:repo]
    @tag = match_data[:tag]
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

        # Use the gh CLI to download the release asset. Resolve gh from PATH,
        # falling back to the Homebrew prefix, so this works on Apple Silicon,
        # Intel macs, and Linuxbrew alike.
        gh = which("gh") || "#{HOMEBREW_PREFIX}/bin/gh"
        system_command(gh, args: [
          "release", "download", @tag,
          "-R", "#{@owner}/#{@repo}",
          "--pattern", @filename,
          "-D", temporary_path.to_s
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
