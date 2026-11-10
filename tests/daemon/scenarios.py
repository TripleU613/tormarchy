"""The daemon, as the panel meets it: the real tormarchyd process, driven over
its stdin and read from its stdout, against stand-ins for everything it talks
to -- a control port on a real socket, a root helper that records what it was
asked, Omarchy's notifier and wl-paste on PATH, and a scratch filesystem root.

    python3 tests/daemon/scenarios.py [path/to/tormarchyd]

What these hold is the state machine the panel depends on: the switch reacting
before tor does, a connect cancellable while it waits, a brief tor restart not
mistaken for the kill switch, one toast at a time replacing the last, bridge
lines going to the helper over stdin and never argv.
"""
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
DAEMON = sys.argv[1] if len(sys.argv) > 1 else os.path.join(ROOT, "target", "debug", "tormarchyd")

failures = 0


def check(ok, what):
    global failures
    print(("  ok    " if ok else "  FAIL  ") + what)
    if not ok:
        failures += 1


class FakeTor:
    """A control port: AUTHENTICATE, GETINFO for vitals and an empty circuit
    list, SETEVENTS, SIGNAL NEWNYM. stop() closes everything, as tor going away
    does; start() listens again on the same port."""

    def __init__(self, port):
        self.port = port
        self.bootstrap = 100
        self.conns = []
        self.newnyms = 0
        self.srv = None
        self.lock = threading.Lock()

    def start(self):
        self.srv = socket.socket()
        self.srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.srv.bind(("127.0.0.1", self.port))
        self.srv.listen()
        threading.Thread(target=self._accept, args=(self.srv,), daemon=True).start()

    def stop(self):
        if self.srv:
            # shutdown, not just close: a close while another thread sits in
            # accept() leaves the socket listening on Linux.
            try:
                self.srv.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            self.srv.close()
            self.srv = None
        with self.lock:
            for c in self.conns:
                try:
                    c.shutdown(socket.SHUT_RDWR)
                    c.close()
                except OSError:
                    pass
            self.conns = []

    def push(self, line):
        with self.lock:
            for c in self.conns:
                try:
                    c.sendall(line.encode() + b"\r\n")
                except OSError:
                    pass

    def _accept(self, srv):
        while True:
            try:
                c, _ = srv.accept()
            except OSError:
                return
            with self.lock:
                self.conns.append(c)
            threading.Thread(target=self._serve, args=(c,), daemon=True).start()

    def _serve(self, c):
        f = c.makefile("rb")
        try:
            for raw in f:
                line = raw.decode().strip()
                if line.startswith("AUTHENTICATE") or line.startswith("SETEVENTS"):
                    out = "250 OK\r\n"
                elif line.startswith("SIGNAL NEWNYM"):
                    self.newnyms += 1
                    out = "250 OK\r\n"
                elif line.startswith("GETINFO"):
                    out = ""
                    for key in line.split()[1:]:
                        if key == "status/bootstrap-phase":
                            out += f"250-status/bootstrap-phase=NOTICE BOOTSTRAP PROGRESS={self.bootstrap} TAG=x\r\n"
                        elif key.startswith("traffic/"):
                            out += f"250-{key}=1024\r\n"
                        elif key == "circuit-status":
                            out += "250+circuit-status=\r\n.\r\n"
                    out += "250 OK\r\n"
                elif line.startswith("QUIT"):
                    break
                else:
                    out = "510 Unrecognized command\r\n"
                c.sendall(out.encode())
        except OSError:
            pass


def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


