// TradierPro's chart pieces, copied from C:\tradier-pro\src\App.tsx (line ranges marked below) so
// the scanner's charts look and read exactly like TradierPro's: klinecharts 9.8.12, the neon
// candles, VWAP with its bands, POC and value area, EMAs/SMAs, the half-hour ET axis, day-level
// tags, the extended-hours shade, the fill-style triangle (used here for the scanner's signals)
// and the volume profile.
//
// Copied, not shared: the two programs are separate repos and the scanner must never depend on
// TradierPro's code. Changes are limited to `export`s and one adaptation, marked "scanner:".
/* eslint-disable */
import { registerIndicator, registerXAxis, registerOverlay } from "klinecharts";
import type { LineType, IndicatorSeries } from "klinecharts";

export interface OHLCVBar { time:number; open:number; high:number; low:number; close:number; volume:number; }
export type ChartInterval = "1m"|"5m"|"15m"|"daily"|"weekly"|"monthly";
export type IndicatorKey = "vwap"|"vwap_1sigma"|"vwap_2sigma"|"poc"|"va"|"sma9"|"sma20"|"sma50"|"sma200"|"ema9"|"ema20"|"ema50"|"ema200";

// ── TradierPro src/App.tsx 862-890 ──
// ── Extended-hours-on-5m policy (single source of truth) ─────────────────────────
// One continuous extended session, anchored at the 4pm close: postmarket → overnight → next-day
// premarket, shown (shaded) until the 9:25 drop the following morning. During RTH proper
// (9:25–16:00) and on every other timeframe this reduces to RTH-only — byte-identical to prior
// behavior. Indicators stay RTH-anchored separately via the per-bar `_rth` tag (vwapBands / POC).
// Weekend/holiday safe: the session is bounded by the most recent RTH bar in the data, not a
// calendar (Fri-close → Mon-open is one session).
export const PREMKT_START_MIN=4*60, RTH_START_MIN=9*60+30, RTH_END_MIN=16*60, POST_END_MIN=20*60; // 4:00/9:30/16:00/20:00 ET
export const PREMKT_DROP_MIN=RTH_START_MIN-5;   // 9:25 ET — premarket drops here
// ET minutes-of-day for an epoch-ms instant. Consolidates the inline formatToParts blocks.
export function etMinsOfDay(ms:number):number{
  const p=new Intl.DateTimeFormat("en-US",{timeZone:"America/New_York",hour12:false,hour:"2-digit",minute:"2-digit"}).formatToParts(new Date(ms));
  return parseInt(p.find(x=>x.type==="hour")?.value||"0")*60+parseInt(p.find(x=>x.type==="minute")?.value||"0");
}
// ET calendar date ("YYYY-MM-DD") for an epoch-ms instant.
export function etDateStr(ms:number):string{return new Date(ms).toLocaleDateString("en-CA",{timeZone:"America/New_York"});}
// True inside a bar's RTH window (any day).
export function isRthMs(ms:number):boolean{const m=etMinsOfDay(ms);return m>=RTH_START_MIN&&m<RTH_END_MIN;}
// True inside a bar's EXTENDED window (Tradier pre 4:00–9:30 or post 16:00–20:00), any day.
export function isExtMs(ms:number):boolean{const m=etMinsOfDay(ms);return (m>=PREMKT_START_MIN&&m<RTH_START_MIN)||(m>=RTH_END_MIN&&m<POST_END_MIN);}
// Are we inside an extended session right now? 5m only; the window is [16:00, next 9:25) — i.e.
// everything OUTSIDE regular hours (9:25–16:00). Before 9:25 we're in the prior day's session.
export function showExtended(ivl:ChartInterval):boolean{
  if(ivl!=="5m")return false;
  const m=etMinsOfDay(Date.now());
  return m>=RTH_END_MIN||m<PREMKT_DROP_MIN;   // at/after 16:00, or before 9:25
}
// Does this fetch failure mean the symbol will NEVER resolve, as opposed to a blip worth retrying?
// Tradier answers an unknown symbol with `HTTP 400 Bad Request: Invalid parameter, X: symbol not
// ── TradierPro src/App.tsx 906-912 ──
export function filterDisplayBars(raw:OHLCVBar[],showExt:boolean):OHLCVBar[]{
  if(!showExt)return raw.filter(b=>isRthMs(b.time*1000));
  let lastRthTs=0;
  for(const b of raw){const ms=b.time*1000;if(isRthMs(ms)&&ms>lastRthTs)lastRthTs=ms;}
  return raw.filter(b=>{const ms=b.time*1000;return isRthMs(ms)||(isExtMs(ms)&&ms>lastRthTs);});
}

