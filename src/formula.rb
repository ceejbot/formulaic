class {{package}} < Formula
    desc "{{description}}"
    homepage "{{homepage}}"
    version "{{version}}"
    license "{{license}}"

{%- for asset in assets %}
    if OS.{{asset.os}}? && Hardware::CPU.{{asset.cpu}}?
        url    "{{asset.url}}"{{using_strategy}}
        sha256 "{{asset.sha256}}"
    end
{%- endfor %}

    def install
{%- for asset in assets %}
        if OS.{{ asset.os }}? && Hardware::CPU.{{ asset.cpu }}?
{%- for exe in executables %}
            bin.install {{ exe }}
{%- endfor %}
        end
{%- endfor %}

        doc_files = Dir["README.*", "readme.*", "LICENSE", "LICENSE.*", "CHANGELOG.*"]
        leftover_contents = Dir["*"] - doc_files
        pkgshare.install(*leftover_contents) unless leftover_contents.empty?
    end
{%- if caveats %}

    def caveats
        <<~EOS
            {{ caveats }}
        EOS
    end
{%- endif %}
end
