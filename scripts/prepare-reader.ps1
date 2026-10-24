# Verify the vendored third-party source (hdf5-reader/hdf5-core, freestiler-core)
# against vendor/PROVENANCE.json; no downloads or machine paths. Rebuilding it from
# upstream is scripts/vendor_sources.py.
$ErrorActionPreference = 'Stop'
$vendorRoot = Join-Path (Split-Path $PSScriptRoot -Parent) 'vendor'
$manifest = Get-Content (Join-Path $vendorRoot 'PROVENANCE.json') -Raw | ConvertFrom-Json
$recorded = @{}
foreach ($entry in $manifest.files.PSObject.Properties) {
    $recorded[$entry.Name] = $true
    $actual = (Get-FileHash -LiteralPath (Join-Path $vendorRoot $entry.Name) -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $entry.Value) { throw "Vendored source mismatch: $($entry.Name)" }
}
$skip = @('target', '.git', '__pycache__')
foreach ($file in Get-ChildItem -LiteralPath $vendorRoot -Recurse -File) {
    $rel = [System.IO.Path]::GetRelativePath($vendorRoot, $file.FullName).Replace('\', '/')
    if ($rel -eq 'PROVENANCE.json' -or ($rel.Split('/') | Where-Object { $skip -contains $_ })) { continue }
    if (-not $recorded.ContainsKey($rel)) { throw "Vendored file not recorded in PROVENANCE.json: $rel" }
}
$names = [string[]]@($manifest.files.PSObject.Properties.Name)
[Array]::Sort($names, [StringComparer]::Ordinal)
$text = -join ($names | ForEach-Object { "$_ $($manifest.files.$_)`n" })
$sha = [System.Security.Cryptography.SHA256]::Create()
$digest = -join ($sha.ComputeHash([System.Text.Encoding]::UTF8.GetBytes($text)) | ForEach-Object { $_.ToString('x2') })
if ($digest -ne $manifest.tree_sha256) { throw "tree_sha256 does not match the recorded file list" }
$parts = $manifest.components.PSObject.Properties | ForEach-Object { "$($_.Name) $(if ($_.Value.tag) { $_.Value.tag } else { $_.Value.commit.Substring(0, 7) })" }
Write-Output "Verified vendor/: $($recorded.Count) files ($($parts -join ', ')), tree $($manifest.tree_sha256)"
