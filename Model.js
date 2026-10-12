// Parsing and label helpers for the Tormarchy widget. Kept out of Service.qml
// so the state machine stays readable and this stays unit-testable by eye.

var MODES = ["lan", "strict", "socks"]

// Order the mode row is drawn in: most protective to least, which is not the
// order MODES happens to be in. Kept separate so the wire values never move.
var MODE_ORDER = ["strict", "lan", "socks"]

function modeLabel(mode) {
  switch (String(mode || "")) {
  case "lan": return qsTr("Standard")
  case "strict": return qsTr("Maximum")
  case "socks": return qsTr("Browser only")
  default: return "—"
  }
}

function modeTooltip(mode) {
  switch (String(mode || "")) {
  case "lan": return qsTr("All internet traffic through Tor. Local network still reachable.")
  case "strict": return qsTr("All traffic through Tor. Local network dropped too.")
  case "socks": return qsTr("Nothing is routed system-wide. Any app that speaks SOCKS can opt in on 127.0.0.1:9050 — a browser is just the usual one.")
  default: return ""
  }
}

function isValidMode(mode) {
  return MODES.indexOf(String(mode || "")) !== -1
}

// tormarchy-status --json emits a single object. Anything unparseable is a
// failure we surface rather than guess around: a Tor widget that silently
// shows a stale "connected" is worse than one that admits it doesn't know.
function parseStatus(raw) {
  var text = String(raw || "").trim()
  if (text === "") return { ok: false, lastError: qsTr("tormarchy-status returned nothing") }

  var data = null
  try {
    data = JSON.parse(text)
  } catch (e) {
    return { ok: false, lastError: qsTr("Could not parse tormarchy-status output") }
  }
  if (!data || typeof data !== "object") {
    return { ok: false, lastError: qsTr("Unexpected tormarchy-status output") }
  }

  return {
    ok: true,
    installed: data.installed === true,
    torRunning: data.torRunning === true,
    connected: data.connected === true,
    bootstrap: clampPercent(data.bootstrap),
    // Absent from older helpers, which only ever reported a reachable port.
    controlPort: data.controlPort !== false,
    mode: isValidMode(data.mode) ? String(data.mode) : "",
    exitIp: String(data.exitIp || ""),
    exitCountry: String(data.exitCountry || ""),
    // Bash emits the code; the display name is a JS-side concern.
    exitCountryName: countryName(data.exitCountry),
    requestedExit: String(data.requestedExit || ""),
    circuitAgeSec: Number(data.circuitAgeSec || 0),
    path: Array.isArray(data.path) ? data.path : [],
    bytesRead: Number(data.bytesRead || 0),
    bytesWritten: Number(data.bytesWritten || 0),
    statusText: String(data.statusText || ""),
    // Absent from helpers older than the bridge tool; left undefined then, so
    // the panel keeps what it had rather than reading every bridge as off.
    bridges: ["active", "stashed", "off"].indexOf(String(data.bridges)) !== -1 ? String(data.bridges) : undefined,
    warnings: Array.isArray(data.warnings) ? data.warnings : []
  }
}

function clampPercent(value) {
  var n = Number(value)
  if (!isFinite(n)) return 0
  if (n < 0) return 0
  if (n > 100) return 100
  return Math.round(n)
}

// Short label for the bar itself, where horizontal space is scarce: the exit
// country when we have one, the bootstrap percentage while it's coming up.

function stateLabel(state) {
  if (!state) return qsTr("Unknown")
  if (!state.installed) return qsTr("Not installed")
  if (state.connected) return qsTr("Connected")
  if (state.bootstrapping) return qsTr("Connecting")
  return qsTr("Disconnected")
}

// Durations go through qsTr with placeholders rather than string addition:
// the unit letters are language-specific, and some languages need the parts in
// the other order. A translator can reorder %1 and %2 without touching code.
function formatAge(seconds) {
  var n = Math.max(0, Math.floor(Number(seconds) || 0))
  if (n < 60) return qsTr("%1s").arg(n)
  if (n < 3600) return qsTr("%1m %2s").arg(Math.floor(n / 60)).arg(n % 60)
  return qsTr("%1h %2m").arg(Math.floor(n / 3600)).arg(Math.floor((n % 3600) / 60))
}

function elide(text, limit) {
  var max = Number(limit) || 140
  var value = String(text || "").replace(/\s+/g, " ").trim()
  return value.length > max ? value.substring(0, max - 3) + "…" : value
}

