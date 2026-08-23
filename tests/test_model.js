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

console.log(failures === 0
  ? "  ok    Model.js behaves"
  : `  ${failures} assertion(s) failed`);
process.exit(failures === 0 ? 0 : 1);
