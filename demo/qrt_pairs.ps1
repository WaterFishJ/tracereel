param(
    [Parameter(Mandatory = $true)][string]$JsonPath
)

$ErrorActionPreference = 'Stop'
$ci = [System.Globalization.CultureInfo]::InvariantCulture
function P([string]$s) { return [double]::Parse($s, $ci) }

$t = [System.IO.File]::ReadAllText($JsonPath)

$m = [regex]::Match($t, '"projLon":([-0-9.eE+]+),"projLat":([-0-9.eE+]+)')
$projLon = P $m.Groups[1].Value
$projLat = P $m.Groups[2].Value

$wpSeg = $t.Substring($t.IndexOf('"waypoints":['))
$all = [regex]::Matches($wpSeg, '\{"lon":([-0-9.eE+]+),"lat":([-0-9.eE+]+),"time":(\d+),"alt":([-0-9.eE+]+),"hr":([-0-9.eE+]+)\}')
$lonL = New-Object System.Collections.Generic.List[double]
$latL = New-Object System.Collections.Generic.List[double]
for ($i = 0; $i -lt $all.Count; $i++) {
    $lo = P $all[$i].Groups[1].Value
    $la = P $all[$i].Groups[2].Value
    if ($lo -eq 0.0 -and $la -eq 0.0) { continue }
    $lonL.Add($lo); $latL.Add($la)
}
$lon = $lonL.ToArray(); $lat = $latL.ToArray()

function Project([double]$lo, [double]$la) {
    $r = 6378200.0
    $l0 = $projLon * [Math]::PI / 180.0
    $p0 = $projLat * [Math]::PI / 180.0
    $lm = $lo * [Math]::PI / 180.0
    $pm = $la * [Math]::PI / 180.0
    $x = $r * [Math]::Cos($pm) * [Math]::Sin($lm - $l0)
    $y = $r * ([Math]::Cos($p0) * [Math]::Sin($pm) - [Math]::Sin($p0) * [Math]::Cos($pm) * [Math]::Cos($lm - $l0))
    return [double[]]@( ($x), ($y) )
}

function ProjAt([double]$plValue) {
    $idx = [int][Math]::Floor($plValue)
    $frac = $plValue - $idx
    if ($idx -lt 0) { $idx = 0; $frac = 0.0 }
    if ($idx -ge $lon.Count - 1) { $idx = $lon.Count - 2; $frac = 1.0 }
    $p0 = [double[]](Project $lon[$idx] $lat[$idx])
    $p1 = [double[]](Project $lon[$idx+1] $lat[$idx+1])
    return [double[]]@( ($p0[0] + $frac * ($p1[0] - $p0[0])), ($p0[1] + $frac * ($p1[1] - $p0[1])) )
}

# angle of a vector, matching LinearAlgebraUtil.GetAngleR(PointD)
function AngleR([double]$x, [double]$y) {
    $len = [Math]::Sqrt($x*$x + $y*$y)
    if ($len -eq 0.0) { return 0.0 }
    $dp = $x / $len
    if ($dp -gt 1.0) { $dp = 1.0 } elseif ($dp -lt -1.0) { $dp = -1.0 }
    if ($y -lt 0.0) { $a = 2.0 * [Math]::PI - [Math]::Acos($dp) } else { $a = [Math]::Acos($dp) }
    if ($a -gt [Math]::PI) { $a -= 2.0 * [Math]::PI }
    return $a
}
function AngleBetween([double]$x0, [double]$y0, [double]$x1, [double]$y1) {
    $a0 = AngleR $x0 $y0
    $a1 = (AngleR $x1 $y1) + 2.0 * [Math]::PI
    $diff = ($a1 - $a0) % (2.0 * [Math]::PI)
    if ($diff -gt [Math]::PI) { $diff -= 2.0 * [Math]::PI }
    return $diff
}