// Exit-country names. Tor reports two-letter codes; a bar panel showing "de"
// where it could show "Germany" is just leaking implementation detail at the
// user. Covers the countries that realistically host exit relays.
//
// NOT wrapped in qsTr: these are a data table, and Qt has no code-to-localised
// -name mapping to lean on (Qt.locale() needs a language before it will name a
// country, and would name it in that country's own language, not the reader's).
// Localising them means a translated catalogue keyed on the code -- a
// translation job, not a code change. English names until then.
var COUNTRY_NAMES = {
  ad: "Andorra", ae: "United Arab Emirates", al: "Albania", am: "Armenia",
  ar: "Argentina", at: "Austria", au: "Australia", az: "Azerbaijan",
  ba: "Bosnia and Herzegovina", bd: "Bangladesh", be: "Belgium",
  bg: "Bulgaria", bh: "Bahrain", bo: "Bolivia", br: "Brazil", by: "Belarus",
  ca: "Canada", ch: "Switzerland", cl: "Chile", cn: "China", co: "Colombia",
  cr: "Costa Rica", cy: "Cyprus", cz: "Czechia", de: "Germany",
  dk: "Denmark", do: "Dominican Republic", dz: "Algeria", ec: "Ecuador",
  ee: "Estonia", eg: "Egypt", es: "Spain", et: "Ethiopia", fi: "Finland",
  fr: "France", gb: "United Kingdom", ge: "Georgia", gh: "Ghana",
  gr: "Greece", gt: "Guatemala", hk: "Hong Kong", hn: "Honduras",
  hr: "Croatia", hu: "Hungary", id: "Indonesia", ie: "Ireland",
  il: "Israel", in: "India", iq: "Iraq", ir: "Iran", is: "Iceland",
  it: "Italy", jm: "Jamaica", jo: "Jordan", jp: "Japan", ke: "Kenya",
  kg: "Kyrgyzstan", kh: "Cambodia", kr: "South Korea", kw: "Kuwait",
  kz: "Kazakhstan", lb: "Lebanon", li: "Liechtenstein", lk: "Sri Lanka",
  lt: "Lithuania", lu: "Luxembourg", lv: "Latvia", ma: "Morocco",
  md: "Moldova", me: "Montenegro", mk: "North Macedonia", mn: "Mongolia",
  mt: "Malta", mu: "Mauritius", mx: "Mexico", my: "Malaysia",
  mz: "Mozambique", ng: "Nigeria", nl: "Netherlands", no: "Norway",
  np: "Nepal", nz: "New Zealand", om: "Oman", pa: "Panama", pe: "Peru",
  ph: "Philippines", pk: "Pakistan", pl: "Poland", pr: "Puerto Rico",
  pt: "Portugal", py: "Paraguay", qa: "Qatar", ro: "Romania",
  rs: "Serbia", ru: "Russia", sa: "Saudi Arabia", se: "Sweden",
  sg: "Singapore", si: "Slovenia", sk: "Slovakia", sn: "Senegal",
  sv: "El Salvador", th: "Thailand", tn: "Tunisia", tr: "Türkiye",
  tw: "Taiwan", tz: "Tanzania", ua: "Ukraine", ug: "Uganda",
  us: "United States", uy: "Uruguay", uz: "Uzbekistan", ve: "Venezuela",
  vn: "Vietnam", za: "South Africa", zw: "Zimbabwe"
}

function countryName(code) {
  var cc = String(code || "").toLowerCase()
  if (cc === "") return ""
  return COUNTRY_NAMES[cc] || cc.toUpperCase()
}

// Label for the currently pinned exit. "Automatic" is the honest word for an
// unpinned exit: Tor is choosing, we are not.
function exitLabel(requested) {
  var cc = String(requested || "").toLowerCase()
  return cc === "" ? qsTr("Automatic") : countryName(cc)
}

// Dropdown options: Automatic first, then every country Tor reported an exit
// relay in, sorted by the name we actually display rather than by code.
function exitOptions(codes) {
  var options = [{ value: "auto", label: qsTr("Automatic"), description: qsTr("Let Tor choose the exit") }]
  var list = []
  for (var i = 0; i < (codes || []).length; i++) {
    var cc = String(codes[i] || "").toLowerCase()
    if (cc.length !== 2) continue
    list.push({ value: cc, label: countryName(cc) })
  }
  list.sort(function(a, b) { return a.label.localeCompare(b.label) })
  return options.concat(list)
}

// Pinning exits shrinks the relay pool you can leave through, which is worse
// for anonymity than letting Tor pick. Say so where the choice is made.
function exitWarning(requested) {
  if (String(requested || "") === "") return ""
  return qsTr("A pinned exit country narrows your relay pool. Prefer Automatic unless you need a specific location.")
}

