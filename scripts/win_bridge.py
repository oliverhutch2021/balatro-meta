"""A relay for `make watch`: lets Ubuntu talk to BalatroBot on Windows' own localhost.

BalatroBot listens on Windows' 127.0.0.1, which WSL's default (NAT) networking can't reach. watch.py
starts this under Windows Python, through WSL's Windows interop, and talks to it over stdin/stdout:
one JSON request per line ({"id", "url", "body", "timeout"}) in, one JSON line out ({"id", "ok": true,
"result": ...} or {"id", "ok": false, "error": "..."}; the id is the request's, so the caller can tell a
late reply from the one it waits for). Standard library only; no proxy is used.
"""
import json
import sys
import urllib.request


def main() -> None:
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        req = {}
        try:
            req = json.loads(line)
            data = json.dumps(req["body"]).encode("utf-8")
            r = urllib.request.Request(req["url"], data=data, headers={"Content-Type": "application/json"})
            with opener.open(r, timeout=req.get("timeout", 120)) as resp:
                out = {"ok": True, "result": json.loads(resp.read().decode("utf-8"))}
        except Exception as e:  # noqa: BLE001 - every failure must go back to the caller as a line
            out = {"ok": False, "error": f"{type(e).__name__}: {e}"}
        out["id"] = req.get("id") if isinstance(req, dict) else None
        sys.stdout.write(json.dumps(out) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
