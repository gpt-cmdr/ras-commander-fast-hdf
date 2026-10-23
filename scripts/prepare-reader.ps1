# Verify the checked-in source-only patched reader; no downloads or machine paths.
$ErrorActionPreference = 'Stop'
$vendorRoot = Join-Path (Split-Path $PSScriptRoot -Parent) 'vendor'
$manifest = Get-Content (Join-Path $vendorRoot 'PROVENANCE.json') -Raw | ConvertFrom-Json
foreach ($entry in $manifest.files.PSObject.Properties) {
    $actual = (Get-FileHash -LiteralPath (Join-Path $vendorRoot $entry.Name) -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $entry.Value) { throw "Vendored source mismatch: $($entry.Name)" }
}
Write-Output "Verified source-only reader base $($manifest.base), patched tree $($manifest.patched_tree_sha256)"