// ── TradierPro src/App.tsx 948-1048 ──
// Volume-profile POC across 60 buckets, distributing each bar's volume across its high-low span.
// ── Session volume histogram — ONE bucketing, shared by the POC line and the value area ──────
// Extracted from calcPOC verbatim so the amber POC line and the VA bands can never disagree about
// where volume sat. The bucketing scheme is deliberately UNCHANGED: 60 buckets, anchored to the
// session's own min, each bar's volume spread evenly across the rows its high/low span. Its known
// weakness — a new session extreme re-bins every prior bar — is documented in scratch/PROFILE_PLAN.md
// and is NOT addressed here. This extraction is numerically inert; POC values must not move.
// Only the three fields the profile math actually reads — so the klinecharts-shaped bars the VP
// watermark holds (timestamp/open/high/low/close/volume) satisfy it just as well as OHLCVBar does.
export interface ProfileBar { high:number; low:number; volume:number; }
export interface SessionHistogram { minP:number; bSz:number; buckets:number; vols:number[]; }
export function sessionHistogram(bars:ProfileBar[],buckets=60):SessionHistogram|null{
  if(!bars.length||buckets<1)return null;
  let minP=Infinity,maxP=-Infinity;
  for(const b of bars){if(b.low<minP)minP=b.low;if(b.high>maxP)maxP=b.high;}
  if(!isFinite(minP)||maxP<=minP)return null;
  const bSz=(maxP-minP)/buckets;
  const vols=new Array<number>(buckets).fill(0);
  for(const b of bars){
    const vol=b.volume||0;if(!vol)continue;
    const b1=Math.max(0,Math.floor((b.low-minP)/bSz));
    const b2=Math.min(buckets-1,Math.floor((b.high-minP)/bSz));
    const sp=b2-b1+1;
    const per=vol/sp;
    for(let i=b1;i<=b2;i++)vols[i]+=per;
  }
  return {minP,bSz,buckets,vols};
}
export function pocBinOf(h:SessionHistogram):number{
  let maxVol=-Infinity,pocBin=0;
  for(let i=0;i<h.buckets;i++)if(h.vols[i]>maxVol){maxVol=h.vols[i];pocBin=i;}
  return pocBin;
}
export const binCenter=(h:SessionHistogram,i:number):number=>Math.round((h.minP+(i+0.5)*h.bSz)*100)/100;

export function calcPOC(bars:OHLCVBar[]):number|null{
  const h=sessionHistogram(bars);if(!h)return null;
  return binCenter(h,pocBinOf(h));
}

// ── Value area (VAH / VAL) ────────────────────────────────────────────────────────────────────
// The universal construction — what Sierra / Bookmap / CQG / TOS-in-TICKSIZE all agree on: start at
// the POC row and repeatedly annex whichever PAIR of rows (two above vs two below) carries more
// volume, until the enclosed rows hold VA_FRACTION of the session's volume. Reported as row
// CENTERS, the same convention calcPOC uses, so VAL <= POC <= VAH always holds.
//
// Two rules stated explicitly rather than left implicit — an unstated rule is how two
// implementations of "the same" standard end up disagreeing:
//   TIE   — equal volume above and below annexes UPWARD.
//   EDGE  — once one side is exhausted the other absorbs the remainder alone, which is also what
//           guarantees termination when the 70% target can't be reached by a pair-wise step.
export const VA_FRACTION=0.70;
export function calcValueArea(bars:OHLCVBar[]):{vah:number;val:number}|null{
  const h=sessionHistogram(bars);if(!h)return null;
  let total=0;for(const v of h.vols)total+=v;
  if(!(total>0))return null;
  const target=total*VA_FRACTION;
  const poc=pocBinOf(h);
  let lo=poc,hi=poc,acc=h.vols[poc];
  while(acc<target&&(lo>0||hi<h.buckets-1)){
    const up=(hi<h.buckets-1?h.vols[hi+1]:0)+(hi<h.buckets-2?h.vols[hi+2]:0);
    const dn=(lo>0?h.vols[lo-1]:0)+(lo>1?h.vols[lo-2]:0);
    if(hi<h.buckets-1&&(up>=dn||lo===0))hi=Math.min(h.buckets-1,hi+2);
    else if(lo>0)lo=Math.max(0,lo-2);
    else break;
    acc=0;for(let i=lo;i<=hi;i++)acc+=h.vols[i];
  }
  return {val:binCenter(h,lo),vah:binCenter(h,hi)};
}

