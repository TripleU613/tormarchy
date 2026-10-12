// Model.js holds the parsing and formatting, with no Qt in it, precisely so it
// can be tested without a compositor. This runs anywhere node does.
//
// qsTr only exists inside QML, so it is stubbed here. The stub returns a String
// object carrying .arg(), which is what qsTr("%1 ms").arg(n) needs.

const fs = require("fs");
const path = require("path");

function tr(s) {
  const wrapped = new String(s);
  wrapped.arg = function (v) { return tr(String(this).replace(/%\d/, v)); };
  return wrapped;
}
global.qsTr = tr;
String.prototype.arg = function (v) { return tr(String(this).replace(/%\d/, v)); };

eval(fs.readFileSync(path.join(__dirname, "..", "Model.js"), "utf8"));

let failures = 0;
function is(actual, expected, what) {
  const a = String(actual);
  if (a === String(expected)) {
    console.log(`  ok    ${what}`);
  } else {
    console.log(`  FAIL  ${what}: got ${JSON.stringify(a)}, wanted ${JSON.stringify(String(expected))}`);
    failures++;
  }
}

// Latency. Zero is not a measurement, so it must not read as one -- a dash says
// nothing has been measured, "0 ms" claims a round trip of zero.
is(formatLatency(0), "— ms", "unmeasured latency shows a dash");
is(formatLatency(842), "842 ms", "sub-second latency in ms");
is(formatLatency(2400), "2.4 s", "over a second switches to seconds");
is(latencyVerdict(0), "", "no verdict without a measurement");
is(latencyVerdict(300), "Fast for Tor", "300ms is fast for Tor");
is(latencyVerdict(842), "Normal for Tor", "842ms is normal for Tor");
is(latencyFraction(0).toFixed(2), "0.00", "gauge empty when unmeasured");
is(latencyFraction(3000).toFixed(2), "1.00", "gauge pins at the 3s ceiling");
is(latencyFraction(9999).toFixed(2), "1.00", "gauge clamps past the ceiling");

// Countries. Tor reports codes; showing "de" leaks implementation detail.
is(countryName("de"), "Germany", "known country code resolves to a name");
is(countryName("zz"), "ZZ", "unknown code falls back to the code itself");
is(countryName(""), "", "empty code stays empty");
is(exitLabel(""), "Automatic", "no pinned exit reads as Automatic");
is(exitLabel("nl"), "Netherlands", "a pinned exit shows the country");

// Modes. The wire values stay lan/strict/socks whatever the labels say.
is(MODE_ORDER.join(","), "strict,lan,socks", "modes are ordered most protective first");
is(modeLabel("strict"), "Maximum", "strict is labelled Maximum");
is(modeLabel("lan"), "Standard", "lan is labelled Standard");
is(modeLabel("socks"), "Browser only", "socks is labelled Browser only");
is(isValidMode("lan"), true, "lan is a valid mode");
is(isValidMode("nonsense"), false, "junk is not a valid mode");

// Circuit path. "xx" is Tor's own answer for a relay it cannot place.
is(pathHops(["de", "nl", "us"]).join(" "), "DE NL US", "hops upper-case");
is(pathHops(["de", "xx", ""]).join(" "), "DE ?? ??", "unplaceable hops show ??");
is(hopRole(0, 3), "Guard", "first hop is the guard");
is(hopRole(1, 3), "Middle", "middle hop");
is(hopRole(2, 3), "Exit", "last hop is the exit");

// Traffic counters arrive as raw byte totals.
is(formatBytes(0), "0 B", "zero bytes");
is(formatBytes(512), "512 B", "bytes stay whole");
is(formatBytes(4396728), "4.2 MB", "megabytes get one decimal");

// Status parsing has to fail loudly. A widget showing a stale "connected" is
// worse than one admitting it does not know.
is(parseStatus("").ok, false, "empty status is not ok");
is(parseStatus("not json").ok, false, "unparseable status is not ok");
const good = parseStatus(JSON.stringify({
  installed: true, torRunning: true, connected: true, bootstrap: 100,
  mode: "lan", exitCountry: "de", path: ["it", "nl", "de"],
  bytesRead: 1024, circuitAgeSec: 90, statusText: "ok",
}));
is(good.ok, true, "well-formed status parses");
is(good.exitCountryName, "Germany", "exit country name is derived, not trusted");
is(good.path.join(","), "it,nl,de", "path survives parsing");
is(formatAge(90), "1m 30s", "age formats as minutes and seconds");

