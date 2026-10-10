param(
    [Parameter(Mandatory = $true)][string]$QrtPath,
    [Parameter(Mandatory = $true)][string]$OutPath
)

$ErrorActionPreference = 'Stop'

# Self-contained QuickRoute .qrt extractor.
#
# Uses .NET's own BinaryFormatter with a capture binder: every QuickRoute /
# PowerCollections type is bound to a `Bag` stub whose serialization constructor
# records the name/value pairs, so the entire object graph comes out as data
# without needing the original assemblies.
#
# Emits JSON: { mapBase64, mapBytes, projectionOrigin, waypoints[], handles[] }
#
# Usage: powershell -File qrt_dump.ps1 <file.qrt> <out.json>

$src = @'
#pragma warning disable 0649, 0169, 0414, 0618
using System;
using System.Collections;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Runtime.Serialization;
using System.Runtime.Serialization.Formatters.Binary;
using System.Text;

[Serializable]
public class Bag : ISerializable
{
    public List<string> keys = new List<string>();
    public List<object> values = new List<object>();
    public Bag() { }
    protected Bag(SerializationInfo i, StreamingContext c)
    {
        foreach (SerializationEntry e in i) { keys.Add(e.Name); values.Add(e.Value); }
    }
    public void GetObjectData(SerializationInfo i, StreamingContext c) { }
    public object Get(string name)
    {
        for (int i = 0; i < keys.Count; i++) if (keys[i] == name) return values[i];
        return null;
    }
}

public sealed class CaptureBinder : SerializationBinder
{
    public override Type BindToType(string assemblyName, string typeName)
    {
        switch (typeName)
        {
            case "System.Byte[]": return typeof(byte[]);
            case "System.Double[]": return typeof(double[]);
            case "System.Int32[]": return typeof(int[]);
            case "System.String[]": return typeof(string[]);
            case "System.Object[]": return typeof(object[]);
            case "System.String": return typeof(string);
            case "System.Double": return typeof(double);
            case "System.Int32": return typeof(int);
            case "System.Boolean": return typeof(bool);
            case "System.DateTime": return typeof(DateTime);
            case "System.Double[][]": return typeof(double[][]);
            case "System.Object": return typeof(object);
        }
        return typeof(Bag);
    }
}

public static class QrtExtract
{
    static StringBuilder j;

    static void Num(double v)
    { j.Append(v.ToString("R", CultureInfo.InvariantCulture)); }

    static void Str(string s)
    {
        if (s == null) { j.Append("null"); return; }
        j.Append('"');
        foreach (char c in s)
        {
            if (c == '"' || c == '\\') { j.Append('\\').Append(c); }
            else if (c == '\n') j.Append("\\n");
            else if (c == '\r') j.Append("\\r");
            else if (c == '\t') j.Append("\\t");
            else if (c < 32) j.Append(' ');
            else j.Append(c);
        }
        j.Append('"');
    }

    static List<object> Items(object o)
    {
        var list = new List<object>();
        if (o == null || o is string) return list;
        Bag b = o as Bag;
        if (b != null)
        {
            object inner = b.Get("_items") ?? b.Get("items") ?? b.Get("KeyValuePairs");
            if (inner != null && !object.ReferenceEquals(inner, o)) return Items(inner);
            return list;
        }
        IEnumerable en = o as IEnumerable;
        if (en == null) return list;
        foreach (object x in en) list.Add(x);
        return list;
    }

    static object F(object o, string n)
    {
        Bag b = o as Bag;
        return b == null ? null : b.Get(n);
    }

    static double Fd(object o, string n)
    {
        object v = F(o, n);
        if (v == null) return 0.0;
        if (v is double) return (double)v;
        if (v is int) return (int)v;
        if (v is float) return (float)v;
        return 0.0;
    }

    // Enums arrive boxed as the enum type, so `is int` is false; Convert handles
    // every underlying type (including long/byte) in one go.
    static long Fi(object o, string n)
    {
        object v = F(o, n);
        if (v == null) return 0L;
        try { return Convert.ToInt64(v); } catch { return 0L; }
    }

    // A `Lap` is the split point itself: `time` + `lapType` (+ `description`).
    // Waypoints also carry `time`, so requiring `lapType` tells them apart.
    static void CollectLaps(object o, List<object> found, int depth)
    {
        if (o == null || depth > 14 || found.Count > 200000) return;
        Bag b = o as Bag;
        if (b != null)
        {
            if (b.keys.Contains("time") && b.keys.Contains("lapType"))
            { found.Add(o); return; }
            foreach (object v in b.values) CollectLaps(v, found, depth + 1);
            return;
        }
        IEnumerable en = o as IEnumerable;
        if (en != null && !(o is string)) foreach (object x in en) CollectLaps(x, found, depth + 1);
    }