class World:
    def __init__(self, settings=None, tor_up=True):
        self.dir = tempfile.mkdtemp(prefix="tormarchyd-test-")
        self.root = os.path.join(self.dir, "root")
        self.bin = os.path.join(self.dir, "bin")
        self.log = os.path.join(self.dir, "log")
        os.makedirs(os.path.join(self.root, "etc/tor/torrc.d"))
        os.makedirs(os.path.join(self.root, "run/tormarchy"))
        os.makedirs(self.bin)
        open(os.path.join(self.root, "etc/tor/torrc.d/00-tormarchy.conf"), "w").write("SocksPort 9050\n")
        open(os.path.join(self.root, "run/tormarchy/control.cookie"), "wb").write(b"\x07" * 32)
        open(self.log, "w").close()

        # The root helper, recording argv and stdin, and acting on the scratch
        # root the way the real one acts on the machine.
        self.helper = os.path.join(self.bin, "tormarchy")
        self._script(self.helper, f"""#!/bin/bash
root={self.root}
log={self.log}
input=""
[[ $1 == bridge && $2 == set ]] && input=$(cat)
printf 'helper %s\\n' "$*" >>"$log"
[[ -n $input ]] && printf 'stdin %s\\n' "$(printf '%s' "$input" | tr '\\n' '|')" >>"$log"
knob() {{ cat "$root/knob.$1" 2>/dev/null; }}
case $1 in
connect)
  hold=$(knob connect-hold); [[ -n $hold ]] && sleep "$hold"
  code=$(knob connect-exit)
  if [[ -n $code && $code != 0 ]]; then echo "tormarchy: tor stalled at 40% bootstrap. No rules applied." >&2; exit "$code"; fi
  [[ -e $root/knob.cancelled ]] && {{ echo "tormarchy: cancelled" >&2; exit 1; }}
  printf '%s\\n' "${{2:-lan}}" >"$root/run/tormarchy/mode"
  [[ $2 != socks ]] && : >"$root/run/tormarchy/active"
  echo "Connected." ;;
disconnect)
  code=$(knob disconnect-exit); [[ -n $code && $code != 0 ]] && {{ echo "tormarchy: could not stop" >&2; exit "$code"; }}
  : >"$root/knob.cancelled"
  rm -f "$root/run/tormarchy/active" "$root/run/tormarchy/mode"
  echo "Disconnected." ;;
bridge) echo "Bridges active." ;;
*) echo ok ;;
esac
""")
        self._script(os.path.join(self.bin, "omarchy-notification-send"), f"""#!/bin/bash
printf 'notify' >>{self.log}; printf ' [%s]' "$@" >>{self.log}; printf '\\n' >>{self.log}
echo 7
""")
        self._script(os.path.join(self.bin, "wl-paste"), f"""#!/bin/bash
code=$(cat {self.root}/knob.clipboard-exit 2>/dev/null)
[[ -n $code ]] && exit "$code"
sleep "$(cat {self.root}/knob.clipboard-delay 2>/dev/null || echo 0)"
cat {self.root}/knob.clipboard 2>/dev/null
""")

        self.tor = FakeTor(free_port())
        if tor_up:
            self.tor.start()
        env = dict(os.environ)
        env.update({
            "TORMARCHY_ROOT": self.root,
            "TORMARCHY_HELPER": self.helper,
            "TORMARCHY_CONTROL_PORT": str(self.tor.port),
            "PATH": self.bin + ":" + env.get("PATH", "/usr/bin:/bin"),
        })
        self.proc = subprocess.Popen([DAEMON], stdin=subprocess.PIPE, stdout=subprocess.PIPE, env=env, text=True, bufsize=1)
        self.view = None
        self.views = []
        threading.Thread(target=self._read, daemon=True).start()
        self.send({"cmd": "hello", "settings": settings or {}, "pluginDir": ROOT})

    def _script(self, path, body):
        with open(path, "w") as f:
            f.write(body)
        os.chmod(path, 0o755)

    def _read(self):
        for line in self.proc.stdout:
            try:
                self.view = json.loads(line)["view"]
                self.views.append(self.view)
            except (ValueError, KeyError):
                pass

    def send(self, obj):
        self.proc.stdin.write(json.dumps(obj) + "\n")
        self.proc.stdin.flush()

    def knob(self, name, value=None):
        path = os.path.join(self.root, "knob." + name)
        if value is None:
            if os.path.exists(path):
                os.remove(path)
        else:
            open(path, "w").write(str(value))

    def until(self, pred, timeout=6.0):
        end = time.time() + timeout
        while time.time() < end:
            v = self.view
            if v is not None:
                try:
                    if pred(v):
                        return True
                except (KeyError, TypeError):
                    pass
            time.sleep(0.05)
        return False

    def logged(self, prefix):
        return [l.rstrip("\n") for l in open(self.log) if l.startswith(prefix)]

    def set_rules(self, on, mode="lan"):
        active = os.path.join(self.root, "run/tormarchy/active")
        open(os.path.join(self.root, "run/tormarchy/mode"), "w").write(mode + "\n")
        if on:
            open(active, "w").close()
        elif os.path.exists(active):
            os.remove(active)

    def close(self):
        try:
            self.proc.stdin.close()
            self.proc.wait(timeout=5)
        except Exception:
            self.proc.kill()
        self.tor.stop()
        shutil.rmtree(self.dir, ignore_errors=True)


