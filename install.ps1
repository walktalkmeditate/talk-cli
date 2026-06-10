# talk installer (Windows). Downloads the latest release, verifies its checksum,
# and installs it to %LOCALAPPDATA%\Programs\talk. talk fetches its speech models on
# first run (`talk download models`, ~330 MB) — the binary itself ships without them.
$ErrorActionPreference = "Stop"

$repo = "walktalkmeditate/talk-cli"
$bin = "talk"

$arch = switch ($env:PROCESSOR_ARCHITECTURE) {
    "AMD64" { "x86_64" }
    "ARM64" { "aarch64" }
    default { throw "talk: unsupported architecture '$($env:PROCESSOR_ARCHITECTURE)'" }
}
$target = "$arch-pc-windows-msvc"

$headers = @{}
if ($env:GITHUB_TOKEN) { $headers["Authorization"] = "Bearer $($env:GITHUB_TOKEN)" }
$release = Invoke-RestMethod -Headers $headers -Uri "https://api.github.com/repos/$repo/releases/latest"
$tag = $release.tag_name
if (-not $tag) { throw "talk: could not find the latest release" }

$base = "https://github.com/$repo/releases/download/$tag"
$archive = "$bin-$target.zip"
$tmp = New-Item -ItemType Directory -Path (Join-Path $env:TEMP ([System.Guid]::NewGuid()))

try {
    Invoke-WebRequest -Uri "$base/$archive" -OutFile (Join-Path $tmp $archive)
    Invoke-WebRequest -Uri "$base/checksums.txt" -OutFile (Join-Path $tmp "checksums.txt")

    $expected = (Get-Content (Join-Path $tmp "checksums.txt") |
        Where-Object { $_ -match [Regex]::Escape($archive) + '$' } |
        ForEach-Object { ($_ -split '\s+')[0] }) | Select-Object -First 1
    $actual = (Get-FileHash -Algorithm SHA256 (Join-Path $tmp $archive)).Hash.ToLower()
    if (-not $expected -or $expected.ToLower() -ne $actual) {
        throw "talk: checksum verification failed — aborting"
    }

    Expand-Archive -Path (Join-Path $tmp $archive) -DestinationPath $tmp -Force
    $dest = Join-Path $env:LOCALAPPDATA "Programs\talk"
    New-Item -ItemType Directory -Force -Path $dest | Out-Null
    Copy-Item -Force (Join-Path $tmp "$bin-$target\$bin.exe") (Join-Path $dest "$bin.exe")

    Write-Host "Installed $bin $tag to $dest\$bin.exe"
    Write-Host "Run 'talk download models' once (~330 MB) before your first reflection."
    if ($env:PATH -notlike "*$dest*") {
        Write-Host "Add $dest to your PATH to run 'talk'."
    }
}
finally {
    Remove-Item -Recurse -Force $tmp
}