// Overlay math is BEST-EFFORT; the load path is not. `calcPOC` as written today is total — pure
// arithmetic behind isFinite guards, no allocation sized by input — so it cannot throw. Its planned
// replacement (the session-profile module, fixed 1-cent rows) allocates rows FROM LIVE PRICES, so a
// bad tick can throw where this one cannot. Both call sites sit inside the load path and neither
// failure is survivable:
//
//   :2449 (SWR cache-refresh handler) — the load token is already claimed and there is NO enclosing
//         try/catch, so a throw skips `setLoading(false)`. `loading` strands true, which disables
//         the wedge self-heal (its <20s loadingRef grace never releases). Chart sits on stale bars
//         with no path back.
//   :2636 (loadData) — throws BEFORE `dataSymbolRef` is stamped at :2689. The outer catch turns it
//         into setErr(), leaving dataSymbolRef on the PREVIOUS symbol — which is exactly the
//         re-fire-every-3s strobe condition a0d076c fixed.
//
// Neither is worth a POC line. Null means "no POC", which syncPOC already handles by drawing
// nothing. Keep every future POC/profile call behind this wrapper.
export function safeCalcPOC(bars:OHLCVBar[]):number|null{
  try{return calcPOC(bars);}
  catch(e){console.error("[CHART] POC compute threw — POC suppressed, load continues:",e);return null;}
}

// Same contract as safeCalcPOC, same reasoning: the VA call sites sit in the load path, and a
// missing band is a cosmetic loss where a stranded `loading` flag is not.
export function safeCalcValueArea(bars:OHLCVBar[]):{vah:number;val:number}|null{
  try{return calcValueArea(bars);}
  catch(e){console.error("[CHART] value-area compute threw — VA suppressed, load continues:",e);return null;}
}

// POC must reflect TODAY's session only (TOS behavior). The intraday data
// array spans several days, so filter to bars whose ET calendar date is today
// before computing the volume profile. Empty (pre-open / non-trading day) ->
// ── TradierPro src/App.tsx 1057-1099 ──
export function defaultIndicators(ivl:ChartInterval):Set<IndicatorKey>{
  if(ivl==="1m"||ivl==="5m"||ivl==="15m")return new Set<IndicatorKey>(["vwap","vwap_1sigma","vwap_2sigma","poc","va","ema9","ema20"]);
  if(ivl==="daily")return new Set<IndicatorKey>(["sma20","sma50","sma200","ema9","ema20"]);
  // weekly / monthly: long-term view — switch to the slow MAs.
  return new Set<IndicatorKey>(["sma50","sma200","ema50","ema200"]);
}

// Trader-standard bar-count presets. 78 = one RTH session at 5m, 390 = one RTH session at 1m.
export const INTRADAY_BARS=[78,156,234,390,585,780,1170,1950] as const;
export const SWING_BARS   =[50,100,200,500,1000]              as const;

export function defaultBars(ivl:ChartInterval):number{
  if(ivl==="1m")    return 390;
  if(ivl==="5m")    return 78;
  if(ivl==="15m")   return 78;
  if(ivl==="daily") return 200;
  if(ivl==="weekly")return 200;
  return 100; // monthly
}

// Extra prior-session intraday bars loaded purely to WARM UP indicators. MACD needs slow-EMA(26) +
// signal(9) ≈ 35 bars before its histogram is valid; without this a fresh 5-min session shows no
// MACD until ~35 bars in — i.e. ~noon. These bars sit off-screen to the LEFT of the displayed
// window (autoFit fits the requested display count, not the full loaded array), so today still
// fills the screen while indicators are valid from the 9:30 open. Already-fetched (the loader pulls
// a multi-day window then slices), so this costs nothing extra — it just keeps bars we used to drop.
export const INTRADAY_INDICATOR_WARMUP=50;

// Calendar-day lookback that, accounting for weekends/holidays, comfortably covers `bars` bars.
export function lookbackDays(bars:number,ivl:ChartInterval):number{
  if(ivl==="1m")    return Math.ceil(bars/390*7/5)+3;
  if(ivl==="5m")    return Math.ceil(bars/ 78*7/5)+3;
  if(ivl==="15m")   return Math.ceil(bars/ 26*7/5)+3;
  if(ivl==="daily") return Math.ceil(bars*7/5)+7;
  if(ivl==="weekly")return Math.ceil(bars*7)+14;
  return Math.ceil(bars*31)+30; // monthly
}

