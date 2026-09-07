param()

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$inputs = @(
    '.cargo/config.toml'
    'Cargo.lock'
    'Cargo.toml'
    'apps/desktop/src-tauri/tests/production_dependency_migration.rs'
    'crates/desktop-notes-infra/src/sqlcipher.rs'
    'crates/desktop-notes-infra/tests/sqlcipher_foundation.rs'
    'scripts/new-production-sqlcipher-vendor.ps1'
    'scripts/test-production-sqlcipher-vendor.ps1'
    'scripts/run-production-dependency-build.ps1'
    'third_party/libsqlite3-sys-0.38.2-sqlcipher-4.18.0/DESKTOP-NOTES-PROVENANCE.json'
    'third_party/libsqlite3-sys-0.38.2-sqlcipher-4.18.0/DESKTOP-NOTES-SOURCE-MANIFEST.sha256'
)

$entries = foreach ($relative in $inputs | Sort-Object) {
    $path = Join-Path $repoRoot $relative
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Missing baseline input: $relative" }
    [pscustomobject]@{
        path = $relative.Replace('\', '/')
        sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $path).Hash
    }
}
$canonical = ($entries | ForEach-Object { "$($_.path)`n$($_.sha256)`n" }) -join ''
$bytes = [System.Text.UTF8Encoding]::new($false).GetBytes($canonical)
$sha = [System.Security.Cryptography.SHA256]::HashData($bytes)
$aggregate = -join ($sha | ForEach-Object { $_.ToString('X2') })

[pscustomobject]@{
    schemaVersion = 1
    algorithm = 'SHA-256(path LF sha256 LF, UTF-8, ordinal path order)'
    productionBaselineSha256 = $aggregate
    inputs = @($entries)
} | ConvertTo-Json -Depth 5
