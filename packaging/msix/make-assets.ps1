param(
    [string]$Source = '',
    [string]$Destination = ''
)

# Generate the MSIX image assets from the single approved brand raster.
# Sizes follow the Microsoft Store asset requirements; scale-200 variants are
# included because AppxManifest.xml declares Scale 100 and 200.
$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
if (-not $Source) { $Source = Join-Path $Root 'assets/branding/tracereel-icon.png' }
if (-not $Destination) { $Destination = Join-Path $PSScriptRoot 'Assets' }
if (-not (Test-Path -LiteralPath $Source)) { throw "Missing brand image: $Source" }

Add-Type -AssemblyName System.Drawing

function Save-Resized {
    param(
        [System.Drawing.Image]$Image,
        [int]$Width,
        [int]$Height,
        [string]$Path,
        [System.Drawing.Color]$Background
    )
    $bitmap = New-Object System.Drawing.Bitmap $Width, $Height
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
        $graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
        $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
        $graphics.Clear($Background)
        $scale = [Math]::Min($Width / $Image.Width, $Height / $Image.Height)
        $drawWidth = [int][Math]::Round($Image.Width * $scale)
        $drawHeight = [int][Math]::Round($Image.Height * $scale)
        $x = [int](($Width - $drawWidth) / 2)
        $y = [int](($Height - $drawHeight) / 2)
        $graphics.DrawImage($Image, $x, $y, $drawWidth, $drawHeight)
    } finally {
        $graphics.Dispose()
    }
    try {
        $bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    } finally {
        $bitmap.Dispose()
    }
}

$brand = [System.Drawing.Color]::FromArgb(255, 255, 120, 0)
$transparent = [System.Drawing.Color]::Transparent
New-Item -ItemType Directory -Force -Path $Destination | Out-Null
$image = [System.Drawing.Image]::FromFile($Source)
try {
    $assets = @(
        @('StoreLogo.png', 50, 50, $transparent),
        @('StoreLogo.scale-200.png', 100, 100, $transparent),
        @('Square44x44Logo.png', 44, 44, $transparent),
        @('Square44x44Logo.scale-200.png', 88, 88, $transparent),
        @('Square71x71Logo.png', 71, 71, $transparent),
        @('Square71x71Logo.scale-200.png', 142, 142, $transparent),
        @('Square150x150Logo.png', 150, 150, $transparent),
        @('Square150x150Logo.scale-200.png', 300, 300, $transparent),
        @('Square310x310Logo.png', 310, 310, $transparent),
        @('Square310x310Logo.scale-200.png', 620, 620, $transparent),
        @('Wide310x150Logo.png', 310, 150, $brand),
        @('Wide310x150Logo.scale-200.png', 620, 300, $brand)
    )
    foreach ($asset in $assets) {
        $path = Join-Path $Destination $asset[0]
        Save-Resized -Image $image -Width $asset[1] -Height $asset[2] -Path $path -Background $asset[3]
        Write-Host "  $($asset[0]) = $($asset[1])x$($asset[2])"
    }
} finally {
    $image.Dispose()
}
Write-Host "MSIX assets written to $Destination"
