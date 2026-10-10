param(
    [Parameter(Mandatory = $true)][string]$JsonPath,
    [int]$MaxHandles = 6
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
$zeroRunStart = -1
for ($i = 0; $i -lt $all.Count; $i++) {
    $lo = P $all[$i].Groups[1].Value
    $la = P $all[$i].Groups[2].Value
    if ($lo -eq 0.0 -and $la -eq 0.0) { if ($zeroRunStart -lt 0) { $zeroRunStart = $i }; continue }
    $lonL.Add($lo); $latL.Add($la)
}
$lon = $lonL.ToArray(); $lat = $latL.ToArray()
$wms = $all
Write-Output "waypoints parsed: $($all.Count) real=$($lon.Count) firstZeroAt=$zeroRunStart  projOrigin=($projLon, $projLat)"

# cumulative haversine distance in metres
$R = 6378200.0
$cum = New-Object 'double[]' $lon.Count
for ($i = 1; $i -lt $lon.Count; $i++) {
    $p1 = $lat[$i-1] * [Math]::PI / 180.0
    $p2 = $lat[$i] * [Math]::PI / 180.0
    $dp = ($lat[$i] - $lat[$i-1]) * [Math]::PI / 180.0
    $dl = ($lon[$i] - $lon[$i-1]) * [Math]::PI / 180.0
    $a = [Math]::Sin($dp/2) * [Math]::Sin($dp/2) + [Math]::Cos($p1) * [Math]::Cos($p2) * [Math]::Sin($dl/2) * [Math]::Sin($dl/2)
    $d = 2.0 * $R * [Math]::Atan2([Math]::Sqrt($a), [Math]::Sqrt(1.0 - $a))
    $cum[$i] = $cum[$i-1] + $d
}
Write-Output ("total route length (haversine) = {0:N1} m" -f $cum[$lon.Count-1])

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

$hSeg = $t.Substring($t.IndexOf('"handles":['))
$hms = [regex]::Matches($hSeg, '\{"x":([-0-9.eE+]+),"y":([-0-9.eE+]+),"segmentIndex":(-?\d+),"value":([-0-9.eE+]+),"type":(-?\d+),"matrix":\{"m":3,"n":3,"a":\[\[([-0-9.eE+]+),([-0-9.eE+]+),([-0-9.eE+]+)\],\[([-0-9.eE+]+),([-0-9.eE+]+),([-0-9.eE+]+)\],\[0,0,1\]\]\}\}')
Write-Output "handles parsed: $($hms.Count)"

$im = [regex]::Match($t, '"initialMatrices":\[\{"m":3,"n":3,"a":\[\[([-0-9.eE+]+),([-0-9.eE+]+),([-0-9.eE+]+)\],\[([-0-9.eE+]+),([-0-9.eE+]+),([-0-9.eE+]+)\],\[0,0,1\]\]\}')
$T0 = @( (P $im.Groups[1].Value), (P $im.Groups[2].Value), (P $im.Groups[3].Value),
          (P $im.Groups[4].Value), (P $im.Groups[5].Value), (P $im.Groups[6].Value) )
Write-Output ("initialMatrix a={0} b={1} tx={2} / b={3} m11={4} ty={5}" -f $T0[0],$T0[1],$T0[2],$T0[3],$T0[4],$T0[5])

function ProjAt([double]$plValue) {
    # value is a fractional waypoint index inside the segment
    $idx = [int][Math]::Floor($plValue)
    $frac = $plValue - $idx
    if ($idx -lt 0) { $idx = 0; $frac = 0.0 }
    if ($idx -ge $lon.Count - 1) { $idx = $lon.Count - 2; $frac = 1.0 }
    $p0 = [double[]](Project $lon[$idx] $lat[$idx])
    $p1 = [double[]](Project $lon[$idx+1] $lat[$idx+1])
    return [double[]]@( ($p0[0] + $frac * ($p1[0] - $p0[0])), ($p0[1] + $frac * ($p1[1] - $p0[1])) )
}

Write-Output ""
Write-Output ("{0,-4} {1,-9} {2,-9} {3,-9} {4,-24} {5,-24}" -f 'idx','plValue','wpIdx','scale','A: M_k*proj(v_k)','handle.Location')
for ($k = 0; $k -lt [Math]::Min($MaxHandles, $hms.Count); $k++) {
    $g = $hms[$k].Groups
    $hx = P $g[1].Value
    $hy = P $g[2].Value
    $hv = P $g[4].Value
    $M = @( (P $g[6].Value), (P $g[7].Value), (P $g[8].Value), (P $g[9].Value), (P $g[10].Value), (P $g[11].Value) )

    $p = ProjAt $hv
    $mx = $M[0]*$p[0] + $M[1]*$p[1] + $M[2]
    $my = $M[3]*$p[0] + $M[4]*$p[1] + $M[5]
    $scale = [Math]::Sqrt($M[0]*$M[0] + $M[1]*$M[1])
    $err = [Math]::Sqrt(($mx-$hx)*($mx-$hx) + ($my-$hy)*($my-$hy))

    Write-Output ("{0,-4} {1,-9:N3} {2,-9:N3} {3,-9:N4} ({4,10:N1},{5,10:N1}) ({6,10:N1},{7,10:N1})  err={8:N2}px" -f $k, $hv, [Math]::Floor($hv), $scale, $mx, $my, $hx, $hy, $err)
}

# Pair invariant: in sorted order, M_i must map both p_i->L_i and p_{i+1}->L_{i+1}.
$order = 0..($hms.Count-1) | Sort-Object { P $hms[$_].Groups[4].Value }
Write-Output ""
Write-Output "pair invariant (sorted by parameterized location), first 10:"
for ($j = 0; $j -lt 10; $j++) {
    $ka = $order[$j]; $kb = $order[$j+1]
    $ga = $hms[$ka].Groups; $gb = $hms[$kb].Groups
    $va = P $ga[4].Value; $vb = P $gb[4].Value
    $Ma = @( (P $ga[6].Value), (P $ga[7].Value), (P $ga[8].Value), (P $ga[9].Value), (P $ga[10].Value), (P $ga[11].Value) )
    $pa = ProjAt $va; $pb = ProjAt $vb
    $ax = $Ma[0]*$pa[0] + $Ma[1]*$pa[1] + $Ma[2]; $ay = $Ma[3]*$pa[0] + $Ma[4]*$pa[1] + $Ma[5]
    $bx = $Ma[0]*$pb[0] + $Ma[1]*$pb[1] + $Ma[2]; $by = $Ma[3]*$pb[0] + $Ma[4]*$pb[1] + $Ma[5]
    $ea = [Math]::Sqrt(($ax-(P $ga[1].Value))*($ax-(P $ga[1].Value)) + ($ay-(P $ga[2].Value))*($ay-(P $ga[2].Value)))
    $eb = [Math]::Sqrt(($bx-(P $gb[1].Value))*($bx-(P $gb[1].Value)) + ($by-(P $gb[2].Value))*($by-(P $gb[2].Value)))
    Write-Output ("  sorted[{0}]->[{1}]  v={2:N3}  errAtSelf={3:N2}px  errAtNext={4:N2}px" -f $j, ($j+1), $va, $ea, $eb)
}