// Debug-gated chart-load logging. Toggle in devtools: localStorage.tp_chart_debug = "1".
// OFF by default (one localStorage read per load; loads are infrequent → negligible). Parked
// diagnostic for the "today's bars missing" symptom (e.g. MXL 5min showed ~3 of ~60 today-bars,
// then self-corrected, no code change — likely a transient REST/cache/state glitch, no root
// cause yet). Logs REST-vs-rendered bar count + ET time range, and flags a large mid-session
// ── TradierPro src/App.tsx 1196-1282 ──
let _customIndicatorsRegistered=false;
export function registerCustomIndicators(){
  if(_customIndicatorsRegistered)return;
  _customIndicatorsRegistered=true;

  function vwapBands(d:any[]){
    let cumTPV=0,cumVol=0,cumVar=0,lastDate="";
    return d.map((b:any)=>{
      const dateStr=new Date(b.timestamp).toISOString().slice(0,10);
      if(dateStr!==lastDate){cumTPV=0;cumVol=0;cumVar=0;lastDate=dateStr;}
      // Extended-hours bars (tagged `_rth:false` at reshape, only when showExtended is on) are DISPLAYED
      // but excluded from VWAP so it stays RTH-anchored — begins exactly at the 9:30 bar. `_rth`
      // undefined => treated as RTH, so 1m/15m/daily and any untagged path are unchanged.
      const isPrem=b._rth===false;
      const tp=(b.high+b.low+b.close)/3;
      const vol=b.volume||0;
      if(!isPrem&&vol>0){cumTPV+=tp*vol;cumVol+=vol;}
      // Premarket bars emit NaN => klinecharts draws no VWAP/±σ point over the premarket region.
      const v=cumVol>0?cumTPV/cumVol:(isPrem?NaN:tp);
      if(!isPrem&&vol>0&&cumVol>0)cumVar+=vol*Math.pow(tp-v,2);
      const sd=cumVol>0?Math.sqrt(cumVar/cumVol):0;
      return{vwap:v,d1up:v+sd,d1dn:v-sd,d2up:v+2*sd,d2dn:v-2*sd};
    });
  }

  // Workaround for klinecharts 9.8.12 — line-segment merger crashes on undefined dashedValue
  // (index.esm.js:8023) so every line style must spell out style/smooth/dashedValue.
  const ln=(color:string,size=1)=>({color,size,style:"solid" as unknown as LineType,smooth:false,dashedValue:[2,2] as [number,number]});

  registerIndicator({
    name:"VWAP",shortName:"VWAP",series:"price" as unknown as IndicatorSeries,precision:2,
    figures:[{key:"vwap",title:"VWAP: ",type:"line"}],
    styles:{lines:[ln("#ffee00",2)]},
    calc:(d:any[])=>vwapBands(d).map(x=>({vwap:x.vwap})),
  });
  registerIndicator({
    name:"VWAPSD1",shortName:"±1σ",series:"price" as unknown as IndicatorSeries,precision:2,
    figures:[{key:"up",title:"+1σ: ",type:"line"},{key:"dn",title:"-1σ: ",type:"line"}],
    styles:{lines:[ln("#2277ee"),ln("#2277ee")]},
    calc:(d:any[])=>vwapBands(d).map(x=>({up:x.d1up,dn:x.d1dn})),
  });
  registerIndicator({
    name:"VWAPSD2",shortName:"±2σ",series:"price" as unknown as IndicatorSeries,precision:2,
    figures:[{key:"up",title:"+2σ: ",type:"line"},{key:"dn",title:"-2σ: ",type:"line"}],
    styles:{lines:[ln("#aa33cc"),ln("#aa33cc")]},
    calc:(d:any[])=>vwapBands(d).map(x=>({up:x.d2up,dn:x.d2dn})),
  });

  function defineMA(name:string,period:number,color:string){
    registerIndicator({
      name,shortName:name,series:"price" as unknown as IndicatorSeries,precision:2,
      figures:[{key:"ma",title:name+": ",type:"line"}],
      styles:{lines:[ln(color)]},
      calc:(d:any[])=>d.map((_,i)=>{
        if(i<period-1)return{ma:null};
        let sum=0;for(let j=0;j<period;j++)sum+=d[i-j].close;
        return{ma:sum/period};
      }),
    });
  }
  defineMA("MA9",  9,  "#88dd00");
  defineMA("MA20", 20, "#00aaff");
  defineMA("MA50", 50, "#ff8800");
  defineMA("MA200",200,"#ff4488");

  function defineEMA(name:string,period:number,color:string){
    registerIndicator({
      name,shortName:name,series:"price" as unknown as IndicatorSeries,precision:2,
      figures:[{key:"ma",title:name+": ",type:"line"}],
      styles:{lines:[ln(color)]},
      calc:(d:any[])=>{
        const k=2/(period+1);
        let prev:number|null=null;
        return d.map((b:any,i:number)=>{
          if(i===0){prev=b.close;return{ma:prev};}
          prev=(b.close-(prev as number))*k+(prev as number);
          return{ma:prev};
        });
      },
    });
  }
  defineEMA("EMA9",  9,   "#cc88ff");
  defineEMA("EMA20", 20,  "#88ccff");
  defineEMA("EMA50", 50,  "#ffcc88");
  defineEMA("EMA200",200, "#ff88cc");
}