// The control port's reachability. Without it the bootstrap figure is a
// placeholder, and an older helper that never sent the field only ever reported
// a reachable port.
is(good.controlPort, true, "an older status with no controlPort field reads as reachable");
is(parseStatus(JSON.stringify({ installed: true, torRunning: true, controlPort: false })).controlPort,
  false, "an unreachable control port is carried through");

// Status fields added with the panel's tools. Absent from older helpers, and
// each must then read as "nothing", not as garbage or a crash.
const tools = parseStatus(JSON.stringify({
  installed: true, connected: true, torRunning: true, bridges: "stashed",
  excludedExits: ["US", "ru", "us", "nope", 4],
  hops: [{ cc: "SE", nickname: "guardOne", flags: ["Fast", "Guard"], bandwidthKBps: 31200 }, null]
}));
is(tools.bridges, "stashed", "the bridge state is carried through");
is(tools.excludedExits.join(","), "us,ru", "excluded exits are lowercased, deduplicated and validated");
is(tools.hops[0].cc + " " + tools.hops[0].nickname + " " + tools.hops[0].flags.join("+"), "se guardOne Fast+Guard",
  "a hop's relay details survive parsing");
is(tools.hops[1].nickname + "|" + tools.hops[1].bandwidthKBps, "|0", "a null hop parses as an empty one");
const older = parseStatus(JSON.stringify({ installed: true }));
is(older.bridges, "undefined", "an older helper's missing bridge state is left undefined, not read as off");
is(older.hops.length + older.excludedExits.length, 0, "and its missing hops and exclusions are empty");

// Hop tooltips: role, place, name, speed, and the flags that are not the role.
is(hopTooltip(tools.hops[0], 0, 3), "Guard · Sweden · guardOne · 31 MB/s · Fast",
  "a guard's tooltip names it and leaves out the Guard flag it already says");
is(hopTooltip({ cc: "nl", nickname: "m", flags: ["Fast", "Guard", "Stable"], bandwidthKBps: 840 }, 1, 3),
  "Middle · Netherlands · m · 840 KB/s · Fast, Guard, Stable", "a middle relay keeps a Guard flag: it is news there");
is(hopTooltip(null, 2, 3, "de"), "Exit · Germany", "with no relay details the tooltip still says role and place");
is(formatRelayBandwidth(5400), "5.4 MB/s", "relay bandwidth under 10 MB/s keeps a decimal");
is(formatRelayBandwidth(0), "", "an unmeasured relay shows no bandwidth");

// The exit picker. Recent picks first, excluded countries never.
const picker = exitOptions(["se", "de", "nl", "us", "ch"], ["nl", "fr", "de", "nl"], ["us"]);
is(picker.map(o => o.value).join(","), "auto,nl,de,se,ch",
  "recent countries come right after Automatic, then the rest alphabetically, without the excluded");
is(picker[1].description, "Recent", "a recent country says so");
is(picker.filter(o => o.value === "nl").length, 1, "a recent country is not listed twice");
is(picker[0].label, "Not US", "with exclusions, Automatic is labelled with them -- the trigger shows that label");
is(exitOptions(["de"], [], [])[0].label, "Automatic", "without exclusions it is just Automatic");
is(exitLabel("", ["us", "ru"]), "Not US, RU", "on Automatic the picker reads as the exclusions");
is(exitLabel("de", ["us"]), "Germany", "a pinned exit makes exclusions moot, so it shows only the country");

