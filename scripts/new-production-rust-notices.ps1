param(
    [string]$OutputPath
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
if ([string]::IsNullOrWhiteSpace($OutputPath)) {
    $OutputPath = Join-Path $repoRoot 'THIRD-PARTY-NOTICES.txt'
}
$outputFull = [System.IO.Path]::GetFullPath($OutputPath)
$repoPrefix = $repoRoot.TrimEnd('\') + '\'
if (-not $outputFull.StartsWith($repoPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw 'The generated Notices file must stay inside the repository.'
}

function Get-NormalizedNoticeText {
    param([Parameter(Mandatory)][string]$Path)

    $text = [System.IO.File]::ReadAllText($Path)
    return (($text -replace "`r`n", "`n") -replace "`r", "`n").TrimEnd()
}

function Get-Utf8TextSha256 {
    param([Parameter(Mandatory)][string]$Text)

    $bytes = [System.Text.UTF8Encoding]::new($false).GetBytes($Text)
    return [System.Convert]::ToHexString([System.Security.Cryptography.SHA256]::HashData($bytes))
}

$metadata = cargo.exe metadata --locked --format-version 1 | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw 'cargo metadata failed.' }
$rootPackage = $metadata.packages | Where-Object name -eq 'desktop-notes-desktop' | Select-Object -First 1
if (-not $rootPackage) { throw 'desktop-notes-desktop package was not found.' }
$nodeById = @{}
foreach ($node in $metadata.resolve.nodes) { $nodeById[$node.id] = $node }
$packageById = @{}
foreach ($package in $metadata.packages) { $packageById[$package.id] = $package }
$seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
$queue = [Collections.Generic.Queue[string]]::new()
$queue.Enqueue($rootPackage.id)
while ($queue.Count -gt 0) {
    $id = $queue.Dequeue()
    if (-not $seen.Add($id)) { continue }
    foreach ($dependency in $nodeById[$id].deps) { $queue.Enqueue($dependency.pkg) }
}

$packages = @($seen | ForEach-Object { $packageById[$_] } | Sort-Object name, version)
$lines = [Collections.Generic.List[string]]::new()
$lines.Add('Desktop Notes - Third-Party Notices')
$lines.Add('Generated from Cargo.lock, cargo metadata --locked, and the npm production tree locked by apps/desktop/package-lock.json.')
$lines.Add('This inventory is engineering compliance evidence, not legal advice.')
$lines.Add('')
foreach ($package in $packages) {
    $lines.Add(('=' * 78))
    $lines.Add("$($package.name) $($package.version)")
    $lines.Add("License expression: $(if ($package.license) { $package.license } else { 'not declared in Cargo metadata' })")
    $lines.Add("Source: $(if ($package.source) { $package.source } else { 'repository workspace' })")
    $manifestRoot = Split-Path -Parent $package.manifest_path
    $licenseFiles = @(Get-ChildItem -LiteralPath $manifestRoot -File | Where-Object {
        $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE)(\.|-|$)'
    } | Sort-Object Name)
    if ($licenseFiles.Count -eq 0) {
        $lines.Add('Bundled notice files: none found at package root; retain the declared license expression and upstream package source.')
        $lines.Add('')
        continue
    }
    foreach ($file in $licenseFiles) {
        $licenseText = Get-NormalizedNoticeText -Path $file.FullName
        $lines.Add('')
        $lines.Add("--- $($file.Name) | UTF-8/LF SHA-256 $(Get-Utf8TextSha256 -Text $licenseText) ---")
        $lines.Add($licenseText)
    }
    $lines.Add('')
}

$desktopUiRoot = Join-Path $repoRoot 'apps\desktop'
$npmCommand = Get-Command npm.cmd -ErrorAction SilentlyContinue
if (-not $npmCommand) { $npmCommand = Get-Command npm -ErrorAction Stop }
Push-Location $desktopUiRoot
try {
    $npmPaths = @(& $npmCommand.Source ls --omit=dev --all --parseable 2>$null)
    if ($LASTEXITCODE -ne 0) { throw 'npm production dependency enumeration failed.' }
} finally {
    Pop-Location
}

$jsPackagesById = [Collections.Generic.Dictionary[string, object]]::new([StringComparer]::Ordinal)
foreach ($packageRoot in $npmPaths) {
    $packageFull = [System.IO.Path]::GetFullPath($packageRoot)
    if ($packageFull.Equals($desktopUiRoot, [StringComparison]::OrdinalIgnoreCase)) { continue }
    $manifestPath = Join-Path $packageFull 'package.json'
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
        throw "npm package manifest missing: $packageFull"
    }
    $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    if ([string]::IsNullOrWhiteSpace($manifest.name) -or [string]::IsNullOrWhiteSpace($manifest.version)) {
        throw "npm package identity missing: $manifestPath"
    }
    $id = "$($manifest.name)@$($manifest.version)"
    if (-not $jsPackagesById.ContainsKey($id)) {
        $jsPackagesById[$id] = [pscustomobject]@{
            name = [string]$manifest.name
            version = [string]$manifest.version
            license = if ($manifest.license) { [string]$manifest.license } else { 'not declared in package.json' }
            root = $packageFull
        }
    }
}
$jsPackages = @($jsPackagesById.Values | Sort-Object name, version)
$lines.Add(('=' * 78))
$lines.Add('JavaScript production dependencies')
$lines.Add('Resolved by npm from apps/desktop/package-lock.json; development-only build tools are not distributed in the application bundle.')
$lines.Add('')
foreach ($package in $jsPackages) {
    $lines.Add(('=' * 78))
    $lines.Add("$($package.name) $($package.version)")
    $lines.Add("License expression: $($package.license)")
    $lines.Add("Source: npm package locked by apps/desktop/package-lock.json")
    $licenseFiles = @(Get-ChildItem -LiteralPath $package.root -File | Where-Object {
        $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE)(\.|-|_|$)'
    } | Sort-Object Name)
    if ($licenseFiles.Count -eq 0) {
        $lines.Add('Bundled notice files: none found at package root; retain the declared license expression and locked npm package identity.')
        $lines.Add('')
        continue
    }
    foreach ($file in $licenseFiles) {
        $licenseText = Get-NormalizedNoticeText -Path $file.FullName
        $lines.Add('')
        $lines.Add("--- $($file.Name) | UTF-8/LF SHA-256 $(Get-Utf8TextSha256 -Text $licenseText) ---")
        $lines.Add($licenseText)
    }
    $lines.Add('')
}

