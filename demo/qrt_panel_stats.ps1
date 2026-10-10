param(
    [Parameter(Mandatory = $true)][string]$Path
)
# Left-panel (map) statistics for a rendered frame: how much of the panel is map
# rather than background, and how much of it is the route line (#0064ff).
# `-Path a.png;b.png` (PowerShell 5.1 `-File` cannot take an array).
Add-Type -AssemblyName System.Drawing
foreach ($p in ($Path -split ';')) {
    $bmp = [System.Drawing.Bitmap]::FromFile((Resolve-Path $p))
    $leftW = [int]($bmp.Width * 0.25)
    $colors = New-Object 'System.Collections.Generic.HashSet[string]'
    $black = 0
    $route = 0
    $samples = 0
    $minX = [int]::MaxValue; $maxX = -1; $minY = [int]::MaxValue; $maxY = -1
    for ($y = 0; $y -lt $bmp.Height; $y += 2) {
        for ($x = 0; $x -lt $leftW; $x += 2) {
            $c = $bmp.GetPixel($x, $y)
            $samples++
            [void]$colors.Add("$($c.R),$($c.G),$($c.B)")
            if ($c.R -eq 0 -and $c.G -eq 0 -and $c.B -eq 0) { $black++ }
            if ($c.R -le 40 -and [Math]::Abs($c.G - 100) -le 40 -and $c.B -ge 215) {
                $route++
                if ($x -lt $minX) { $minX = $x }; if ($x -gt $maxX) { $maxX = $x }
                if ($y -lt $minY) { $minY = $y }; if ($y -gt $maxY) { $maxY = $y }
            }
        }
    }
    Write-Output ("{0,-34} colors={1,6}  black={2,4}/{3}  route={4,5} px  bbox=({5},{6})-({7},{8})" -f `
        (Split-Path $p -Leaf), $colors.Count, $black, $samples, $route, $minX, $minY, $maxX, $maxY)
    $bmp.Dispose()
}
