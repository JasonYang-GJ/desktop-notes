param(
    [string]$VendorRoot
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
if ([string]::IsNullOrWhiteSpace($VendorRoot)) {
    $VendorRoot = Join-Path $repoRoot 'third_party\libsqlite3-sys-0.38.2-sqlcipher-4.18.0'
}
$vendorFull = (Resolve-Path -LiteralPath $VendorRoot).Path
$manifestPath = Join-Path $vendorFull 'DESKTOP-NOTES-SOURCE-MANIFEST.sha256'
$provenancePath = Join-Path $vendorFull 'DESKTOP-NOTES-PROVENANCE.json'

$failures = [System.Collections.Generic.List[string]]::new()
foreach ($line in Get-Content -LiteralPath $manifestPath) {
    if ([string]::IsNullOrWhiteSpace($line)) { continue }
    if ($line -notmatch '^([A-F0-9]{64})  (.+)$') {
        $failures.Add("Malformed manifest line: $line")
        continue
    }
    $expected = $Matches[1]
    $relative = $Matches[2]
    $path = Join-Path $vendorFull $relative
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        $failures.Add("Missing: $relative")
        continue
    }
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $path).Hash
    if ($actual -ne $expected) { $failures.Add("Hash mismatch: $relative") }
}

$provenance = Get-Content -LiteralPath $provenancePath -Raw | ConvertFrom-Json
$sqlite3 = Get-Content -LiteralPath (Join-Path $vendorFull 'sqlcipher\sqlite3.c') -Raw
if ($sqlite3 -notmatch '#define SQLITE_VERSION\s+"3\.53\.4"') {
    $failures.Add('sqlite3.c does not identify SQLite 3.53.4')
}
if ($sqlite3 -notmatch '#define CIPHER_VERSION_NUMBER\s+4\.18\.0') {
    $failures.Add('sqlite3.c does not identify SQLCipher 4.18.0')
}
if ($provenance.baseCrate.archiveSha256 -ne 'F1D20BEF17F513B9B3004532233187769CD072D790971F4E4DA0E346EB6401E8') {
    $failures.Add('Base crate archive checksum is not frozen')
}
if ($provenance.sqlcipher.commit -ne '63697beb0fafcb61faa7a3e6fd267036548ab11b') {
    $failures.Add('SQLCipher commit is not frozen')
}

if ($failures.Count -gt 0) {
    $failures | ForEach-Object { Write-Error $_ }
    exit 1
}

[pscustomobject]@{
    status = 'PASS'
    vendorRoot = $vendorFull
    manifestSha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $manifestPath).Hash
    verifiedFiles = @(Get-Content -LiteralPath $manifestPath | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }).Count
    sqlcipherVersion = $provenance.sqlcipher.version
    sqliteVersion = $provenance.sqlcipher.sqliteVersion
    normalBuildRequiresNetwork = $provenance.normalBuildRequiresNetwork
} | ConvertTo-Json -Depth 4
