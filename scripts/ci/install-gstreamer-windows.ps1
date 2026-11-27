# Install the GStreamer MSVC runtime and development MSIs on the Windows CI
# runner, each checked against its published SHA-256, then export
# PKG_CONFIG_PATH and PATH for later steps. Called by ci.yml (Tauri build,
# windows-2022) with GST_VERSION in the environment.
$ErrorActionPreference = "Stop"

$base = "https://gstreamer.freedesktop.org/data/pkg/windows/$env:GST_VERSION/msvc"
$installers = [ordered]@{
  "gstreamer-1.0-msvc-x86_64-$env:GST_VERSION.msi"       = "0e884dde0cf868491292ae44c527c55f415c795805db5a4ff2ed39355e870da7"
  "gstreamer-1.0-devel-msvc-x86_64-$env:GST_VERSION.msi" = "7073632eec899fbf9b56336ed523ddfa0f75246e737691db77092b31c728e2fc"
}
foreach ($msi in $installers.Keys) {
  $out = Join-Path $env:RUNNER_TEMP $msi
  Invoke-WebRequest -Uri "$base/$msi" -OutFile $out
  $sum = (Get-FileHash -Algorithm SHA256 $out).Hash.ToLowerInvariant()
  if ($sum -ne $installers[$msi]) { throw "$msi sha256 $sum does not match the published sum" }
  $p = Start-Process msiexec.exe -ArgumentList "/i", "`"$out`"", "/qn", "ADDLOCAL=ALL" -Wait -PassThru
  if ($p.ExitCode -ne 0) { throw "msiexec $msi exited $($p.ExitCode)" }
}
$root = Get-ChildItem -Path "C:\gstreamer", "C:\Program Files\gstreamer" -Directory -Recurse -Depth 2 -Filter msvc_x86_64 -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $root) { throw "GStreamer msvc_x86_64 root not found" }
"PKG_CONFIG_PATH=$($root.FullName)\lib\pkgconfig" | Out-File -Append -Encoding utf8 $env:GITHUB_ENV
"$($root.FullName)\bin" | Out-File -Append -Encoding utf8 $env:GITHUB_PATH
