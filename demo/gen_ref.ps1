$ErrorActionPreference = 'Stop'
$src = @'
using System;
using System.Collections.Generic;
using System.IO;
using System.Runtime.Serialization;
using System.Runtime.Serialization.Formatters.Binary;
using System.Text;

[Serializable]
public class MiniDoc : ISerializable
{
    public string MapName;
    public int ProjectionOrigin;
    public List<MiniItem> Items;
    public MiniDoc() { }
    protected MiniDoc(SerializationInfo info, StreamingContext ctx)
    {
        MapName = info.GetString("mapName");
        ProjectionOrigin = info.GetInt32("projectionOrigin");
        Items = (List<MiniItem>)info.GetValue("items", typeof(List<MiniItem>));
    }
    void ISerializable.GetObjectData(SerializationInfo info, StreamingContext ctx)
    {
        info.AddValue("mapName", MapName);
        info.AddValue("projectionOrigin", ProjectionOrigin);
        info.AddValue("items", Items, typeof(List<MiniItem>));
    }
}

[Serializable]
public class MiniItem
{
    public double Longitude;
    public double Latitude;
    public int Index;
}

public static class Gen
{
    public static void Run(string outPath)
    {
        var d = new MiniDoc { MapName = "QuickRoute.BusinessEntities.TestMap", ProjectionOrigin = 42,
                              Items = new List<MiniItem>() };
        for (int i = 0; i < 3; i++)
            d.Items.Add(new MiniItem { Longitude = 113.0 + i * 0.001, Latitude = 23.1 + i * 0.001, Index = i });
        var fs = new FileStream(outPath, FileMode.Create);
        var bf = new BinaryFormatter();
        bf.Serialize(fs, d);
        fs.Close();
    }
}
'@
Add-Type -TypeDefinition $src -Language CSharp
$out = Join-Path $env:TEMP 'mini.qrt'
[Gen]::Run($out)
"wrote $out : $((Get-Item $out).Length) bytes"

$b = [System.IO.File]::ReadAllBytes($out)
$sb = New-Object System.Text.StringBuilder
for ($i = 0; $i -lt $b.Length; $i++) {
    [void]$sb.Append(("{0:X2} " -f $b[$i]))
    if (($i + 1) % 24 -eq 0) { [void]$sb.AppendLine() }
}
[void]$sb.AppendLine()
[void]$sb.AppendLine('--- as chars ---')
for ($i = 0; $i -lt $b.Length; $i++) {
    $c = $b[$i]
    [void]$sb.Append($(if ($c -ge 32 -and $c -lt 127) { [char]$c } else { '.' }))
}
$sb.ToString() | Out-File -Encoding utf8 (Join-Path $env:TEMP 'mini_hex.txt')
Get-Content (Join-Path $env:TEMP 'mini_hex.txt') -Encoding utf8 | Select-Object -First 30