$hSeg = $t.Substring($t.IndexOf('"handles":['))
$hms = [regex]::Matches($hSeg, '\{"x":([-0-9.eE+]+),"y":([-0-9.eE+]+),"segmentIndex":(-?\d+),"value":([-0-9.eE+]+),"type":(-?\d+),"matrix":\{"m":3,"n":3,"a":\[\[([-0-9.eE+]+),([-0-9.eE+]+),([-0-9.eE+]+)\],\[([-0-9.eE+]+),([-0-9.eE+]+),([-0-9.eE+]+)\],\[0,0,1\]\]\}\}')

$byKey = @{}
foreach ($hm in $hms) {
    $g = $hm.Groups
    $key = "$($g[3].Value)|$($g[4].Value)"
    if (-not $byKey.ContainsKey($key)) {
        $byKey[$key] = @{
            x = P $g[1].Value; y = P $g[2].Value
            seg = [int]$g[3].Value; val = P $g[4].Value
            type = [int]$g[5].Value
            M = @( (P $g[6].Value), (P $g[7].Value), (P $g[8].Value), (P $g[9].Value), (P $g[10].Value), (P $g[11].Value) )
        }
    }
}
$hs = $byKey.Values | Sort-Object -Property @{Expression={$_.seg}}, @{Expression={$_.val}}
Write-Output "distinct handles: $($hs.Count)  (from $($hms.Count) raw)"

$types = $hs | Group-Object type | ForEach-Object { "$($_.Name)x$($_.Count)" }
Write-Output "handle types: $($types -join ' ')"

Write-Output ""
Write-Output ("{0,-4} {1,-5} {2,-9} {3,-22} {4,-22} {5,-10}" -f 'i','seg','value','predicted a,b','file a,b','err px')
$maxAB = 0.0; $maxT = 0.0
for ($i = 0; $i -lt $hs.Count - 1; $i++) {
    $h0 = $hs[$i]; $h1 = $hs[$i + 1]
    $p0 = [double[]](ProjAt $h0.val)
    $p1 = [double[]](ProjAt $h1.val)
    $dx = $h1.x - $h0.x
    $dy = $h1.y - $h0.y
    $theta = AngleBetween ($p1[0]-$p0[0]) ($p1[1]-$p0[1]) $dx (-$dy)
    $lenQ = [Math]::Sqrt($dx*$dx + $dy*$dy)
    $lenP = [Math]::Sqrt(($p1[0]-$p0[0])*($p1[0]-$p0[0]) + ($p1[1]-$p0[1])*($p1[1]-$p0[1]))
    $s = if ($lenP -eq 0.0) { 0.0 } else { $lenQ / $lenP }
    $a = $s * [Math]::Cos($theta)
    $b = (-1.0) * $s * [Math]::Sin($theta)
    $tx = $h0.x - ($a * $p0[0] + $b * $p0[1])
    $ty = $h0.y - ($b * $p0[0] - $a * $p0[1])

    $fa = $h0.M[0]; $fb = $h0.M[1]; $ftx = $h0.M[2]; $fty = $h0.M[5]
    $eAB = [Math]::Sqrt(($a-$fa)*($a-$fa) + ($b-$fb)*($b-$fb))
    $eT  = [Math]::Sqrt(($tx-$ftx)*($tx-$ftx) + ($ty-$fty)*($ty-$fty))
    $maxAB = [Math]::Max($maxAB, $eAB); $maxT = [Math]::Max($maxT, $eT)

    if ($i -lt 12 -or $eAB -gt 1e-9 -or $eT -gt 1e-6) {
        Write-Output ("{0,-4} {1,-5} {2,-9:N3} ({3,10:N6},{4,10:N6}) ({5,10:N6},{6,10:N6}) ab={7:E2} t={8:E2}" -f $i, $h0.seg, $h0.val, $a, $b, $fa, $fb, $eAB, $eT)
    }
}
Write-Output ""
Write-Output ("max |(a,b) error| = {0:E3}" -f $maxAB)
Write-Output ("max |(tx,ty) error| = {0:E3}" -f $maxT)