// ── TradierPro src/App.tsx 1283-1392 ──
// Custom x-axis that emits a tick at every half-hour ET boundary instead of klinecharts's bar-index
// auto-pick (which lands on times like 10:25 / 10:50). For daily/weekly/monthly bars (delta > 24h)
// it falls through to klinecharts's default tick set.
// Module-level handle to the most recent intraday chart's bar array. Updated by each
// ChartWindow on data load so createTicks below can do exact bar-index math (klinecharts
// positions bars by index, not by clock-time, so we can compute every half-hour bar's
// coord precisely without struggling with weekend-gap interpolation).
export const intradayChartContext: { bars: any[] } = { bars: [] };
export function setIntradayChartBars(bars: any[]) { intradayChartContext.bars = bars; }

let _xAxisRegistered=false;
export function registerHalfHourAxis(){
  if(_xAxisRegistered)return;
  _xAxisRegistered=true;

  // Helper reused inside the createTicks callback. New York timezone aware.
  const tickLabel=(ts:number)=>{
    const d=new Date(ts);
    const p=new Intl.DateTimeFormat("en-US",{timeZone:"America/New_York",hour12:false,hour:"2-digit",minute:"2-digit"}).formatToParts(d);
    const h=parseInt(p.find(x=>x.type==="hour")?.value||"0");
    const m=parseInt(p.find(x=>x.type==="minute")?.value||"0");
    // 9:30 ET = market open → show the date instead of the time so each session is labeled.
    if(h===9&&m===30){
      return new Intl.DateTimeFormat("en-US",{timeZone:"America/New_York",month:"numeric",day:"numeric"}).format(d);
    }
    return `${String(h).padStart(2,"0")}:${String(m).padStart(2,"0")}`;
  };

  registerXAxis({
    name:"halfHourAxis",
    createTicks:(params:any)=>{
      const defaultTicks=params.defaultTicks;
      if(!defaultTicks||defaultTicks.length<2)return defaultTicks;
      // Daily/weekly/monthly: leave the defaults alone (no half-hour concept).
      const dt=(defaultTicks[1].value as number)-(defaultTicks[0].value as number);
      if(dt>=24*60*60*1000)return defaultTicks;

      // Use the bar array (populated by whichever intraday ChartWindow most recently loaded)
      // to do exact bar-index math. klinecharts spaces bars by index, not by time, so once we
      // know the pixel width of one bar we can position any other bar precisely — without
      // interpolating across weekend gaps.
      const bars=intradayChartContext.bars;
      if(bars.length<2)return defaultTicks;

      // Locate two defaultTicks in the bar array to derive bar width.
      const findIdx=(ts:number)=>{
        // Bars are sorted by timestamp — binary search would be faster, linear is fine at N<2000.
        for(let i=0;i<bars.length;i++)if(bars[i].timestamp===ts)return i;
        return -1;
      };
      let anchorIdx=-1, anchorCoord=0, barWidth=0;
      for(let i=0;i<defaultTicks.length-1&&barWidth===0;i++){
        const ai=findIdx(defaultTicks[i].value as number);
        const bi=findIdx(defaultTicks[i+1].value as number);
        if(ai>=0&&bi>ai){
          anchorIdx=ai;
          anchorCoord=defaultTicks[i].coord;
          barWidth=(defaultTicks[i+1].coord-defaultTicks[i].coord)/(bi-ai);
        }
      }
      if(barWidth<=0)return defaultTicks;

      // Now walk every bar and emit a tick where its ET time lands on a labeled boundary: RTH
      // (9:30–16:00) every 30 min, extended hours (4:00–9:30 pre, 16:00–20:00 post) on the hour
      // only (sparser tape → hourly reads cleaner). 9:30 ET renders as the session date via
      // tickLabel; the extended labels are the "timestamps for after-market hours" so a pre/post
      // bar is readable. Overnight (20:00–4:00) is unlabeled. Extended bars only exist while an
      // extended session is shown, so RTH-only charts are unaffected.
      //
      // Label DENSITY: the tick set above is fixed by the clock, but the pixel gap between two
      // bars shrinks as the bar count grows — so at high bar counts a clock-derived grid
      // collides into an unreadable smear. Thin by PRIORITY rather than by truncating the tail:
      // session-open dates first, then hour marks, then half-hours, each accepted only if it
      // clears MIN_TICK_PX from every tick already placed. Low bar counts keep the full
      // half-hour grid; high bar counts degrade to hourly, then to one label per session, and
      // never overlap at any count.
      const MIN_TICK_PX=52;   // ~"10:30" at the 12px default xAxis tickText, plus breathing room
      const cand:Array<{coord:number;value:number;text:string;prio:number}>=[];
      for(let i=0;i<bars.length;i++){
        const ts=bars[i].timestamp as number;
        const p=new Intl.DateTimeFormat("en-US",{timeZone:"America/New_York",hour12:false,hour:"2-digit",minute:"2-digit"}).formatToParts(new Date(ts));
        const h=parseInt(p.find(x=>x.type==="hour")?.value||"0");
        const m=parseInt(p.find(x=>x.type==="minute")?.value||"0");
        const mins=h*60+m;
        const inRth=mins>=9*60+30&&mins<16*60;
        const inExt=(mins>=4*60&&mins<9*60+30)||(mins>=16*60&&mins<20*60);
        let prio:number;
        if(inRth){
          if(m%30!==0)continue;                    // RTH: every 30 min (9:30 → date)
          prio=(h===9&&m===30)?0:(m===0?1:2);       // session date > top of hour > half past
        } else if(inExt){
          if(m!==0)continue;                       // extended: on the hour only
          prio=1;
        } else continue;                           // overnight dead zone: no label
        cand.push({coord:anchorCoord+(i-anchorIdx)*barWidth,value:ts,text:tickLabel(ts),prio});
      }
      const out:Array<{coord:number;value:number;text:string}>=[];
      for(const prio of [0,1,2]){
        for(const c of cand){
          if(c.prio!==prio)continue;
          if(out.some(k=>Math.abs(k.coord-c.coord)<MIN_TICK_PX))continue;
          out.push({coord:c.coord,value:c.value,text:c.text});
        }
      }
      out.sort((a,b)=>a.coord-b.coord);            // priority pass emits out of order; axis wants ascending
      return out.length>0?out:defaultTicks;
    },
  });
}

