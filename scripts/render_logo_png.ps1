# Render the logo SVGs (FAST-HDF, FAST-MAP, FAST-DSS and the Native Rust Engines family) to PNG with headless Microsoft Edge.
#
# Usage (from the repository root):
#   python scripts/generate_logos.py
#   pwsh scripts/render_logo_png.ps1
#
# The SVGs name Arial and Consolas, so render on Windows to get the intended fonts.

$ErrorActionPreference = 'Stop'
$here = Join-Path $PSScriptRoot '..\docs\images\logo' | Resolve-Path
$edge = @(
    "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe",
    "$env:ProgramFiles\Microsoft\Edge\Application\msedge.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $edge) { throw 'Microsoft Edge not found.' }

# svg, png, width, height (PNG pixels)
# The logos render at 4x their viewBox (the family logos are 450 x 320).
$stems = 'ras-commander-fast-hdf', 'ras-commander-fast-map', 'ras-commander-fast-dss',
         'ras-commander-rust-engines', 'ras-commander-rust-engines-dss'
$jobs = foreach ($stem in $stems) {
    $vb = [regex]::Match((Get-Content -Raw (Join-Path $here "${stem}_logo.svg")), 'viewBox="0 0 (\d+) (\d+)"')
    ,@("${stem}_logo.svg",          "${stem}_logo.png",          (4 * [int]$vb.Groups[1].Value), (4 * [int]$vb.Groups[2].Value))
    ,@("${stem}_social.svg",        "${stem}_social.png",        1200, 627)
    ,@("${stem}_github-social.svg", "${stem}_github-social.png", 1280, 640)
}

$tmp = Join-Path ([IO.Path]::GetTempPath()) ("fasthdf-logo-" + [guid]::NewGuid())
New-Item -ItemType Directory $tmp | Out-Null
try {
    foreach ($j in $jobs) {
        $svg, $png, $w, $h = $j
        $svgText = Get-Content -Raw -Encoding utf8 (Join-Path $here $svg)
        # Inline the SVG with a fixed size so it fills the window exactly.
        $svgText = $svgText -replace '<svg ([^>]*?) width="\d+" height="\d+"', '<svg $1'
        $svgText = $svgText -replace '<svg ', "<svg width=`"$w`" height=`"$h`" "
        $html = "<!doctype html><html><head><style>html,body{margin:0;padding:0;overflow:hidden}svg{display:block}</style></head><body>$svgText</body></html>"
        $page = Join-Path $tmp ($png + '.html')
        Set-Content -Path $page -Value $html -Encoding utf8
        $out = Join-Path $here $png
        & $edge --headless=new --disable-gpu --hide-scrollbars --force-device-scale-factor=1 `
            --user-data-dir="$tmp\profile" --window-size="$w,$h" `
            --screenshot="$out" ("file:///" + ($page -replace '\\', '/')) 2>$null | Out-Null
        if (-not (Test-Path $out)) { throw "Edge did not write $out" }
        Write-Host "$png  ${w}x${h}"
    }
}
finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