// Latency through the circuit. Tor round-trips are hundreds of milliseconds at
// best, so seconds only ever appear when something is wrong -- worth showing
// rather than hiding behind a spinner.
function formatLatency(ms) {
  var n = Number(ms)
  // An em dash, not a zero. "0 ms" is a measurement, and claiming a
  // round-trip of zero while disconnected states something false; a dash says
  // plainly that nothing has been measured.
  if (!isFinite(n) || n <= 0) return qsTr("— ms")
  if (n < 1000) return qsTr("%1 ms").arg(Math.round(n))
  return qsTr("%1 s").arg((n / 1000).toFixed(1))
}

// Where a latency sits on the gauge, 0..1. Anchored at 3000ms because a Tor
// circuit slower than that is effectively unusable, so pinning the needle
// there says more than a scale that keeps stretching.
function latencyFraction(ms) {
  var n = Number(ms)
  if (!isFinite(n) || n <= 0) return 0
  return Math.max(0, Math.min(1, n / 3000))
}

// Plain-language verdict. A number alone does not tell you whether 900ms is
// normal for Tor (it is) or bad (it is not).
function latencyVerdict(ms) {
  var n = Number(ms)
  if (!isFinite(n) || n <= 0) return ""
  if (n < 400) return qsTr("Fast for Tor")
  if (n < 900) return qsTr("Normal for Tor")
  if (n < 2000) return qsTr("Slow — try a new circuit")
  return qsTr("Very slow — try a new circuit")
}

function parseSpeed(raw) {
  var text = String(raw || "").trim()
  if (text === "") return { ok: false }
  try {
    var data = JSON.parse(text)
    return {
      ok: data.ok === true,
      latencyMs: Number(data.latencyMs || 0),
      error: String(data.error || "")
    }
  } catch (e) {
    return { ok: false, error: qsTr("Could not read the speed result") }
  }
}

// Traffic counters come from Tor as raw byte totals for the session.
function formatBytes(bytes) {
  var n = Number(bytes)
  if (!isFinite(n) || n <= 0) return qsTr("0 B")
  var units = [qsTr("B"), qsTr("KB"), qsTr("MB"), qsTr("GB"), qsTr("TB")]
  var i = 0
  while (n >= 1024 && i < units.length - 1) { n /= 1024; i += 1 }
  // Whole numbers for bytes, one decimal above that: "4.2 MB", not "4.196 MB".
  return (i === 0 ? String(Math.round(n)) : n.toFixed(n < 10 ? 1 : 0)) + " " + units[i]
}

// The hops of the current circuit, as display labels. Tor gives lowercase
// country codes and "xx" where it could not place a relay.
function pathHops(path) {
  var hops = []
  for (var i = 0; i < (path || []).length; i++) {
    var cc = String(path[i] || "").toLowerCase()
    hops.push(cc === "" || cc === "xx" ? "??" : cc.toUpperCase())
  }
  return hops
}

// Which hop is which. Tor's own vocabulary, because it is what the docs and
// every other Tor tool use.
function hopRole(index, total) {
  if (total <= 0) return ""
  if (index === 0) return qsTr("Guard")
  if (index === total - 1) return qsTr("Exit")
  return qsTr("Middle")
}

// Trim a latency history to a fixed window, dropping the oldest.
function pushHistory(history, value, limit) {
  var out = (history || []).slice()
  out.push(Number(value) || 0)
  var max = Number(limit) || 16
  while (out.length > max) out.shift()
  return out
}

// The leak check: `tormarchy doctor --json`. A failing check still produces a
// full report (doctor exits non-zero then), so the report is judged on its own
// contents, never on the exit code.
var DOCTOR_STATES = ["pass", "warn", "fail", "skip"]

function parseDoctor(raw) {
  var text = String(raw || "").trim()
  if (text === "") return { ok: false, error: qsTr("The leak check returned nothing") }
  var data = null
  try {
    data = JSON.parse(text)
  } catch (e) {
    return { ok: false, error: qsTr("Could not read the leak check") }
  }
  if (!data || typeof data !== "object" || !Array.isArray(data.checks)) {
    return { ok: false, error: qsTr("Unexpected leak check output") }
  }
  var checks = []
  for (var i = 0; i < data.checks.length; i++) {
    var c = data.checks[i] || {}
    checks.push({
      group: String(c.group || ""),
      // An unknown state is shown as skipped rather than trusted: a check this
      // panel does not understand must not render as a pass.
      status: DOCTOR_STATES.indexOf(String(c.status)) !== -1 ? String(c.status) : "skip",
      text: String(c.text || ""),
      detail: String(c.detail || "")
    })
  }
  return {
    ok: true,
    healthy: data.ok === true,
    failures: Math.max(0, Number(data.failures) || 0),
    checkedAt: String(data.checkedAt || ""),
    checks: checks
  }
}