// ── TradierPro src/App.tsx 1412-1484 ──
// Per-day reference marker: an arrow + price number anchored AT a specific bar (the day's
// high/low/open/close candle), instead of a line across the chart. extendData carries the
// label text, arrow glyph, color, and whether to sit above or below the bar.
let _dayLevelOverlayRegistered=false;
export function registerDayLevelOverlay(){
  if(_dayLevelOverlayRegistered)return;
  _dayLevelOverlayRegistered=true;
  registerOverlay({
    name:"dayLevelTag",
    totalStep:1,
    needDefaultPointFigure:false,
    needDefaultXAxisFigure:false,
    needDefaultYAxisFigure:false,
    createPointFigures:(params:any)=>{
      const c=params.coordinates?.[0];
      if(!c)return[];
      const ed=params.overlay.extendData||{};
      const color:string=ed.color||"#76808F";
      const text:string=ed.text||"";
      const above:boolean=ed.above!==false;  // marker sits above the bar (true) or below (false)
      const offsetRows:number=ed.offsetRows||0;  // 9A: stagger lane (steps the text away from the price)
      // Ported from klinecharts' built-in CandleHighLowPriceView._drawMark: a small chevron whose
      // tip sits ~2px off the price (pointing AT it), a short elbow connector, and the price text
      // offset to the side (never over the price). offsetRows steps the text lane vertically when
      // two labels would collide. Transparent text background — klinecharts default paints a box.
      const o0=above?-2:2,o1=above?-9:9;  // 9A: larger base offset so above/below bands separate
      const cx=c.x,cy=c.y,startY=cy+o0;
      const width=params.bounding?.width||0;
      const onRight=cx>width/2;
      const lineEndX=onRight?cx-5:cx+5;
      const y=startY+o1+(above?-1:1)*offsetRows*15;  // 9A: step the text lane (15px) away from the price chevron
      const lineStyles={color,size:1,style:"solid",dashedValue:[]};
      return[
        {key:"arrow",type:"line",attrs:{coordinates:[{x:cx-2,y:startY+o0},{x:cx,y:startY},{x:cx+2,y:startY+o0}]},styles:lineStyles,ignoreEvent:true},
        {key:"elbow",type:"line",attrs:{coordinates:[{x:cx,y:startY},{x:cx,y},{x:lineEndX,y}]},styles:lineStyles,ignoreEvent:true},
        {key:"tag",type:"text",
          attrs:{x:onRight?lineEndX-4:lineEndX+4,y,text,align:onRight?"right":"left",baseline:"middle"},
          styles:{color,size:10,family:"var(--sans)",weight:400,backgroundColor:"rgba(0,0,0,0)",borderColor:"rgba(0,0,0,0)",borderSize:0,paddingLeft:0,paddingRight:0,paddingTop:0,paddingBottom:0},
          ignoreEvent:true},
      ];
    },
  });
}

