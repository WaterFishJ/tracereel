$ErrorActionPreference = 'Stop'

# Reference stream generator: mirrors QuickRoute's Document/Map shape exactly
# (ISerializable with the same field names and nested typed values), written by
# the same .NET Framework BinaryFormatter QuickRoute used.
$src = @'
using System;
using System.Collections.Generic;
using System.IO;
using System.Runtime.Serialization;
using System.Runtime.Serialization.Formatters.Binary;
using System.Text;

public enum MapSourceType { FileSystem, Url }
public enum MapStorageType { Inline, Reference }

[Serializable]
public class LongLat : ISerializable
{
    public double Longitude, Latitude;
    public LongLat() { }
    public LongLat(double lon, double lat) { Longitude = lon; Latitude = lat; }
    protected LongLat(SerializationInfo i, StreamingContext c)
    {
        Longitude = i.GetDouble("longitude");
        Latitude = i.GetDouble("latitude");
    }
    void ISerializable.GetObjectData(SerializationInfo i, StreamingContext c)
    {
        i.AddValue("longitude", Longitude);
        i.AddValue("latitude", Latitude);
    }
}

[Serializable]
public class QMap : ISerializable
{
    public string source;
    public MapSourceType sourceType;
    public MapStorageType storageType;
    public byte[] rawData;
    public QMap() { }
    protected QMap(SerializationInfo i, StreamingContext c)
    {
        source = i.GetString("source");
        sourceType = (MapSourceType)i.GetValue("sourceType", typeof(MapSourceType));
        storageType = (MapStorageType)i.GetValue("storageType", typeof(MapStorageType));
        rawData = (byte[])i.GetValue("rawData", typeof(byte[]));
    }
    void ISerializable.GetObjectData(SerializationInfo i, StreamingContext c)
    {
        i.AddValue("source", source);
        i.AddValue("sourceType", sourceType, typeof(MapSourceType));
        i.AddValue("storageType", storageType, typeof(MapStorageType));
        if (storageType == MapStorageType.Inline) i.AddValue("rawData", rawData);
    }
}

[Serializable]
public class Settings : ISerializable
{
    public int a;
    public Settings() { }
    protected Settings(SerializationInfo i, StreamingContext c) { a = i.GetInt32("a"); }
    void ISerializable.GetObjectData(SerializationInfo i, StreamingContext c)
    {
        i.AddValue("a", a);
    }
}

[Serializable]
public class Doc : ISerializable
{
    public QMap map;
    public Settings settings;
    public List<int> sessions;
    public LongLat projectionOrigin;
    public string properties;
    public Doc() { }
    protected Doc(SerializationInfo i, StreamingContext c)
    {
        map = (QMap)i.GetValue("map", typeof(QMap));
        settings = (Settings)i.GetValue("settings", typeof(Settings));
        sessions = (List<int>)i.GetValue("sessions", typeof(List<int>));
        projectionOrigin = (LongLat)i.GetValue("projectionOrigin", typeof(LongLat));
        properties = (string)i.GetValue("properties", typeof(string));
    }
    void ISerializable.GetObjectData(SerializationInfo i, StreamingContext c)
    {
        i.AddValue("map", map, typeof(QMap));
        i.AddValue("settings", settings, typeof(Settings));
        i.AddValue("sessions", sessions, typeof(List<int>));
        i.AddValue("projectionOrigin", projectionOrigin, typeof(LongLat));
        i.AddValue("properties", properties, typeof(string));
    }
}

public static class Gen
{
    public static string Run(string outPath)
    {
        var d = new Doc
        {
            map = new QMap { source = "", sourceType = MapSourceType.FileSystem,
                             storageType = MapStorageType.Inline,
                             rawData = new byte[] { 0xFF, 0xD8, 0xFF, 0x01, 0x02, 0x03 } },
            settings = new Settings { a = 42 },
            sessions = new List<int> { 1, 2, 3 },
            projectionOrigin = new LongLat(113.0243020, 23.1763540),
            properties = null
        };
        using (var fs = new FileStream(outPath, FileMode.Create))
            new BinaryFormatter().Serialize(fs, d);
        return outPath;
    }
}
'@
Add-Type -TypeDefinition $src -Language CSharp
$out = Join-Path $env:TEMP 'qrtlike.bin'
[Gen]::Run($out) | Out-Null
$len = (Get-Item $out).Length
Write-Output "wrote $out ($len bytes)"

$b = [System.IO.File]::ReadAllBytes($out)
$sb = New-Object System.Text.StringBuilder
for ($i = 0; $i -lt [Math]::Min(320, $b.Length); $i++) {
    [void]$sb.Append(("{0:X2} " -f $b[$i]))
    if (($i + 1) % 24 -eq 0) { [void]$sb.AppendLine() }
}
[void]$sb.AppendLine()
[void]$sb.AppendLine('--- chars ---')
for ($i = 0; $i -lt [Math]::Min(320, $b.Length); $i++) {
    $c = $b[$i]
    [void]$sb.Append($(if ($c -ge 32 -and $c -lt 127) { [char]$c } else { '.' }))
}
$sb.ToString() | Out-File -Encoding utf8 (Join-Path $env:TEMP 'qrtlike_hex.txt')
Get-Content (Join-Path $env:TEMP 'qrtlike_hex.txt') -Encoding utf8
