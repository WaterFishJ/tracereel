# Compare the residual frame-to-frame rotation (jitter) of two stabilized videos in a
# time window, using cv2 ECC (MOTION_EUCLIDEAN). Higher/spikier residual rotation =
# worse stabilization. CLI helper (not a Rust bin).
#
# The source may be HEVC (not decodable by OpenCV), so each window is first re-encoded
# to a small H.264 mp4 by ffmpeg, then read with cv2.
#
# usage: python compare_stab.py <videoA> <videoB> <start_s> <dur_s>

import sys, math, os, subprocess, tempfile
import cv2, numpy as np

FFMPEG = os.environ.get(
    "TRACEREEL_FFMPEG",
    os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                 "tools", "ffmpeg", "bin", "ffmpeg.exe"),
)
W, H, FPS = 320, 240, 15

def window_mp4(path, start_s, dur_s, out):
    subprocess.run([FFMPEG, "-y", "-i", path, "-ss", f"{start_s}", "-t", f"{dur_s}",
                    "-r", str(FPS), "-vf", f"scale={W}:{H}", "-pix_fmt", "yuv420p",
                    "-c:v", "libx264", "-preset", "ultrafast", out],
                   capture_output=True)
    return out

def read_window(path, start_s, dur_s):
    tmp = os.path.join(tempfile.gettempdir(), f"ovcmp_{abs(hash(path))}.mp4")
    window_mp4(path, start_s, dur_s, tmp)
    cap = cv2.VideoCapture(tmp)
    if not cap.isOpened():
        print("cannot open", tmp); return []
    frames = []
    while True:
        ok, fr = cap.read()
        if not ok: break
        g = cv2.cvtColor(cv2.resize(fr, (W, H), interpolation=cv2.INTER_AREA), cv2.COLOR_BGR2GRAY).astype(np.float32)
        frames.append(g)
    cap.release()
    os.remove(tmp)
    return frames

def ecc_rot(frames):
    rots = []
    prev = None
    for img in frames:
        if prev is not None:
            warp = np.eye(2, 3, dtype=np.float32)
            try:
                cc, warp = cv2.findTransformECC(prev, img, warp, cv2.MOTION_EUCLIDEAN,
                                                (cv2.TERM_CRITERIA_EPS | cv2.TERM_CRITERIA_COUNT, 60, 1e-6), None, 3)
            except Exception:
                prev = img
                continue
            rots.append(math.degrees(math.atan2(warp[1, 0], warp[0, 0])))
        prev = img
    if not rots: return (0,0,0,0)
    a = np.array(rots)
    return (float(np.max(np.abs(a))), float(np.mean(np.abs(a))), float(np.std(a)), int(np.sum(np.abs(a) > 0.4)))

if __name__ == "__main__":
    a, b = sys.argv[1], sys.argv[2]
    start, dur = float(sys.argv[3]), float(sys.argv[4])
    fa = read_window(a, start, dur)
    fb = read_window(b, start, dur)
    ra = ecc_rot(fa); rb = ecc_rot(fb)
    print(f"frames A={len(fa)} B={len(fb)}")
    print(f"A {os.path.basename(a)}: max_rot={ra[0]:.3f}deg mean_abs={ra[1]:.3f} std={ra[2]:.4f} spikes(>0.4deg)={ra[3]}")
    print(f"B {os.path.basename(b)}: max_rot={rb[0]:.3f}deg mean_abs={rb[1]:.3f} std={rb[2]:.4f} spikes(>0.4deg)={rb[3]}")