def toasts(w):
    return w.logged("notify")


print("Reading tor")
w = World()
check(w.until(lambda v: v["installed"] and v["bar"] == "off"), "installed, tor up, no rules: the onion is off")
check(w.until(lambda v: v["statusText"] == "Tor is running but traffic is not routed through it"), "and says so in words")
w.tor.bootstrap = 40
w.tor.push("650 STATUS_CLIENT NOTICE BOOTSTRAP PROGRESS=55 TAG=x SUMMARY=\"x\"")
check(w.until(lambda v: v["switchCaption"] == "55%"), "a bootstrap step tor pushes reaches the panel without a poll")
w.close()

print("The switch")
w = World()
w.until(lambda v: v["installed"])
w.knob("connect-hold", 1)
w.send({"cmd": "toggle"})
check(w.until(lambda v: v["active"] and v["busy"], 2), "the switch turns on the instant it is clicked")
check(w.until(lambda v: v["canToggle"], 2), "and can be turned off again while the connect waits")
check(w.until(lambda v: v["bar"] == "routed" and not v["busy"]), "it settles on routed once rules are on")
check(w.logged("helper connect") == ["helper connect lan"], "connect went to the helper in the default mode")
w.send({"cmd": "toggle"})
check(w.until(lambda v: not v["active"] and v["bar"] == "off"), "off again")
check(toasts(w) == [], "an ordinary connect and disconnect say nothing")
w.close()

print("Cancelling a connect")
w = World()
w.until(lambda v: v["installed"])
w.knob("connect-hold", 2)
w.send({"cmd": "toggle"})
w.until(lambda v: v["busy"], 2)
time.sleep(0.2)
w.send({"cmd": "toggle"})
check(w.until(lambda v: "disconnect" in " ".join(w.logged("helper")), 2), "turning it off mid-connect sends the cancel")
check(w.until(lambda v: not v["busy"] and not v["active"], 6), "and the switch ends off")
check(w.view["error"]["full"] == "", "a connect the user cancelled is not reported as an error")
w.close()

w = World()
w.until(lambda v: v["installed"])
w.knob("connect-hold", 2)
w.knob("disconnect-exit", 1)
w.send({"cmd": "toggle"})
w.until(lambda v: v["busy"], 2)
time.sleep(0.2)
w.send({"cmd": "toggle"})
check(w.until(lambda v: "could not stop" in v["error"]["full"], 4), "a cancel that fails says so")
check(w.until(lambda v: v["bar"] == "routed", 6), "and the connect it failed to stop still lands")
w.close()

print("Notifications")
w = World()
w.until(lambda v: v["installed"])
w.set_rules(True)
check(w.until(lambda v: v["bar"] == "routed"), "rules on with tor up reads as routed")
time.sleep(0.6)
check(toasts(w) == [], "the first reading is a baseline, not a change")
w.tor.stop()
check(w.until(lambda v: v["bar"] == "blocked", 4), "tor gone with rules on is the kill switch holding")
time.sleep(1.5)
check(toasts(w) == [], "not toasted at once: it could be tor restarting")
check(w.until(lambda v: len(toasts(w)) == 1, 8), "a block that lasts raises one toast")
sent = toasts(w)
check(sent and "[Kill switch engaged]" in sent[0] and "[critical]" in sent[0], "a critical one, in a few words")
check(sent and sent[0].endswith("[--exec] [omarchy-shell] [tripleu.tor] [open]"), "and clicking it opens the panel")
w.tor.start()
check(w.until(lambda v: len(toasts(w)) == 2, 6), "tor coming back is news too")
sent = toasts(w)
check(len(sent) == 2 and "[Tor is back]" in sent[1] and "[-r] [7]" in sent[1], "and replaces the kill-switch toast instead of stacking")
w.close()