// "5 passed · 1 warning". Skips are left out: they are what the current mode
// does not need checking, not a result.
function doctorSummary(result) {
  if (!result || !result.ok) return ""
  var passed = 0, warned = 0, failed = 0
  for (var i = 0; i < result.checks.length; i++) {
    var s = result.checks[i].status
    if (s === "pass") passed++
    else if (s === "warn") warned++
    else if (s === "fail") failed++
  }
  var parts = []
  if (failed > 0) parts.push(failed === 1 ? qsTr("1 problem") : qsTr("%1 problems").arg(failed))
  if (warned > 0) parts.push(warned === 1 ? qsTr("1 warning") : qsTr("%1 warnings").arg(warned))
  if (passed > 0) parts.push(qsTr("%1 passed").arg(passed))
  return parts.length ? parts.join(" · ") : qsTr("Nothing to check")
}

// The worst result in a report, for the tile's one glyph.
function doctorOverall(result) {
  if (!result || !result.ok) return ""
  var worst = "pass"
  for (var i = 0; i < result.checks.length; i++) {
    var s = result.checks[i].status
    if (s === "fail") return "fail"
    if (s === "warn") worst = "warn"
  }
  return worst
}

function doctorGlyph(status) {
  switch (String(status || "")) {
  case "pass": return "󰄬"
  case "warn": return "󰀦"
  case "fail": return "󰅙"
  default: return "–"
  }
}

// How long ago something happened, for "checked 2 min ago".
function formatAgo(seconds) {
  var n = Math.max(0, Math.floor(Number(seconds) || 0))
  if (n < 60) return qsTr("just now")
  if (n < 3600) return qsTr("%1 min ago").arg(Math.floor(n / 60))
  return qsTr("%1 h ago").arg(Math.floor(n / 3600))
}

// Bridges, as the tile states them. `state` comes from the status poll on every
// refresh; `detail` from `tormarchy bridge status --json`, read when the bridge
// tool opens, and is only ever transport names and counts.
function bridgeStateLabel(state, detail) {
  var count = detail && Number(detail.count) > 0 ? Number(detail.count) : 0
  var transports = detail && Array.isArray(detail.transports) ? detail.transports : []
  // It has a third of the panel's width: the transport only when there is just
  // one, the full breakdown is in the expanded section.
  var what = count === 0 ? ""
    : (transports.length === 1 ? qsTr("%1 %2").arg(count).arg(String(transports[0].name || ""))
    : (count === 1 ? qsTr("1 bridge") : qsTr("%1 bridges").arg(count)))
  switch (String(state || "")) {
  case "active": return what ? qsTr("On · %1").arg(what) : qsTr("On")
  case "stashed": return count > 0 ? qsTr("Off · %1 saved").arg(count) : qsTr("Off · saved")
  default: return qsTr("Off")
  }
}

// ---- notifications -------------------------------------------------------
//
// What deserves a toast, decided here rather than in the Service so every
// transition can be tested without a shell. The states are the bar icon's:
// off, connecting, routed, browser, blocked. "problems" (the default) speaks
// only when something has gone wrong or come back; "all" also confirms the
// slow successes; "off" never speaks.

var NOTIFICATION_POLICIES = ["problems", "all", "off"]

function notificationPolicy(value) {
  var v = String(value || "")
  return NOTIFICATION_POLICIES.indexOf(v) !== -1 ? v : "problems"
}

// The connection as one word, from a status that has been applied in full.
function connectionState(s) {
  if (!s || !s.installed) return "off"
  if (s.connected && !s.torRunning) return "blocked"
  if (s.connected) return "routed"
  if (s.mode === "socks" && s.torRunning && !s.bootstrapping) return "browser"
  if (s.torRunning) return "connecting"
  return "off"
}