// The leak check.
const passing = parseDoctor(fs.readFileSync(path.join(__dirname, "qml", "fixtures", "doctor-pass.json"), "utf8"));
const failing = parseDoctor(fs.readFileSync(path.join(__dirname, "qml", "fixtures", "doctor-fail.json"), "utf8"));
is(passing.ok && passing.healthy, true, "a clean report parses as healthy");
is(doctorOverall(passing), "pass", "and its overall result is a pass");
is(doctorOverall(failing), "fail", "any failure makes the whole report a failure");
is(/problem/.test(doctorSummary(failing)), true, "the summary leads with the problems");
is(parseDoctor('{"checks":[{"status":"maybe","text":"x"}]}').checks[0].status, "skip",
  "a status this panel does not know renders as skipped, never as a pass");
is(parseDoctor("").ok, false, "an empty answer is an error, not a clean report");
is(parseDoctor("{nope").ok, false, "neither is an unreadable one");
is(doctorSummary({ ok: true, checks: [{ status: "skip" }] }), "Nothing to check", "all skipped says so");
is(formatAgo(30), "just now", "under a minute is just now");
is(formatAgo(150), "2 min ago", "minutes ago");

// Bridges, as the tile and the paste preview state them.
const check = parseBridgeReport(fs.readFileSync(path.join(__dirname, "qml", "fixtures", "bridge-check-missing.json"), "utf8"));
is(bridgeCountText(check), "3 bridges", "the preview counts the paste; its transports are chips beneath");
is(missingTransports(check).map(t => t.name).join(","), "obfs4,webtunnel", "transports this machine lacks are named");
is(bridgeCountText(parseBridgeReport('{"count":0,"transports":[]}')), "No bridge lines", "an empty paste says so");
is(parseBridgeReport("not json").ok, false, "unreadable bridge output is not a report");
is(parseBridgeReport('{"state":"weird"}').state, "", "an unknown bridge state is dropped");
is(bridgeStateLabel("active", { count: 2, transports: [{ name: "obfs4" }] }), "On · 2 obfs4", "one transport is named on the tile");
is(bridgeStateLabel("active", { count: 3, transports: [{ name: "obfs4" }, { name: "webtunnel" }] }), "On · 3 bridges",
  "several are counted, to fit a third of the panel");
is(bridgeStateLabel("stashed", { count: 3, transports: [] }), "Off · 3 saved", "saved bridges that are off say so");
is(bridgeStateLabel("off", null), "Off", "no bridges is just off");

// Notifications. The policy decides what is worth a toast; "problems", the
// default, speaks only when something has gone wrong or come back.
is(notificationPolicy("nonsense"), "problems", "an unknown policy falls back to problems");
is(connectionState({ installed: true, connected: true, torRunning: false }), "blocked", "rules with no tor is the kill switch");
is(connectionState({ installed: true, connected: true, torRunning: true }), "routed", "rules with tor is routed");
is(connectionState({ installed: true, mode: "socks", torRunning: true, bootstrapping: false }), "browser", "browser-only, ready");
is(connectionState({ installed: false, connected: true }), "off", "not installed is off whatever else it says");
const head = n => (n ? String(n.headline) : "none");
for (const policy of ["problems", "all"]) {
  is(head(notificationFor("routed", "blocked", policy)), "Kill switch engaged", `${policy}: the kill switch is announced`);
  is(notificationFor("routed", "blocked", policy).urgency, "critical", `${policy}: and it is critical`);
  is(head(notificationFor("blocked", "routed", policy)), "Tor is back", `${policy}: recovery is announced`);
  is(head(notificationFor("blocked", "off", policy)), "Network restored", `${policy}: turning off out of it is announced`);
  is(head(notificationFor("routed", "off", policy)), "none", `${policy}: an ordinary disconnect is not`);
  is(head(notificationFor("", "blocked", policy)), "none", `${policy}: the first reading is a baseline, never a toast`);
  is(head(notificationFor("blocked", "blocked", policy)), "none", `${policy}: no change, no toast`);
}
is(head(notificationFor("connecting", "routed", "problems")), "none", "problems: a connect succeeding is not announced");
is(head(notificationFor("connecting", "routed", "all", { exitCountryName: "Germany" })), "Connected through Tor",
  "all: a connect succeeding is");
is(String(notificationFor("connecting", "routed", "all", { exitCountryName: "Germany" }).body), "Leaving from Germany",
  "with where it leaves from");