w = World()
w.until(lambda v: v["installed"])
w.set_rules(True)
w.until(lambda v: v["bar"] == "routed")
w.tor.stop()
w.until(lambda v: v["bar"] == "blocked", 4)
w.tor.start()
w.until(lambda v: v["bar"] == "routed", 6)
time.sleep(1)
check(toasts(w) == [], "a tor restart that recovers in time says nothing")
w.close()

w = World({"notifications": "off"})
w.until(lambda v: v["installed"])
w.set_rules(True)
w.until(lambda v: v["bar"] == "routed")
w.tor.stop()
time.sleep(6)
check(toasts(w) == [], "notifications off sends nothing, not even the kill switch")
w.close()

w = World({"notifications": "all"})
w.until(lambda v: v["installed"] and v["bar"] == "off")
w.send({"cmd": "toggle"})
check(w.until(lambda v: len(toasts(w)) == 1, 6) and "[Connected through Tor]" in toasts(w)[0], "with notifications all, a connect is announced")
w.close()

w = World()
w.until(lambda v: v["installed"])
w.knob("connect-exit", 1)
w.send({"cmd": "toggle"})
check(w.until(lambda v: len(toasts(w)) == 1, 6), "a connect that fails on its own raises a toast")
check(toasts(w) and "[Tor stalled at 40% bootstrap]" in toasts(w)[0], "with the reason, cut to its first clause")
check(w.until(lambda v: not v["active"] and "stalled" in v["error"]["full"]), "and the switch falls back with the error shown")
w.close()

w = World()
w.until(lambda v: v["installed"])
w.knob("connect-exit", 126)
w.send({"cmd": "toggle"})
time.sleep(1.5)
check(toasts(w) == [], "dismissing the password prompt is not a failure worth a toast")
w.close()

w = World()
w.until(lambda v: v["installed"])
os.remove(os.path.join(w.bin, "omarchy-notification-send"))
w.set_rules(True)
w.until(lambda v: v["bar"] == "routed")
w.tor.stop()
time.sleep(6)
w.tor.start()
w.until(lambda v: v["bar"] == "routed", 6)
check(w.proc.poll() is None, "an Omarchy without the notifier is no crash")
w.close()

print("Bridges")
w = World()
w.until(lambda v: v["installed"])
# A transport this machine cannot run is shown, and cannot be applied.
w.knob("clipboard", "webtunnel [2001:db8::1]:443 CCCC url=https://example.com/x\n")
w.send({"cmd": "watchers", "count": 1})
w.send({"cmd": "pasteBridges"})
check(w.until(lambda v: v["bridges"]["hasPreview"] and not v["bridges"]["canApply"]), "bridges needing a missing transport cannot be applied")
check(w.view["bridges"]["chips"] and not w.view["bridges"]["chips"][0]["available"]
      and "webtunnel" in w.view["bridges"]["chips"][0]["tip"], "and the chip says what to install")
w.send({"cmd": "applyBridges"})
time.sleep(0.5)
check(w.logged("helper") == [], "Use these does nothing for them")
w.send({"cmd": "cancelBridges"})
check(w.until(lambda v: not v["bridges"]["hasPreview"]), "Cancel drops the preview")
# Plain bridges need no transport, so any machine can apply them.
lines = "192.0.2.1:443 AAAA\nBridge 192.0.2.2:443 BBBB\n"
w.knob("clipboard", lines)
w.send({"cmd": "watchers", "count": 1})
w.send({"cmd": "pasteBridges"})
check(w.until(lambda v: v["bridges"]["hasPreview"]), "a paste is previewed before anything changes")
v = w.view["bridges"]
check(v["previewText"] == "Pasted · 2 bridges", "as a count")
check("192.0.2" not in json.dumps(w.view), "and nothing of the lines themselves reaches the panel")
check(w.logged("helper") == [], "previewing runs nothing as root")
w.send({"cmd": "applyBridges"})
check(w.until(lambda v: not v["bridges"]["hasPreview"] and w.logged("helper bridge set")), "Use these hands them to the helper")
check(w.logged("helper bridge set") == ["helper bridge set"], "with no lines on its command line")
# (The recorder's $(cat) drops the final newline.)
check(w.logged("stdin") == ["stdin 192.0.2.1:443 AAAA|192.0.2.2:443 BBBB"],
      "the lines go over stdin, normalised")
w.close()

