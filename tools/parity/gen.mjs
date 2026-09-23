// Runs the TradierPro reference math (reference.mjs) over the parity fixture and writes the
// expected outputs that crates/tp-marketdata/tests/parity.rs checks the Rust port against.
//
//   node tools/parity/gen.mjs
//
// Bars are prepared the way TradierPro's chart prepares them: `timestamp` in UTC ms and
// `_rth: false` on extended-hours bars. POC and VA run per session over that session's RTH bars
// only.

import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { calcPOC, calcValueArea, vwapBands } from "./reference.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const fixture = join(here, "../../crates/tp-marketdata/tests/fixtures/parity_two_sessions.csv");
const outFile = join(here, "expected/parity_two_sessions.json");

const fmt = new Intl.DateTimeFormat("en-US", {
  timeZone: "America/New_York", hourCycle: "h23",
  year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", second: "2-digit",
});
const nyOffsetMs = (t) => {
  const p = Object.fromEntries(fmt.formatToParts(new Date(t)).map((x) => [x.type, x.value]));
  return Date.UTC(+p.year, +p.month - 1, +p.day, +p.hour, +p.minute, +p.second) - t;
};
// New York wall time -> UTC ms. Two passes settle the offset. Fixture times avoid the DST edges.
const nyToUtc = (date, time) => {
  const [y, mo, d] = date.split("-").map(Number);
  const [h, mi, s] = time.split(":").map(Number);
  const wall = Date.UTC(y, mo - 1, d, h, mi, s);
  let t = wall - nyOffsetMs(wall);
  t = wall - nyOffsetMs(t);
  return t;
};

const bars = [];
for (const raw of readFileSync(fixture, "utf8").split(/\r?\n/)) {
  const l = raw.trim();
  if (!l || l.startsWith("#") || l.startsWith("symbol,")) continue;
  const [, start, o, h, lo, c, v] = l.split(",");
  const [date, time] = start.split(" ");
  const [hh, mm] = time.split(":").map(Number);
  const mins = hh * 60 + mm;
  bars.push({
    date,
    timestamp: nyToUtc(date, time),
    open: +o, high: +h, low: +lo, close: +c, volume: +v,
    _rth: mins >= 9 * 60 + 30 && mins < 16 * 60,
  });
}

const num = (x) => (Number.isFinite(x) ? x : null);
const vwap = vwapBands(bars).map((b) =>
  Number.isFinite(b.vwap) ? { vwap: b.vwap, d1up: b.d1up, d1dn: b.d1dn, d2up: b.d2up, d2dn: b.d2dn } : null);

const sessions = [];
for (const date of [...new Set(bars.map((b) => b.date))]) {
  const rth = bars.filter((b) => b.date === date && b._rth);
  const va = calcValueArea(rth);
  sessions.push({ date, poc: num(calcPOC(rth)), vah: va ? va.vah : null, val: va ? va.val : null });
}

const out = {
  source: "tradier-pro src/App.tsx @ 8e8be1d (tools/parity/reference.mjs)",
  bars: bars.length,
  vwap,
  sessions,
};
writeFileSync(outFile, JSON.stringify(out, null, 1) + "\n");
console.log(`wrote ${outFile}: ${bars.length} bars, ${sessions.length} sessions`);
