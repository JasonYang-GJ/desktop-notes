param(
    [Parameter(Mandatory = $true)]
    [string]$TargetRoot,
    [string]$PerlRoot,
    [switch]$Offline
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$targetFull = [System.IO.Path]::GetFullPath($TargetRoot).TrimEnd('\')
if ($targetFull -notmatch '^[A-Za-z]:\\[A-Za-z0-9._-]+$') {
    throw "TargetRoot must be a short ASCII directory directly below a fixed drive root: $targetFull"
}
if ($targetFull.StartsWith($repoRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw 'The native Release target must be outside the Unicode repository path.'
}

$vendor = & (Join-Path $PSScriptRoot 'test-production-sqlcipher-vendor.ps1') | ConvertFrom-Json
if ($vendor.status -ne 'PASS') { throw 'The production SQLCipher vendor did not verify.' }
$vendor | ConvertTo-Json -Depth 4 | Out-Host

if ([string]::IsNullOrWhiteSpace($PerlRoot)) {
    $perlCommand = Get-Command perl.exe -ErrorAction Stop
    $perlExe = $perlCommand.Source
} else {
    $perlFull = (Resolve-Path -LiteralPath $PerlRoot).Path
    $perlExe = Join-Path $perlFull 'perl\bin\perl.exe'
    $perlCBin = Join-Path $perlFull 'c\bin'
    if (-not (Test-Path -LiteralPath $perlExe -PathType Leaf)) {
        throw "Perl executable not found: $perlExe"
    }
    $env:PATH = (Split-Path -Parent $perlExe) + ';' + $perlCBin + ';' + $env:PATH
}

& $perlExe -MLocale::Maketext::Simple -MIPC::Cmd -e 1
if ($LASTEXITCODE -ne 0) {
    throw 'Perl lacks the modules required by the pinned OpenSSL source build.'
}
$env:OPENSSL_SRC_PERL = $perlExe

$arguments = @('build', '--workspace', '--release', '--locked', '--target-dir', $targetFull)
if ($Offline) { $arguments += '--offline' }
$started = Get-Date
& cargo.exe @arguments
if ($LASTEXITCODE -ne 0) { throw 'The frozen production Release build failed.' }

$binary = Join-Path $targetFull 'release\desktop-notes.exe'
if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
    throw "The Release product binary is missing: $binary"
}
[pscustomobject]@{
    status = 'PASS'
    target = 'x86_64-pc-windows-msvc'
    profile = 'release'
    locked = $true
    offline = [bool]$Offline
    targetRoot = $targetFull
    binary = $binary
    binarySha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $binary).Hash
    elapsedSeconds = [Math]::Round(((Get-Date) - $started).TotalSeconds, 3)
} | ConvertTo-Json -Depth 4
