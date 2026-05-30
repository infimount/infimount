param(
  [string]$Version = $env:INFIMOUNT_VERSION,
  [ValidateSet("msi", "exe")]
  [string]$Installer = $(if ($env:INFIMOUNT_WINDOWS_INSTALLER) { $env:INFIMOUNT_WINDOWS_INSTALLER } else { "msi" }),
  [string]$Repo = $(if ($env:INFIMOUNT_REPO) { $env:INFIMOUNT_REPO } else { "infimount/infimount" }),
  [string]$ReleaseBaseUrl = $env:INFIMOUNT_RELEASE_BASE_URL,
  [switch]$DryRun = $([bool]$env:INFIMOUNT_INSTALL_DRY_RUN)
)

$ErrorActionPreference = "Stop"
if ([string]::IsNullOrWhiteSpace($Version)) { $Version = "latest" }

function Fail($Message) {
  Write-Error "Infimount install failed: $Message"
  exit 1
}

function Get-ReleaseBaseUrl {
  if (-not [string]::IsNullOrWhiteSpace($ReleaseBaseUrl)) {
    return $ReleaseBaseUrl
  }

  if ($Version -eq "latest") {
    return "https://github.com/$Repo/releases/latest/download"
  }

  $tag = if ($Version.StartsWith("v")) { $Version } else { "v$Version" }
  return "https://github.com/$Repo/releases/download/$tag"
}

function Invoke-Download($Url, $Destination) {
  Write-Host "Downloading $Url"
  Invoke-WebRequest -Uri $Url -OutFile $Destination -UseBasicParsing
}

function Get-AssetName {
  if ($Installer -eq "exe") { return "Infimount-setup.exe" }
  return "Infimount.msi"
}

function Test-Checksum($AssetPath, $SumsPath, $AssetName) {
  $line = Get-Content $SumsPath | Where-Object { $_ -match "\s$([regex]::Escape($AssetName))$" } | Select-Object -First 1
  if (-not $line) { Fail "checksum entry not found for $AssetName" }

  $expected = ($line -split "\s+")[0].ToLowerInvariant()
  $actual = (Get-FileHash -Algorithm SHA256 -Path $AssetPath).Hash.ToLowerInvariant()

  if ($expected -ne $actual) {
    Fail "checksum mismatch for $AssetName. Expected $expected, got $actual"
  }
}

function Install-Infimount($AssetPath, $AssetName) {
  if ($AssetName.EndsWith(".msi")) {
    $args = @("/i", $AssetPath, "/passive", "/norestart")
    $process = Start-Process -FilePath "msiexec.exe" -ArgumentList $args -Wait -PassThru
  } else {
    $process = Start-Process -FilePath $AssetPath -ArgumentList @("/S") -Wait -PassThru
  }

  if ($process.ExitCode -ne 0) {
    Fail "installer exited with code $($process.ExitCode). Try running PowerShell as Administrator."
  }
}

if (-not [Environment]::Is64BitOperatingSystem) {
  Fail "Windows release assets require 64-bit Windows."
}

$baseUrl = Get-ReleaseBaseUrl
$assetName = Get-AssetName
$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("infimount-install-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $tempDir | Out-Null

try {
  $assetPath = Join-Path $tempDir $assetName
  $sumsPath = Join-Path $tempDir "SHA256SUMS.txt"

  Write-Host "Installing Infimount from $baseUrl"
  Write-Host "Selected asset: $assetName"

  Invoke-Download "$baseUrl/SHA256SUMS.txt" $sumsPath
  Invoke-Download "$baseUrl/$assetName" $assetPath
  Test-Checksum $assetPath $sumsPath $assetName
  Write-Host "Checksum verified."

  if ($DryRun) {
    Write-Host "Dry run requested; skipping installation."
    return
  }

  Install-Infimount $assetPath $assetName
  Write-Host "Infimount installation complete."
} finally {
  Remove-Item -Recurse -Force $tempDir -ErrorAction SilentlyContinue
}