    static void CollectHandles(object o, List<object> found, int depth)
    {
        if (o == null || depth > 14 || found.Count > 200000) return;
        Bag b = o as Bag;
        if (b != null)
        {
            if (b.keys.Contains("location") && b.keys.Contains("parameterizedLocation"))
            { found.Add(o); return; }
            foreach (object v in b.values) CollectHandles(v, found, depth + 1);
            return;
        }
        IEnumerable en = o as IEnumerable;
        if (en != null && !(o is string)) foreach (object x in en) CollectHandles(x, found, depth + 1);
    }

    // A GeneralMatrix arrives as a Bag with { m, n, a } where `a` is double[][].
    static void AppendMatrix(StringBuilder sb, object tm)
    {
        if (tm == null) { sb.Append("null"); return; }
        sb.Append("{\"m\":").Append(((int)Fd(tm, "m")).ToString(CultureInfo.InvariantCulture));
        sb.Append(",\"n\":").Append(((int)Fd(tm, "n")).ToString(CultureInfo.InvariantCulture));
        sb.Append(",\"a\":[");
        object aRaw = F(tm, "a");
        IEnumerable rows = aRaw as IEnumerable;
        bool firstRow = true;
        if (rows != null)
        {
            foreach (object row in rows)
            {
                if (!firstRow) sb.Append(',');
                firstRow = false;
                sb.Append('[');
                IEnumerable cols = row as IEnumerable;
                bool firstCol = true;
                if (cols != null)
                {
                    foreach (object v in cols)
                    {
                        if (!firstCol) sb.Append(',');
                        firstCol = false;
                        sb.Append(Convert.ToDouble(v, CultureInfo.InvariantCulture)
                                   .ToString("R", CultureInfo.InvariantCulture));
                    }
                }
                sb.Append(']');
            }
        }
        sb.Append("]}");
    }

