// Tor's rows in the Omarchy menu, read by the menu's own parser.
//
//   node tests/menu.js ~/src/omarchy
//
// `tormarchy menu add` edits a file Omarchy owns the format of, so the only
// check worth having is Omarchy's: MenuModel.js from the checkout given, the
// same code the menu runs, parsing the file the way the menu parses it. A file
// it cannot parse costs the user every entry of their own, silently.

const fs = require("fs");
const os = require("os");
const path = require("path");
const { spawnSync } = require("child_process");

const omarchy = path.resolve(process.argv[2] || "/usr/share/omarchy");
const repo = path.resolve(__dirname, "..");
const model = path.join(omarchy, "shell", "plugins", "menu", "MenuModel.js");
eval(fs.readFileSync(model, "utf8"));

let failures = 0;
function check(ok, what) {
  console.log((ok ? "  ok    " : "  FAIL  ") + what);
  if (!ok) failures++;
}

// The menu functions, run without the dispatcher and without the root check,
// against a home directory of the test's own.
function tormarchy(home, verb) {
  const script = `source <(sed -n '/^# -* constants --$/,/^# -* dispatch --$/p' tormarchy); menu_write ${verb}`;
  return spawnSync("bash", ["-c", script], { cwd: repo, env: { ...process.env, HOME: home }, encoding: "utf8" });
}

function home() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "tormarchy-menu-"));
  fs.mkdirSync(path.join(dir, ".config", "omarchy", "extensions"), { recursive: true });
  return dir;
}
const fileIn = (dir) => path.join(dir, ".config", "omarchy", "extensions", "omarchy-menu.jsonc");

const defaults = parseMenuJsonc(fs.readFileSync(path.join(omarchy, "default", "omarchy", "omarchy-menu.jsonc"), "utf8"));
const ours = ["trigger.toggle.tor", "trigger.tor", "trigger.tor.connect", "trigger.tor.disconnect",
  "trigger.tor.new-circuit", "trigger.tor.panel", "trigger.tor.leak-check", "trigger.tor.bridges"];

function merged(dir) {
  return mergeMenuSources(defaults, parseMenuJsonc(fs.readFileSync(fileIn(dir), "utf8"))).items;
}

// The sample Omarchy installs into every home: braces around comments.
const sample = path.join(omarchy, "config", "omarchy", "extensions", "omarchy-menu.jsonc");
let dir = home();
fs.copyFileSync(sample, fileIn(dir));
let run = tormarchy(dir, "add");
check(run.status === 0, "adds to the sample extension file Omarchy installs" + (run.stderr ? ` (${run.stderr.trim()})` : ""));
let items = merged(dir);
check(ours.every((id) => items[id]), "every Tor row parses, with the menu's own parser");
check(items["trigger.toggle.tor"] && items["trigger.toggle.tor"].parent === "trigger.toggle" && items["trigger.toggle"],
  "the switch sits in Trigger > Toggle, beside Omarchy's own toggles");
check(items["trigger.tor"].parent === "trigger" && items["trigger.tor.connect"].parent === "trigger.tor",
  "the rest is a Tor submenu under Trigger");
check(Object.keys(items).length === defaults.length + ours.length + (defaults.some((d) => d.id === "root") ? 0 : 1),
  "and nothing of Omarchy's own menu is lost");
const guards = guardScript(items);
check(spawnSync("bash", ["-n"], { input: guards }).status === 0, "the menu's batched checks are still valid bash");
fs.rmSync(dir, { recursive: true });

// Someone's own rows, and a second add.
dir = home();
const own = '{\n  "personal": {"icon":"","label":"Personal"},\n  "personal.notes": {"icon":"󰎞","label":"Notes","action":"omarchy-launch-editor ~/notes"}\n}\n';
fs.writeFileSync(fileIn(dir), own);
tormarchy(dir, "add");
tormarchy(dir, "add");
items = merged(dir);
check(items["personal.notes"] && items["trigger.tor"], "their own rows and Tor's parse side by side");
check(fs.readFileSync(fileIn(dir), "utf8").split("trigger.tor.panel").length === 2, "adding twice leaves one block, not two");
run = tormarchy(dir, "remove");
check(run.status === 0 && fs.readFileSync(fileIn(dir), "utf8") === own, "remove puts the file back byte for byte");
fs.rmSync(dir, { recursive: true });

// A dotfiles setup: the file is a symlink into a repository.
dir = home();
const real = path.join(dir, "dotfiles-menu.jsonc");
fs.writeFileSync(real, own);
fs.symlinkSync(real, fileIn(dir));
tormarchy(dir, "add");
check(fs.lstatSync(fileIn(dir)).isSymbolicLink() && fs.readFileSync(real, "utf8").includes("trigger.tor"),
  "a symlinked file stays a symlink, and the file it points at gets the rows");
fs.rmSync(dir, { recursive: true });

// A file the menu cannot parse already: leave it for its owner to fix.
dir = home();
const broken = '{\n  "personal": {"label":"Personal"} "oops"\n}\n';
fs.writeFileSync(fileIn(dir), broken);
run = tormarchy(dir, "add");
check(run.status !== 0 && fs.readFileSync(fileIn(dir), "utf8") === broken, "a file that does not parse is refused, untouched");
fs.rmSync(dir, { recursive: true });

// No file at all.
dir = home();
tormarchy(dir, "add");
check(merged(dir)["trigger.tor"] !== undefined, "with no extension file yet, one is made");
fs.rmSync(dir, { recursive: true });

console.log(failures === 0 ? "  menu entries parse as the menu reads them" : `  ${failures} menu check(s) failed`);
process.exit(failures === 0 ? 0 : 1);
