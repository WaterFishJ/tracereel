$ErrorActionPreference = 'Stop'
$src = @'
using System;
using System.IO;
using System.Runtime.Serialization.Formatters.Binary;
using System.Text;

// Two plain [Serializable] classes with FIELDS (not ISerializable), one nested
// reference -- this mirrors Map/Handle which QuickRoute serializes by field.
[Serializable]
public class Outer
{
    public string Name;
    public Inner Child;
}

[Serializable]
public class Inner
{
    public int A;
    public int B;
    public int C;
}

public static class Gen2
{
    public static void Run(string outPath)
    {
        var o = new Outer { Name = "hello", Child = new Inner { A = 1, B = 2, C = 3 } };
        var fs = new FileStream(outPath, FileMode.Create);
        new BinaryFormatter().Serialize(fs, o);
        fs.Close();
    }
}
'@
Add-Type -TypeDefinition $src -Language CSharp
$out = Join-Path $env:TEMP 'outer.bin'
[Gen2]::Run($out)
"wrote $out : $((Get-Item $out).Length) bytes"

$b = [System.IO.File]::ReadAllBytes($out)
$sb = New-Object System.Text.StringBuilder
for ($i = 0; $i -lt $b.Length; $i++) { [void]$sb.Append(("{0:X2} " -f $b[$i])) }
[void]$sb.AppendLine()
[void]$sb.AppendLine('--- chars ---')
for ($i = 0; $i -lt $b.Length; $i++) {
    $c = $b[$i]
    [void]$sb.Append($(if ($c -ge 32 -and $c -lt 127) { [char]$c } else { '.' }))
}
$sb.ToString() | Out-File -Encoding utf8 (Join-Path $env:TEMP 'outer_hex.txt')
Get-Content (Join-Path $env:TEMP 'outer_hex.txt') -Encoding utf8
