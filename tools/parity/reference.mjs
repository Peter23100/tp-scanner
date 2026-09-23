// Reference snapshot of TradierPro's chart indicator math, used ONLY to generate parity fixtures.
//
// Source: C:\tradier-pro\src\App.tsx at commit 8e8be1d
//   sessionHistogram / pocBinOf / binCenter / calcPOC / calcValueArea  (~App.tsx:957-1032)
//   vwapBands (nested in registerCustomIndicators)                     (~App.tsx:1201-1219)
//
// Copied verbatim with the TypeScript type annotations removed. Nothing else is changed. If the
// TradierPro originals change, re-copy them here, bump the commit above, and rerun gen.mjs. The
// Rust tests then say whether the port still agrees.

export function sessionHistogram(bars,buckets=60){
  if(!bars.length||buckets<1)return null;
  let minP=Infinity,maxP=-Infinity;
  for(const b of bars){if(b.low<minP)minP=b.low;if(b.high>maxP)maxP=b.high;}
  if(!isFinite(minP)||maxP<=minP)return null;
  const bSz=(maxP-minP)/buckets;
  const vols=new Array(buckets).fill(0);
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
export function pocBinOf(h){
  let maxVol=-Infinity,pocBin=0;
  for(let i=0;i<h.buckets;i++)if(h.vols[i]>maxVol){maxVol=h.vols[i];pocBin=i;}
  return pocBin;
}
export const binCenter=(h,i)=>Math.round((h.minP+(i+0.5)*h.bSz)*100)/100;

export function calcPOC(bars){
  const h=sessionHistogram(bars);if(!h)return null;
  return binCenter(h,pocBinOf(h));
}

const VA_FRACTION=0.70;
export function calcValueArea(bars){
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

export function vwapBands(d){
  let cumTPV=0,cumVol=0,cumVar=0,lastDate="";
  return d.map((b)=>{
    const dateStr=new Date(b.timestamp).toISOString().slice(0,10);
    if(dateStr!==lastDate){cumTPV=0;cumVol=0;cumVar=0;lastDate=dateStr;}
    const isPrem=b._rth===false;
    const tp=(b.high+b.low+b.close)/3;
    const vol=b.volume||0;
    if(!isPrem&&vol>0){cumTPV+=tp*vol;cumVol+=vol;}
    const v=cumVol>0?cumTPV/cumVol:(isPrem?NaN:tp);
    if(!isPrem&&vol>0&&cumVol>0)cumVar+=vol*Math.pow(tp-v,2);
    const sd=cumVol>0?Math.sqrt(cumVar/cumVol):0;
    return{vwap:v,d1up:v+sd,d1dn:v-sd,d2up:v+2*sd,d2dn:v-2*sd};
  });
}