$lines.Add(('=' * 78))
$lines.Add('SQLite 3.53.4 embedded in SQLCipher 4.18.0 Community')
$lines.Add('Source: https://www.sqlite.org/')
$lines.Add('License status: SQLite core is dedicated to the public domain.')
$lines.Add('Official copyright and public-domain notice: https://www.sqlite.org/copyright.html')
$lines.Add('')

$opensslSourcePackage = $packages | Where-Object name -eq 'openssl-src' | Select-Object -First 1
if (-not $opensslSourcePackage) { throw 'The distributed OpenSSL source package was not found.' }
$embeddedNotices = @(
    [pscustomobject]@{
        component = 'SQLCipher 4.18.0 Community embedded native source'
        path = Join-Path $repoRoot 'third_party\libsqlite3-sys-0.38.2-sqlcipher-4.18.0\sqlcipher\LICENSE'
    }
    [pscustomobject]@{
        component = 'OpenSSL 3.6.3 embedded native source'
        path = Join-Path (Split-Path -Parent $opensslSourcePackage.manifest_path) 'openssl\LICENSE.txt'
    }
)
foreach ($notice in $embeddedNotices) {
    if (-not (Test-Path -LiteralPath $notice.path -PathType Leaf)) {
        throw "Embedded native notice missing: $($notice.component)"
    }
    $embeddedNoticeText = Get-NormalizedNoticeText -Path $notice.path
    $lines.Add(('=' * 78))
    $lines.Add($notice.component)
    $lines.Add("Notice UTF-8/LF SHA-256: $(Get-Utf8TextSha256 -Text $embeddedNoticeText)")
    $lines.Add('')
    $lines.Add($embeddedNoticeText)
    $lines.Add('')
}
$noticeText = (($lines -join "`n").TrimEnd() + "`n")
$noticeText = [regex]::Replace($noticeText, '[ \t]+(?=\n)', '')
[System.IO.File]::WriteAllText($outputFull, $noticeText, [System.Text.UTF8Encoding]::new($false))

[pscustomobject]@{
    status = 'PASS'
    output = [System.IO.Path]::GetRelativePath($repoRoot, $outputFull).Replace('\', '/')
    rustPackageCount = $packages.Count
    javascriptProductionPackageCount = $jsPackages.Count
    sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $outputFull).Hash
} | ConvertTo-Json -Depth 4
