# Compare OUR smoothed quats (quats_ours.csv) with gyroflow native stab_quat
# (gyroflow_meta.json) over the same window. Reports per-sample angular difference
# and the frame-to-frame delta rotation magnitudes for both (the foot-strike
# high-frequency lives in the deltas).
import json, math, csv

def qnorm(q):
    n = math.sqrt(sum(x * x for x in q))
    return [x / n for x in q]

def qdot(a, b):
    return sum(x * y for x, y in zip(a, b))

def angle_between(a, b):
    d = abs(qdot(qnorm(a), qnorm(b)))
    d = min(1.0, d)
    return math.degrees(2.0 * math.acos(d))

def nlerp(a, b, t):
    a = qnorm(a); b = qnorm(b)
    d = qdot(a, b)
    if d < 0: b = [-x for x in b]
    out = [a[i] * (1 - t) + b[i] * t for i in range(4)]
    return qnorm(out)

# --- native ---
meta = json.load(open("gyroflow_meta.json", encoding="utf-8"))
ts = [e["timestamp_ms"] for e in meta]
qts = [e["stab_quat"] for e in meta]
print(f"native entries: {len(ts)}, t0={ts[0]:.3f} t1={ts[1]:.3f} step={ts[1]-ts[0]:.4f}ms last={ts[-1]:.1f}")

def native_quat(t):
    if t <= ts[0]: return qts[0]
    if t >= ts[-1]: return qts[-1]
    # binary search
    lo, hi = 0, len(ts) - 1
    while hi - lo > 1:
        mid = (lo + hi) // 2
        if ts[mid] <= t: lo = mid
        else: hi = mid
    if ts[hi] - ts[lo] < 1e-9: return qts[lo]
    f = (t - ts[lo]) / (ts[hi] - ts[lo])
    return nlerp(qts[lo], qts[hi], f)

# --- ours ---
ours_t = []
ours_q = []
for row in csv.DictReader(open("quats_ours.csv", encoding="utf-8")):
    ours_t.append(float(row["t_ms"]))
    ours_q.append([float(row["w"]), float(row["x"]), float(row["y"]), float(row["z"])])
print(f"ours samples: {len(ours_t)}, t0={ours_t[0]:.3f} step={ours_t[1]-ours_t[0]:.4f}ms")

# per-sample angle difference
diffs = []
for t, q in zip(ours_t, ours_q):
    diffs.append(angle_between(q, native_quat(t)))
dmax = max(diffs); dmean = sum(diffs) / len(diffs)
print(f"\nour vs native smoothed quat angle: max={dmax:.4f}deg mean={dmean:.4f}deg")

# frame-to-frame delta rotation (high-frequency residual) for both series
def deltas(times, quats):
    out = []
    for i in range(1, len(quats)):
        out.append(angle_between(quats[i-1], quats[i]))
    return out

our_d = deltas(ours_t, ours_q)
# native deltas at the same timestamps
nat_t = [native_quat(t) for t in ours_t]
nat_d = deltas(ours_t, nat_t)

def stats(d):
    return (max(d), sum(d)/len(d), sum(1 for x in d if x > 0.05))

print(f"\nframe-delta rotation (deg):")
print(f"  ours  : max={stats(our_d)[0]:.4f} mean={stats(our_d)[1]:.4f} >0.05deg={stats(our_d)[2]}")
print(f"  native: max={stats(nat_d)[0]:.4f} mean={stats(nat_d)[1]:.4f} >0.05deg={stats(nat_d)[2]}")

# find worst sample offsets
worst = sorted(zip(ours_t, diffs), key=lambda x: -x[1])[:5]
print(f"\nworst samples (t_ms, diff_deg): {[(f'{t:.1f}', f'{d:.4f}') for t, d in worst]}")
