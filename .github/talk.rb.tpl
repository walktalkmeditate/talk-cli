class Talk < Formula
  desc "Terminal listening companion — speak a reflection, and it settles into a quiet file"
  homepage "https://github.com/walktalkmeditate/talk-cli"
  version "${VERSION}"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/walktalkmeditate/talk-cli/releases/download/${TAG}/talk-aarch64-apple-darwin.tar.gz"
      sha256 "${SHA256_MAC_ARM}"
    end
    on_intel do
      url "https://github.com/walktalkmeditate/talk-cli/releases/download/${TAG}/talk-x86_64-apple-darwin.tar.gz"
      sha256 "${SHA256_MAC_X86}"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/walktalkmeditate/talk-cli/releases/download/${TAG}/talk-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "${SHA256_LINUX_X86}"
    end
  end

  def install
    bin.install "talk"
  end

  def caveats
    "talk fetches its speech models on first run: run `talk download models` (~330 MB)."
  end

  test do
    assert_match "talk", shell_output("#{bin}/talk --version")
  end
end
