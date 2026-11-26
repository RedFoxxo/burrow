# Installs or updates burrow on Windows, in PowerShell:
#
#   irm https://raw.githubusercontent.com/RedFoxxo/burrow/main/install.ps1 | iex
#
# Downloads burrow-windows-x64.exe (Windows on ARM runs it through its x64
# emulation), checks it against the release's SHA256SUMS, puts it in
# %LOCALAPPDATA%\Programs\burrow as burrow.exe, and adds that folder to your
# user PATH when it is missing. Running it again updates burrow.
#
# Settings (environment):
#   BURROW_VERSION          release to install, e.g. 2.0.1 (default: the latest)
#   BURROW_INSTALL_DIR      folder for the program (default: %LOCALAPPDATA%\Programs\burrow)
#   BURROW_NO_MODIFY_PATH   1 leaves your PATH alone

$ErrorActionPreference = 'Stop'
# Windows PowerShell 5.1 defaults to old TLS versions GitHub refuses.
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
# The progress bar makes Invoke-WebRequest many times slower in Windows PowerShell 5.1.
$ProgressPreference = 'SilentlyContinue'

$repo = 'RedFoxxo/burrow'
$asset = 'burrow-windows-x64.exe'
$dir = if ($env:BURROW_INSTALL_DIR) { $env:BURROW_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\burrow' }
$version = "$env:BURROW_VERSION"
$base = if (-not $version) { "https://github.com/$repo/releases/latest/download" }
        elseif ($version.StartsWith('v')) { "https://github.com/$repo/releases/download/$version" }
        else { "https://github.com/$repo/releases/download/v$version" }

function Fail($message) { throw "burrow install: $message" }

New-Item -ItemType Directory -Force -Path $dir | Out-Null
$target = Join-Path $dir 'burrow.exe'
$download = Join-Path $dir 'burrow.exe.download'
try {
  Invoke-WebRequest -UseBasicParsing -Uri "$base/$asset" -OutFile $download
  $sums = (Invoke-WebRequest -UseBasicParsing -Uri "$base/SHA256SUMS").Content
} catch {
  Remove-Item -Force -ErrorAction SilentlyContinue $download
  Fail "download from $base failed (a release just tagged may still be building): $($_.Exception.Message)"
}
if ($sums -is [byte[]]) { $sums = [Text.Encoding]::UTF8.GetString($sums) }
$expected = $null
foreach ($line in $sums -split "`n") {
  $parts = $line.Trim() -split '\s+'
  if ($parts.Count -eq 2 -and $parts[1] -eq $asset) { $expected = $parts[0] }
}
$actual = (Get-FileHash -Algorithm SHA256 -Path $download).Hash.ToLowerInvariant()
if (-not $expected -or $expected -ne $actual) {
  Remove-Item -Force $download
  Fail "$asset does not match its SHA256SUMS entry; nothing was installed"
}

# Copies an earlier update moved aside, once the sessions that ran them have ended; a copy still running stays.
Get-ChildItem -Path $dir -Filter 'burrow.exe.*.old' -ErrorAction SilentlyContinue | ForEach-Object {
  Remove-Item -Force -ErrorAction SilentlyContinue $_.FullName
}
# Windows cannot overwrite a program that runs, but can rename it: an open session keeps the old copy until it ends.
$old = $null
if (Test-Path $target) {
  $old = "burrow.exe.$([DateTimeOffset]::Now.ToUnixTimeMilliseconds()).old"
  Rename-Item -Path $target -NewName $old
}
try {
  Rename-Item -Path $download -NewName 'burrow.exe'
} catch {
  if ($old) { Rename-Item -Path (Join-Path $dir $old) -NewName 'burrow.exe' }
  Remove-Item -Force -ErrorAction SilentlyContinue $download
  Fail "cannot put the new burrow.exe in $dir; the old one stays: $($_.Exception.Message)"
}

$installed = & $target --version
if ($LASTEXITCODE -ne 0) { Fail "$target does not start" }
Write-Output "$installed installed in $dir"

# The registry value as stored: entries like %USERPROFILE%\bin stay unexpanded, and the value keeps its type.
$key = Get-Item -Path 'HKCU:\Environment'
$userPath = $key.GetValue('Path', '', 'DoNotExpandEnvironmentNames')
$entries = @("$userPath" -split ';' | Where-Object { $_ })
if ($entries | Where-Object { [Environment]::ExpandEnvironmentVariables($_).TrimEnd('\') -eq $dir.TrimEnd('\') }) { return }
if ($env:BURROW_NO_MODIFY_PATH -eq '1') {
  Write-Output "$dir is not on your PATH: add it, or give clients the full path $target"
  return
}
Set-ItemProperty -Path 'HKCU:\Environment' -Name 'Path' -Type ExpandString -Value (($entries + $dir) -join ';')
# Tells running programs (Explorer among them) that the environment changed, so new terminals see it.
[Environment]::SetEnvironmentVariable('BURROW_INSTALL_NOTIFY', '1', 'User')
[Environment]::SetEnvironmentVariable('BURROW_INSTALL_NOTIFY', $null, 'User')
$env:Path = "$env:Path;$dir"
Write-Output "Added $dir to your user PATH. Open a new terminal, and restart programs that should find burrow."
