#!/usr/bin/env python3
"""Play saved real-game differences again through today's checks, in the real game (make check-saved): each file's
seed and moves, game and engine side by side as `make watch`'s checks play them, up to the move that differed.
Passes print "ok"; a difference is saved again as a new file beside the others, as the checks always do.

    scripts/check_saved.py .sync/watch/mismatches/5VQWSWEC-move17.json [more files...]

Needs Balatro and BalatroBot as `make watch` does (the hidden game on the checks' port); seconds per file.
"""
from __future__ import annotations

import json
import sys
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import checker  # noqa: E402

CONNECT_SECONDS = 240


def main(argv: list[str]) -> int:
    files = [Path(a) for a in argv]
    if not files:
        print(__doc__, file=sys.stderr)
        return 2
    c = checker.Checker(launch=True)
    stop = threading.Event()
    c.stop_event = stop
    threading.Thread(target=c.keep_control, args=(stop,), daemon=True).start()
    try:
        t0 = time.time()
        while not c.connect():
            if time.time() - t0 > CONNECT_SECONDS:
                print("The hidden game didn't answer: is Balatro with BalatroBot set up as for make watch?")
                return 1
            time.sleep(2)
        failed = 0
        for f in files:
            d = json.loads(f.read_text())
            moves = [tuple(m) for m in d["moves"]]
            ok, msg = c.check_run(d["seed"], moves, f"recheck {f.name}", int(d.get("stake") or 1))
            failed += not ok
            print(f"{f.name}: {'ok' if ok else 'still differs'}: {msg}")
        return 1 if failed else 0
    finally:
        c.close()


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