w = World()
w.until(lambda v: v["installed"])
w.knob("clipboard", "a" * 70000)
w.send({"cmd": "pasteBridges"})
check(w.until(lambda v: "Too much" in v["bridges"]["message"]["full"]), "a clipboard over the limit is refused")
w.knob("clipboard", "this is just prose\n")
w.send({"cmd": "pasteBridges"})
check(w.until(lambda v: "No bridge lines" in v["bridges"]["message"]["full"]), "prose is not bridge lines")
w.knob("clipboard-exit", 4)
w.send({"cmd": "pasteBridges"})
check(w.until(lambda v: "Could not read the clipboard" in v["bridges"]["message"]["full"]), "a clipboard that fails says so")
w.knob("clipboard-exit")
w.knob("clipboard", "obfs4 192.0.2.1:443 AAAA cert=x\n")
w.send({"cmd": "watchers", "count": 1})
w.send({"cmd": "pasteBridges"})
w.until(lambda v: v["bridges"]["hasPreview"])
w.send({"cmd": "watchers", "count": 0})
check(w.until(lambda v: not v["bridges"]["hasPreview"]), "closing the panel drops a pending paste")
w.send({"cmd": "watchers", "count": 2})
w.send({"cmd": "pasteBridges"})
w.until(lambda v: v["bridges"]["hasPreview"])
w.send({"cmd": "watchers", "count": 1})
time.sleep(0.5)
check(w.view["bridges"]["hasPreview"], "with two panels open, closing one keeps the other's paste")
# A clipboard that answers late, after the paste it answers was given up.
w.send({"cmd": "cancelBridges"})
w.until(lambda v: not v["bridges"]["hasPreview"])
w.knob("clipboard-delay", 1)
w.send({"cmd": "pasteBridges"})
w.until(lambda v: v["bridges"]["working"])
w.send({"cmd": "watchers", "count": 0})
time.sleep(1.8)
w.send({"cmd": "watchers", "count": 1})
time.sleep(0.4)
check(not w.view["bridges"]["hasPreview"] and not w.view["bridges"]["working"],
      "a paste still being read when the panel closes does not come back when it reopens")
w.send({"cmd": "pasteBridges"})
w.until(lambda v: v["bridges"]["working"])
w.send({"cmd": "cancelBridges"})
time.sleep(1.8)
check(not w.view["bridges"]["hasPreview"], "nor one canceled, or left by changing section, while it was being read")
w.send({"cmd": "watchers", "count": 2})
w.send({"cmd": "pasteBridges"})
w.until(lambda v: v["bridges"]["working"])
w.send({"cmd": "watchers", "count": 1})
check(w.until(lambda v: v["bridges"]["hasPreview"]), "but with a panel still open, the late paste arrives")
w.knob("clipboard-delay", 2)
for _ in range(2):
    w.send({"cmd": "pasteBridges"})
    w.until(lambda v: v["bridges"]["working"])
    w.send({"cmd": "cancelBridges"})
w.send({"cmd": "pasteBridges"})
check(w.until(lambda v: "Still reading the clipboard" in v["bridges"]["message"]["full"]),
      "cancel and paste again cannot pile up clipboard reads: a third waits for the first two")
time.sleep(2.5)
w.knob("clipboard-delay")
w.send({"cmd": "pasteBridges"})
check(w.until(lambda v: v["bridges"]["hasPreview"]), "and once they have answered, pasting works again")
w.knob("clipboard-delay")
w.close()

print("New circuit")
w = World()
w.until(lambda v: v["installed"])
w.set_rules(True)
w.until(lambda v: v["canNewCircuit"])
w.send({"cmd": "newCircuit"})
check(w.until(lambda v: w.tor.newnyms == 1), "NEWNYM goes over the control session")
check(w.logged("helper") == [], "with no helper and no password")
w.close()

print("Not installed")
w = World(tor_up=False)
os.remove(os.path.join(w.root, "etc/tor/torrc.d/00-tormarchy.conf"))
check(w.until(lambda v: not v["installed"]), "no tor config is not installed")
w.send({"cmd": "toggle"})
time.sleep(0.5)
check(w.logged("helper") == [], "and the switch does nothing")
w.close()

print()
print("  the daemon behaves" if failures == 0 else f"  {failures} daemon check(s) failed")
sys.exit(1 if failures else 0)