    public static string Run(string path)
    {
        j = new StringBuilder(1 << 22);
        var fs = new FileStream(path, FileMode.Open, FileAccess.Read);
        object root;
        try
        {
            var bf = new BinaryFormatter();
            bf.Binder = new CaptureBinder();
            root = bf.Deserialize(fs);
        }
        finally { fs.Close(); }

        j.Append('{');
        // ---- map ----
        object map = F(root, "map");
        byte[] raw = F(map, "rawData") as byte[];
        j.Append("\"mapBytes\":").Append(raw == null ? 0 : raw.Length).Append(',');
        j.Append("\"mapBase64\":");
        if (raw == null) j.Append("null");
        else { j.Append('"').Append(Convert.ToBase64String(raw)).Append('"'); }
        j.Append(',');

        // ---- projectionOrigin ----
        object po = F(root, "projectionOrigin");
        j.Append("\"projLon\":"); Num(Fd(po, "longitude"));
        j.Append(",\"projLat\":"); Num(Fd(po, "latitude"));

        // ---- sessions -> route waypoints + handles ----
        object sessions = F(root, "sessions");
        List<object> sList = Items(sessions);
        if (sList.Count == 0)
        {
            object inner = F(sessions, "sessions");
            sList = Items(inner);
        }
        j.Append(",\"sessionCount\":").Append(sList.Count);

        j.Append(",\"waypoints\":[");
        bool firstWp = true;
        j.Append("],\"handles\":[],\"segments\":0");   // placeholders, filled below
        // Rebuild properly: we need waypoints before closing the object.
        j.Length -= "],\"handles\":[],\"segments\":0".Length;

        int segCount = 0;
        var wpOut = new StringBuilder();
        var hOut = new StringBuilder();
        int wpCount = 0, hCount = 0;
        if (sList.Count > 0)
        {
            object s0 = sList[0];
            object route = F(s0, "route");
            List<object> segs = Items(F(route, "segments"));
            segCount = segs.Count;
            foreach (object seg in segs)
            {
                foreach (object wp in Items(F(seg, "waypoints")))
                {
                    object ll = F(wp, "longLat");
                    object t = F(wp, "time");
                    long ticks = t is DateTime ? ((DateTime)t).Ticks : 0L;
                    double alt = Fd(wp, "altitude"), hr = Fd(wp, "heartRate");
                    if (!firstWp) wpOut.Append(',');
                    firstWp = false;
                    wpOut.Append("{\"lon\":"); wpOut.Append(Fd(ll, "longitude").ToString("R", CultureInfo.InvariantCulture));
                    wpOut.Append(",\"lat\":"); wpOut.Append(Fd(ll, "latitude").ToString("R", CultureInfo.InvariantCulture));
                    wpOut.Append(",\"time\":"); wpOut.Append(ticks.ToString(CultureInfo.InvariantCulture));
                    wpOut.Append(",\"alt\":").Append(alt.ToString("R", CultureInfo.InvariantCulture));
                    wpOut.Append(",\"hr\":").Append(hr.ToString("R", CultureInfo.InvariantCulture));
                    wpOut.Append('}');
                    wpCount++;
                }
            }
            var handles = new List<object>();
            object hb = F(s0, "handles");
            CollectHandles(F(hb, "handles") ?? hb, handles, 0);
            bool firstH = true;
            foreach (object h in handles)
            {
                object loc = F(h, "location");
                object pl = F(h, "parameterizedLocation");
                object ty = F(h, "type");
                if (!firstH) hOut.Append(',');
                firstH = false;
                hOut.Append("{\"x\":").Append(Fd(loc, "x").ToString("R", CultureInfo.InvariantCulture));
                hOut.Append(",\"y\":").Append(Fd(loc, "y").ToString("R", CultureInfo.InvariantCulture));
                hOut.Append(",\"segmentIndex\":").Append(((int)Fd(pl, "segmentIndex")).ToString(CultureInfo.InvariantCulture));
                hOut.Append(",\"value\":").Append(Fd(pl, "value").ToString("R", CultureInfo.InvariantCulture));
                hOut.Append(",\"type\":").Append(((int)Fd(ty, "value__")).ToString(CultureInfo.InvariantCulture));
                hOut.Append(",\"matrix\":"); AppendMatrix(hOut, F(h, "transformationMatrix"));
                hOut.Append('}');
                hCount++;
            }
        }
        j.Append(wpOut);
        j.Append("],\"handles\":[");
        j.Append(hOut);
        j.Append("],\"segments\":").Append(segCount);
        j.Append(",\"waypointCount\":").Append(wpCount);
        j.Append(",\"handleCount\":").Append(hCount);

        // ---- laps: the split points, one LapCollection per session ----
        var lapOut = new StringBuilder();
        int lapCount = 0;
        var perSession = new StringBuilder();
        bool firstS = true;
        foreach (object s in sList)
        {
            var found = new List<object>();
            object lc = F(s, "laps");
            CollectLaps(lc, found, 0);
            if (!firstS) perSession.Append(',');
            firstS = false;
            perSession.Append(found.Count);
            foreach (object l in found)
            {
                object t = F(l, "time");
                long ticks = t is DateTime ? ((DateTime)t).Ticks : 0L;
                if (lapCount > 0) lapOut.Append(',');
                lapOut.Append("{\"time\":").Append(ticks.ToString(CultureInfo.InvariantCulture));
                lapOut.Append(",\"lapType\":").Append(Fi(l, "lapType").ToString(CultureInfo.InvariantCulture));
                lapOut.Append('}');
                lapCount++;
            }
        }
        j.Append(",\"laps\":[").Append(lapOut).Append(']');
        j.Append(",\"lapCount\":").Append(lapCount);
        j.Append(",\"lapsPerSession\":[").Append(perSession).Append(']');
        j.Append(",\"initialMatrices\":[");
        bool firstM = true;
        foreach (object s in sList)
        {
            if (!firstM) j.Append(',');
            firstM = false;
            AppendMatrix(j, F(s, "initialTransformationMatrix"));
        }
        j.Append(']');
        j.Append(",\"sessionKeys\":[");
        bool firstK = true;
        if (sList.Count > 0)
        {
            Bag sb = sList[0] as Bag;
            if (sb != null)
            {
                foreach (string k in sb.keys)
                {
                    if (!firstK) j.Append(',');
                    firstK = false;
                    Str(k);
                }
            }
        }
        j.Append(']');
        j.Append('}');
        return j.ToString();
    }
}
'@
Add-Type -TypeDefinition $src -Language CSharp

Write-Output "reading $QrtPath"
$json = [QrtExtract]::Run($QrtPath)
[System.IO.File]::WriteAllText($OutPath, $json, (New-Object System.Text.UTF8Encoding($false)))
Write-Output ("wrote {0} ({1:N0} bytes)" -f $OutPath, $json.Length)
