# Pack the portable build after `pnpm build`:
#
#   pnpm portable
#
# Writes target\release\bundle\portable\Rusty Downloader_<version>_x64_portable.zip
# with a "Rusty Downloader" folder holding the app, the browser host, the
# license and portable.flag, which keeps all data in a "data" folder beside
# them (see crates\rdl-core\src\paths.rs).

$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..\..')
$release = Join-Path $root 'target\release'

$cargo = Get-Content (Join-Path $root 'Cargo.toml') -Raw
if ($cargo -notmatch '(?m)^version = "([^"]+)"') { throw 'No version in Cargo.toml' }
$version = $Matches[1]

$files = @(
    (Join-Path $release 'rusty-downloader.exe'),
    (Join-Path $release 'rdl-native-host.exe'),
    (Join-Path $root 'LICENSE')
)
foreach ($file in $files) {
    if (-not (Test-Path $file)) { throw "Missing $file; run pnpm build first" }
}

$out = Join-Path $release 'bundle\portable'
$stage = Join-Path $out 'Rusty Downloader'
if (Test-Path $out) { Remove-Item $out -Recurse -Force }
New-Item -ItemType Directory -Force $stage | Out-Null
Copy-Item $files $stage
[IO.File]::WriteAllLines((Join-Path $stage 'portable.flag'), [string[]]@(
    'This file makes Rusty Downloader portable: settings, history and tools',
    'are kept in the "data" folder next to it instead of in %APPDATA%.'
))

$zip = Join-Path $out "Rusty Downloader_${version}_x64_portable.zip"
# Entries added one by one with "/" in their names: Windows PowerShell's
# Compress-Archive and ZipFile write backslashes, which some tools mishandle.
Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
$archive = [IO.Compression.ZipFile]::Open($zip, 'Create')
try {
    foreach ($file in Get-ChildItem $stage -File) {
        [IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
            $archive, $file.FullName, "Rusty Downloader/$($file.Name)", 'Optimal') | Out-Null
    }
} finally {
    $archive.Dispose()
}
Remove-Item $stage -Recurse -Force
Write-Host "wrote $zip"
