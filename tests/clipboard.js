// The clipboard read behind Paste bridges, run for real against a fake wl-paste.
//
//   node tests/clipboard.js
//
// The clipboard is sent by whichever app copied last, as much of it and as
// slowly as it likes, into the process that runs the whole bar. Model.js caps
// the read; this runs that exact argv through bash with a wl-paste that sends
// too much, never stops, or stalls, and checks the shell is only ever handed
// what fits under the helper's limit.

const fs = require("fs");
const os = require("os");
const path = require("path");
const { spawnSync } = require("child_process");

global.qsTr = (s) => s;
eval(fs.readFileSync(path.join(__dirname, "..", "Model.js"), "utf8"));

let failures = 0;
function check(ok, what) {
  console.log((ok ? "  ok    " : "  FAIL  ") + what);
  if (!ok) failures++;
}

const dir = fs.mkdtempSync(path.join(os.tmpdir(), "tormarchy-clipboard-"));
fs.writeFileSync(path.join(dir, "wl-paste"), `#!/usr/bin/env bash
case $FAKE_CLIPBOARD in
  file)    cat "$FAKE_CLIPBOARD_FILE" ;;
  endless) yes 'obfs4 192.0.2.1:443 0123456789ABCDEF0123456789ABCDEF01234567 cert=abc iat-mode=0' ;;
  stall)   printf 'obfs4 192.0.2.1:443 0123456789ABCDEF0123456789ABCDEF01234567\\n'; exec sleep 30 ;;
  nothing) echo "Nothing is copied" >&2; exit 1 ;;
  broken)  exit 2 ;;
esac
`, { mode: 0o755 });
const clip = path.join(dir, "clipboard");

function paste(mode, content) {
  if (content !== undefined) fs.writeFileSync(clip, content);
  const argv = clipboardArgv(1);
  const started = Date.now();
  const run = spawnSync(argv[0], argv.slice(1), {
    env: { ...process.env, PATH: `${dir}:${process.env.PATH}`, FAKE_CLIPBOARD: mode, FAKE_CLIPBOARD_FILE: clip },
    encoding: "latin1", timeout: 20000, maxBuffer: 4 * BRIDGE_INPUT_LIMIT,
  });
  return { code: run.status, out: run.stdout, seconds: (Date.now() - started) / 1000 };
}

const lines = fs.readFileSync(path.join(__dirname, "qml", "fixtures", "bridges.txt"), "latin1");
let r = paste("file", lines);
check(r.code === 0 && r.out === lines, "bridge lines come through byte for byte, trailing newline and all");

r = paste("file", "a".repeat(BRIDGE_INPUT_LIMIT));
check(r.code === 0 && r.out.length === BRIDGE_INPUT_LIMIT, "exactly the limit is accepted, whole");

r = paste("file", "a".repeat(BRIDGE_INPUT_LIMIT + 1));
check(r.code === 3 && r.out === "", "one byte over is refused, and hands over nothing");

// A newline landing just past the limit, which command substitution would strip
// and so make a cut-off paste look like one that fit.
r = paste("file", "a".repeat(BRIDGE_INPUT_LIMIT) + "\n" + "b".repeat(100));
check(r.code === 3 && r.out === "", "a newline right at the cut does not make a cut-off paste look whole");

r = paste("endless");
check(r.code === 3 && r.out === "" && r.seconds < 5,
  `a clipboard that never stops is cut off at the limit and refused (${r.seconds}s)`);

r = paste("stall");
check(r.code === 4 && r.out === "" && r.seconds < 5,
  `a clipboard that stalls is given up on, and its partial lines are not used (${r.seconds}s)`);

r = paste("nothing");
check(r.code === 0 && r.out === "", "nothing copied is an empty paste");

r = paste("broken");
check(r.code === 4 && r.out === "", "wl-paste failing is not mistaken for an empty clipboard");

fs.rmSync(dir, { recursive: true });
console.log(failures === 0 ? "  the shell never holds more than the limit" : `  ${failures} clipboard check(s) failed`);
process.exit(failures === 0 ? 0 : 1);