function notificationFor(prev, next, policy, context) {
  var p = notificationPolicy(policy)
  if (p === "off" || !prev || prev === next) return null
  var ctx = context || {}

  // The one that matters most: the network has gone quiet on purpose, and the
  // user may be nowhere near the bar to see the icon turn red. Critical, so it
  // stays until it is dealt with.
  if (next === "blocked") {
    return {
      headline: qsTr("Kill switch engaged"),
      body: qsTr("Tor stopped, so the network is blocked. Turn Tormarchy off to get it back."),
      urgency: "critical",
      glyph: "󰻌"
    }
  }
  // Leaving it replaces that toast, so a critical one never outlives the
  // problem it describes.
  if (prev === "blocked" && next === "routed") {
    return {
      headline: qsTr("Tor is back"),
      body: qsTr("Traffic is going through Tor again."),
      urgency: "normal",
      glyph: "󰕥"
    }
  }
  if (prev === "blocked") {
    return {
      headline: qsTr("Network restored"),
      body: qsTr("Tormarchy is off, so traffic no longer goes through Tor."),
      urgency: "low",
      glyph: "󰦞"
    }
  }

  if (p !== "all") return null
  if (next === "routed") {
    var where = String(ctx.exitCountryName || "")
    return {
      headline: qsTr("Connected through Tor"),
      body: where !== "" ? qsTr("Leaving from %1.").arg(where) : qsTr("All traffic now goes through Tor."),
      urgency: "low",
      glyph: "󰕥"
    }
  }
  if (next === "browser") {
    return {
      headline: qsTr("Tor is ready"),
      body: qsTr("Browser-only: launch a browser with tormarchy browser."),
      urgency: "low",
      glyph: "󰇧"
    }
  }
  return null
}

// A connect that failed on its own. Not one the user cancelled: they know.
function connectFailedNotification(error, policy) {
  if (notificationPolicy(policy) === "off") return null
  return {
    headline: qsTr("Couldn't connect to Tor"),
    body: elide(error, 200) || qsTr("Networking is unchanged."),
    urgency: "normal",
    glyph: "󰀨"
  }
}

// Bridges switched on by connect's own fallback, rather than by the user.
function bridgesNotification(prevState, nextState, policy, userAction) {
  if (notificationPolicy(policy) !== "all" || userAction) return null
  if (nextState !== "active" || prevState === "active") return null
  return {
    headline: qsTr("Using bridges"),
    body: qsTr("This network seems to block Tor, so Tormarchy switched to your saved bridges."),
    urgency: "low",
    glyph: "󰘘"
  }
}

// The sender's argv. Built here so its shape is tested: discrete arguments,
// never a shell string, through Omarchy's own sender and never notify-send.
// The headline and body are the only free text and go in as positionals, which
// the sender treats as text whatever they contain.
function notificationArgv(n, replaceId) {
  var argv = ["omarchy-notification-send", "--app-name", "Tormarchy",
              "-g", String(n.glyph || ""), "-u", String(n.urgency || "low"), "-p"]
  if (/^[0-9]+$/.test(String(replaceId || ""))) argv.push("-r", String(replaceId))
  argv.push(String(n.headline), String(n.body || ""), "--exec", "omarchy-shell", "tripleu.tor", "open")
  return argv
}

// `tormarchy bridge check` and `bridge status --json`: counts and transport
// names only. The lines themselves never come back.
function parseBridgeReport(raw) {
  var data = null
  try {
    data = JSON.parse(String(raw || "").trim())
  } catch (e) {
    return { ok: false }
  }
  if (!data || typeof data !== "object") return { ok: false }
  var transports = []
  var list = Array.isArray(data.transports) ? data.transports : []
  for (var i = 0; i < list.length; i++) {
    var t = list[i] || {}
    transports.push({
      name: String(t.name || ""),
      count: Math.max(0, Number(t.count) || 0),
      available: t.available === true,
      installHint: String(t.installHint || "")
    })
  }
  return {
    ok: true,
    state: ["active", "stashed", "off"].indexOf(String(data.state)) !== -1 ? String(data.state) : "",
    count: Math.max(0, Number(data.count) || 0),
    invalidLines: Math.max(0, Number(data.invalidLines) || 0),
    transports: transports
  }
}

// "4 bridges: 2 obfs4, 1 webtunnel, 1 plain"
function bridgeCountText(report) {
  if (!report || !report.ok || report.count === 0) return qsTr("No bridge lines")
  var parts = []
  for (var i = 0; i < report.transports.length; i++) {
    parts.push(report.transports[i].count + " " + report.transports[i].name)
  }
  var head = report.count === 1 ? qsTr("1 bridge") : qsTr("%1 bridges").arg(report.count)
  return parts.length ? head + ": " + parts.join(", ") : head
}

function missingTransports(report) {
  var out = []
  if (!report || !report.ok) return out
  for (var i = 0; i < report.transports.length; i++) {
    if (!report.transports[i].available) out.push(report.transports[i])
  }
  return out
}
