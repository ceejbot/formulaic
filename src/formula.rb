class {{package}} < Formula
    desc "{{description}}"
    homepage "{{homepage}}"
    version "{{version}}"
    license "{{license}}"

{%- for asset in assets %}
    if OS.{{asset.os}}? && Hardware::CPU.{{asset.cpu}}?
        url    "{{asset.url}}"
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
