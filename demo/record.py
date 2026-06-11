#!/usr/bin/env python3
"""Record a real `talk` session to an asciinema .cast for the README demo.

talk's live edge is driven by the microphone, so a VHS tape can't capture it.
This drives a genuine session: it spawns `talk` on a real pty, plays the demo
line through the speakers (which the mic hears — a local acoustic loopback), then
sends `space` to finish. Output is written as an asciinema v2 .cast; render it to
a GIF with:  agg demo/talk.cast demo/talk.gif

Usage:
  python3 demo/record.py --mode journal --out demo/talk.cast
  python3 demo/record.py --mode byo --question "What am I avoiding?" \
      --speech "I keep putting off the conversation with my brother." \
      --speak-at 4 --space-at 13 --end-at 16
"""
import argparse, fcntl, json, os, pty, select, struct, subprocess, sys, termios, time


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default="./target/release/talk")
    ap.add_argument("--mode", choices=["journal", "reflect", "byo"], default="journal")
    ap.add_argument("--question", default="What am I avoiding?")
    ap.add_argument("--speech", default="I keep putting off the conversation with my brother. "
                                        "But I think part of me already knows it is time.")
    ap.add_argument("--rate", type=int, default=175)        # say words-per-minute
    ap.add_argument("--voice", default=None)                # say voice; None = system default
    ap.add_argument("--speak-at", type=float, default=4.0)  # seconds after launch to start speaking
    ap.add_argument("--space-at", type=float, default=14.0) # seconds to send `space` (finish)
    ap.add_argument("--end-at", type=float, default=18.0)   # hard stop if talk hasn't exited
    ap.add_argument("--cols", type=int, default=80)
    ap.add_argument("--rows", type=int, default=22)
    ap.add_argument("--out", default="demo/talk.cast")
    a = ap.parse_args()

    talk_args = {"journal": ["journal"], "reflect": ["reflect"], "byo": [a.question]}[a.mode]

    pid, fd = pty.fork()
    if pid == 0:
        os.environ["TERM"] = "xterm-256color"
        os.environ["COLUMNS"], os.environ["LINES"] = str(a.cols), str(a.rows)
        os.execvp(a.bin, [a.bin] + talk_args)
        os._exit(127)

    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", a.rows, a.cols, 0, 0))
    start = time.time()
    events: list = []
    spoke = sent_space = False
    say_proc = None

    while True:
        t = time.time() - start
        if not spoke and t >= a.speak_at:
            cmd = ["say", "-r", str(a.rate)] + (["-v", a.voice] if a.voice else []) + [a.speech]
            say_proc = subprocess.Popen(cmd)
            spoke = True
        if spoke and not sent_space and t >= a.space_at:
            os.write(fd, b" ")
            sent_space = True

        r, _, _ = select.select([fd], [], [], 0.05)
        if fd in r:
            try:
                data = os.read(fd, 65536)
            except OSError:
                break
            if not data:
                break
            os.write(sys.stdout.fileno(), data)
            events.append([round(time.time() - start, 4), "o", data.decode("utf-8", "replace")])

        wpid, _ = os.waitpid(pid, os.WNOHANG)
        if wpid == pid:
            # drain any trailing output
            try:
                while True:
                    r, _, _ = select.select([fd], [], [], 0.1)
                    if fd not in r:
                        break
                    data = os.read(fd, 65536)
                    if not data:
                        break
                    os.write(sys.stdout.fileno(), data)
                    events.append([round(time.time() - start, 4), "o", data.decode("utf-8", "replace")])
            except OSError:
                pass
            break
        if t >= a.end_at:
            os.write(fd, b" ")  # nudge finish, then give up next loop
            if t >= a.end_at + 4:
                break

    if say_proc and say_proc.poll() is None:
        say_proc.terminate()

    header = {"version": 2, "width": a.cols, "height": a.rows,
              "env": {"TERM": "xterm-256color", "SHELL": "/bin/zsh"}}
    with open(a.out, "w") as f:
        f.write(json.dumps(header) + "\n")
        for ev in events:
            f.write(json.dumps(ev) + "\n")
    dur = events[-1][0] if events else 0
    print(f"\n[record] wrote {a.out}: {len(events)} events, {dur:.1f}s", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
