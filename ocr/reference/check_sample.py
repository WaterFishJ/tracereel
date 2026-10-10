"""Manual integration probe using the supplied receipt, without shell encoding loss."""
import json
from receipt_ocr import recognize

if __name__ == "__main__":
    import argparse
    import subprocess
    parser = argparse.ArgumentParser()
    parser.add_argument("image", help="Sample receipt supplied for this feature")
    parser.add_argument("--sidecar", help="Also verify standalone OCR executable")
    args = parser.parse_args()
    request = {
        "path": args.image,
        "corners": [[.335, .485], [.676, .483], [.639, .863], [.335, .85]],
        "rotation": 0,
    }
    result = recognize(request)
    expected = ["00:01:21","00:01:45","00:03:36","00:04:07","00:04:49","00:05:46","00:08:00","00:08:43","00:09:31","00:12:45","00:15:03","00:17:21","00:18:01","00:19:45","00:20:04","00:20:21","00:20:53","00:21:35","00:22:03.1"]
    def verify(data):
        assert data["columns"] == 4
        assert [r["cells"][2] for r in data["rows"][2:]] == expected
        assert data["rows"][13]["cells"][1] == data["rows"][19]["cells"][1] == "120"
    verify(result)
    if args.sidecar:
        output = subprocess.run([args.sidecar], input=json.dumps(request).encode("utf-8"), capture_output=True, timeout=60, check=True)
        verify(json.loads(output.stdout))
        print("Standalone OCR verification passed.")
    print(json.dumps(result, ensure_ascii=True))
