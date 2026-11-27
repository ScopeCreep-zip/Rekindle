# Install the compiler and its plugins from the official Windows release.
# https://github.com/capnproto/capnproto/releases/tag/v1.3.0
# The distribution archive's SHA-256 matches the published package manifest.
# https://community.chocolatey.org/packages/capnproto/1.3.0#files
$ErrorActionPreference = "Stop"

$version = "1.3.0"
$archive = Join-Path $env:RUNNER_TEMP "capnproto-c++-win32-$version.zip"
$destination = Join-Path $env:RUNNER_TEMP "rekindle-capnproto-$version"
$expected = "2c503361f8bf26fa9e7caccb6db04d6b271d5f0ad3da0616cf40e9a51335c89c"

Invoke-WebRequest -Uri "https://capnproto.org/capnproto-c++-win32-$version.zip" -OutFile $archive -TimeoutSec 60 -MaximumRetryCount 2 -RetryIntervalSec 2
$actual = (Get-FileHash -Path $archive -Algorithm SHA256).Hash
if ($actual -ne $expected) { throw "Unexpected Cap'n Proto archive SHA-256: $actual" }

Expand-Archive -LiteralPath $archive -DestinationPath $destination -Force
$bin = Join-Path $destination "capnproto-tools-win32-$version"
$compiler = Join-Path $bin "capnp.exe"
& $compiler --version
if ($LASTEXITCODE -ne 0) { throw "Cap'n Proto preflight exited $LASTEXITCODE" }

$bin | Out-File -Append -Encoding utf8 $env:GITHUB_PATH
