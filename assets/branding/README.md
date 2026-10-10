# TraceReel branding

One approved contour geometry, three backgrounds. `contours.json` is the
canonical vector master (the original design raster is a personal file and is
not shipped). All SVGs use the exact same contour path and viewBox; there are no
independently drawn variants.

- `tracereel-icon`: orange rounded tile and white contours, for the native app
  icon. `tracereel.ico` contains Windows sizes 16 through 256.
- `tracereel-mark`: orange contours with opaque white interior, transparent
  outside, for the menu brand button and About dialog.
- `tracereel-white`: the same mark on a white canvas, for light-background use.

Qt embeds the PNGs through `app/src/branding.rs`; the Windows build embeds the
ICO through `app/tracereel.rc`. No external asset path is needed at runtime.
The existing portable package copies the compiled executable, so it carries
these resources automatically.

To regenerate SVG/PNG/ICO exports, run `python tools/export_branding.py` with
numpy, Pillow and PyMuPDF. Normal exports read the committed master and retain
its geometry exactly. `--trace` rebuilds the master from the original design
raster, which is not part of this repository: set `TRACEREEL_BRAND_SOURCE` to
your own copy if you need it.