is(head(notificationFor("connecting", "browser", "all")), "Tor is ready", "all: browser-only being ready is announced");
is(head(notificationFor("routed", "blocked", "off")), "none", "off: not even the kill switch");
is(head(connectFailedNotification("tor stalled at 40%", "problems")), "Couldn't connect to Tor", "a failed connect is a problem");
is(head(connectFailedNotification("x", "off")), "none", "but not when notifications are off");
is(head(bridgesNotification("off", "active", "all", false)), "Using bridges", "all: connect's bridge fallback is announced");
is(head(bridgesNotification("off", "active", "all", true)), "none", "switching bridges on yourself is not");
is(head(bridgesNotification("off", "active", "problems", false)), "none", "problems: the fallback is not a problem");
const argv = notificationArgv(notificationFor("routed", "blocked", "problems"), "7");
is(argv[0], "omarchy-notification-send", "toasts go through Omarchy's sender");
is(argv.indexOf("notify-send"), -1, "never notify-send");
is(argv.slice(argv.indexOf("-r"), argv.indexOf("-r") + 2).join(" "), "-r 7", "a toast id replaces the previous toast");
is(argv.slice(-4).join(" "), "--exec omarchy-shell tripleu.tor open", "clicking a toast opens the panel, and --exec is last");
is(notificationArgv({ headline: "h", glyph: "g" }, "7; rm -rf /").indexOf("-r"), -1, "a replace id that is not a number is dropped");

// Copy. The panel shows labels; a message from the helper is cut to its first
// clause for the panel and wrapped whole for the bubble.
is(brief("tormarchy: tor stalled at 40% bootstrap. No rules applied, networking is unchanged."),
  "Tor stalled at 40% bootstrap", "a helper error is cut to its first clause");
is(brief("could not reach tor's control port on 127.0.0.1:9051: Permission denied").length <= 32, true,
  "and never past a label's length");
is(brief("could not reach tor's control port on 127.0.0.1:9051: Permission denied").slice(-1), "\u2026",
  "with an ellipsis where it was cut");
is(brief(""), "", "nothing stays nothing");
is(briefWarning("Tor has stopped, so the kill switch is holding the network shut. Turn Tormarchy off to get it back."),
  "Kill switch holding", "the kill-switch warning is a label");
is(briefWarning("Tor is still bootstrapping at 40%."), "Reconnecting \u00b7 40%", "and so is a reconnect");
const wrapped = wrapTip("Asks check.torproject.org whether you are coming from Tor, tries an IPv6 host directly, and checks DNS.");
is(wrapped.split("\n").every(l => l.length <= 46), true, "a bubble's text is wrapped to lines it can hold");
is(wrapped.replace(/\n/g, " "), "Asks check.torproject.org whether you are coming from Tor, tries an IPv6 host directly, and checks DNS.",
  "without losing a word");
is(checkLabel({ id: "ipv6-stack", text: "IPv6 stack is disabled in the kernel as well" }), "IPv6 stack",
  "a leak check is named in a word");
is(checkLabel({ id: "", text: "something new the panel does not know" }).length <= 16, true,
  "an unknown check still gets a short name");
is(checkTip({ text: "check.torproject.org confirms this connection is Tor", detail: "exit as seen from outside: 1.2.3.4" }).indexOf("1.2.3.4") > 0,
  true, "and its sentence and detail are its bubble");
is(parseDoctor(fs.readFileSync(path.join(__dirname, "qml", "fixtures", "doctor-pass.json"), "utf8")).checks[0].id, "tor",
  "the check's id survives parsing");
for (const n of [notificationFor("routed", "blocked", "problems"), notificationFor("blocked", "routed", "problems"),
                 connectFailedNotification("tor stalled at 40% bootstrap. No rules applied, networking is unchanged.", "problems")]) {
  is(String(n.body).length <= 40, true, `the "${n.headline}" toast says it in a few words`);
}

console.log(failures === 0
  ? "  ok    Model.js behaves"
  : `  ${failures} assertion(s) failed`);
process.exit(failures === 0 ? 0 : 1);