// Extended-hours shade: a translucent full-height band behind the current extended session's bars
// (post → overnight → pre), TOS-style. Anchored at ONE point (the first extended bar) and drawn
// from there to the pane's right edge — the extended run is always the trailing segment, so this
// covers it and any live bars that append. Low alpha so candles read through; ignoreEvent + lock.
let _extHoursShadeRegistered=false;
export function registerExtHoursShadeOverlay(){
  if(_extHoursShadeRegistered)return;
  _extHoursShadeRegistered=true;
  registerOverlay({
    name:"extHoursShade",
    totalStep:1,
    needDefaultPointFigure:false,
    needDefaultXAxisFigure:false,
    needDefaultYAxisFigure:false,
    createPointFigures:(params:any)=>{
      const c=params.coordinates?.[0];
      if(!c)return[];
      const w=params.bounding?.width||0, h=params.bounding?.height||0;
      const x0=Math.max(0,c.x-3);   // start ~half a bar left of the first extended bar's center
      const width=Math.max(0,w-x0);
      if(width<=0)return[];
      return[{key:"shade",type:"rect",
        attrs:{x:x0,y:0,width,height:h},
        styles:{style:"fill",color:"rgba(110,130,170,0.10)"},
        ignoreEvent:true}];
    },
  });
}

// ── TradierPro src/App.tsx 1485-1559 ──
// 9B: fill marker — a small triangle whose APEX SITS ON the fill price (an equity fill's own
// avg_fill_price; for an option, the underlying at that fill). Up-triangle under the level for
// buys, down-triangle over it for sells; triangle-only (no label) by design.
let _fillMarkerRegistered=false;
export function registerFillMarkerOverlay(){
  if(_fillMarkerRegistered)return;
  _fillMarkerRegistered=true;
  registerOverlay({
    name:"fillMarker",
    totalStep:1,
    needDefaultPointFigure:false,
    needDefaultXAxisFigure:false,
    needDefaultYAxisFigure:false,
    createPointFigures:(params:any)=>{
      const c=params.coordinates?.[0];
      if(!c)return[];
      const ed=params.overlay.extendData||{};
      const buy:boolean=!!ed.buy;
      // THREE encodings on one glyph, using the three free visual channels:
      //   SHAPE  — apex points at the price: up for a buy, down for a sell
      //   COLOUR — instrument: cyan call / amber put, equity keeps the long-standing green/red
      //   FILL   — HOLLOW opens the position, SOLID closes it
      // The third channel is what separates STO from BTC. A sold-to-open call is a HOLLOW cyan
      // down-triangle; buying it back is a SOLID cyan up-triangle. Equity is always solid:
      // a bare `sell` is genuinely ambiguous (closes a long or opens a short), and inventing a
      // distinction the order row cannot support would be worse than not drawing one. That it
      // now reads like a close is only nominal — green/red already says "this is equity".
      const color=ed.cp==="C"?"#00d0ff":ed.cp==="P"?"#ff9500":(buy?"#00cc55":"#ff4455");
      const hollow=ed.opening===true;
      // ★ The apex sits EXACTLY on the fill price: cy IS the price coordinate. It used to be held
      // 7px clear so the glyph could never cover a candle, but that traded away the one thing the
      // mark exists to say — WHERE the fill happened — for tidiness. The body extends away from
      // the level, so the triangle may overlap the bar while the tip still reads the exact price.
      // Geometry: 10px wide × 8px tall — the SAME triangle as before the apex moved onto the
      // price (apex cy±7 → base cy±15 was also 8 tall). Moving the apex is not licence to
      // resize it; keep this height unless the size itself is what is being changed.
      const cx=c.x,cy=c.y,s=5;
      const apexY=cy;            // ON the price
      const baseY=buy?cy+8:cy-8; // body hangs away from it, 8px (y grows downward)
      return[{key:"tri",type:"polygon",
        attrs:{coordinates:[{x:cx,y:apexY},{x:cx-s,y:baseY},{x:cx+s,y:baseY}]},
        styles:hollow?{style:"stroke",borderColor:color,borderSize:1.5}:{style:"fill",color},
        ignoreEvent:true}];
    },
  });
}

// Current-price connector: a short horizontal line from the latest bar's right edge to the
// chart's right margin (where priceMark.last draws the Y-axis price bubble). Replaces the old
// full-width last-price rule — the line touches only the current bar, not the history.
let _lastPriceMarkRegistered=false;
export function registerLastPriceMarkOverlay(){
  if(_lastPriceMarkRegistered)return;
  _lastPriceMarkRegistered=true;
  registerOverlay({
    name:"lastPriceMark",
    totalStep:1,
    needDefaultPointFigure:false,
    needDefaultXAxisFigure:false,
    needDefaultYAxisFigure:false,
    createPointFigures:(params:any)=>{
      const c=params.coordinates?.[0];
      if(!c)return[];
      const color:string=params.overlay.styles?.line?.color||"#8a98ad";
      return[{
        key:"line",
        type:"line",
        attrs:{coordinates:[{x:c.x,y:c.y},{x:params.bounding.width,y:c.y}]},
        styles:{color,size:1,style:"solid",dashedValue:[]},
        ignoreEvent:true,
      }];
    },
  });
}

// ── TradierPro src/App.tsx 10796-10850 ──
export const VP_BINS=60;
// Shared volume-profile builder for both the Options Chart and the equity ChartWindow. Returns the
// overlay points (bin centers) + extendData (vols/maxV/pocIdx) the `volProfile` overlay's
// createPointFigures reads. Null when there's nothing to draw.
//
// Reads the SAME `sessionHistogram` the POC line does, so the watermark and the amber line no
// longer disagree about how volume is distributed. This replaced a midpoint model — every bar's
// whole volume dumped into the single bin holding (high+low)/2 — which put a hard ceiling on
// granularity: a profile could never have more FILLED rows than there were bars, so raising the
// bin count just opened gaps between spikes. Spreading each bar across the rows it spans has no
// such ceiling, which is what makes VP_BINS=60 legible where it would have been a picket fence.
//
// SESSION scope: TODAY's RTH only, matching the POC line's `todaySessionBars`. A profile is a
// profile OF A SESSION — smearing several days into one watermark was never what it should have
// shown, and it disagreed with the amber POC line drawn on the same chart.
//
// This makes the watermark INTRADAY-ONLY, which is intended: daily/weekly/monthly bars are stamped
// at UTC midnight, so none of them survive the RTH filter, the bar set comes back empty, and the
// spec returns null — no watermark on those timeframes rather than a wrong multi-day one. Stated
// here because it falls out of the filter rather than from an explicit isIntraday gate.
//
// Points stay unrounded — binCenter's cent-rounding is for reported price levels, not pixel Y.
export function vpProfileOverlaySpec(allBars:any[]):{points:{timestamp:number;value:number}[];extendData:any}|null{
  if(!allBars||allBars.length<3)return null;
  // scanner: the session shown (the last bar's date), not the wall-clock day, so replays work.
  const today=etDateStr(allBars[allBars.length-1].timestamp);
  const bars=allBars.filter(b=>isRthMs(b.timestamp)&&etDateStr(b.timestamp)===today);
  if(bars.length<3)return null;
  const h=sessionHistogram(bars,VP_BINS);if(!h)return null;
  const vols=h.vols,pocIdx=pocBinOf(h);
  const maxV=vols[pocIdx]||0;
  if(maxV<=0)return null;
  const ts=bars[0].timestamp;
  return{points:vols.map((_,i)=>({timestamp:ts,value:h.minP+(i+0.5)*h.bSz})),extendData:{vols,maxV,pocIdx}};
}
let _ocVolProfRegistered=false;
export function registerVolProfileOverlay(){
  if(_ocVolProfRegistered)return;_ocVolProfRegistered=true;
  registerOverlay({
    name:"volProfile",totalStep:VP_BINS+1,needDefaultPointFigure:false,needDefaultXAxisFigure:false,needDefaultYAxisFigure:false,
    createPointFigures:(params:any)=>{
      const coords=params.coordinates;const ext=params.overlay.extendData;
      if(!coords?.length||!ext||!ext.maxV)return[];
      const vols:number[]=ext.vols||[],pocIdx:number=ext.pocIdx??-1;
      const w=params.bounding?.width||600;const maxLen=w*0.30;
      const binH=coords.length>1?Math.max(2,Math.abs(coords[1].y-coords[0].y)-1):8;
      const figs:any[]=[];
      coords.forEach((c:any,i:number)=>{
        const v=vols[i]||0;if(v<=0)return;
        const len=Math.max(1,(v/ext.maxV)*maxLen);
        figs.push({type:"rect",attrs:{x:0,y:c.y-binH/2,width:len,height:binH},styles:{style:"fill",color:i===pocIdx?"rgba(120,165,255,0.20)":"rgba(96,124,180,0.12)"},ignoreEvent:true});
      });
      return figs;
    },
  });
}
