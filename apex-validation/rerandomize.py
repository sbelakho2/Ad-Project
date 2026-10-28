#!/usr/bin/env python3
"""Audit a fixed eligible-user outcome roster with random sticky arm splits.

CSV columns: user_id,revenue. Exactly one row per PREDEFINED eligible user,
including users with zero revenue. This validates null calibration under
the supplied assignment model. Injected effects are assumptions, not real lift.
"""
import argparse,csv,json
from pathlib import Path
import numpy as np
from scipy.stats import norm
from benchmark import ci

def run(y,q,splits,effect,seed):
    rng=np.random.default_rng(seed);out={k:0 for k in ["current_ht_90","corrected_ht_975","roster_mean_975"]}
    n=len(y);zero=int(np.sum(y==0));yy=y*y
    for _ in range(splits):
        a=rng.random(n)<q;nt=int(a.sum());nc=n-nt
        if min(nt,nc)<2:continue
        st=y[a].sum()*(1+effect);sc=y[~a].sum()
        st2=yy[a].sum()*(1+effect)**2;sc2=yy[~a].sum()
        if min(st,sc)<=0:continue
        srm=(nt-n*q)**2/(n*q*(1-q))<=10.8276
        lp=np.log(st*(1-q)/(sc*q))
        v=q*(1-q)*(st2/st**2+sc2/sc**2)
        vf=(np.sqrt((1-q)*st2/st**2)+np.sqrt(q*sc2/sc**2))**2
        out["current_ht_90"]+=int(srm and np.expm1(lp)>=.01 and lp>norm.ppf(.9)*np.sqrt(v))
        out["corrected_ht_975"]+=int(srm and np.expm1(lp)>=.01 and lp>norm.ppf(.975)*np.sqrt(vf))
        mt=st/nt;mc=sc/nc
        vt=(st2-st**2/nt)/(nt-1);vc=(sc2-sc**2/nc)/(nc-1)
        lm=np.log(mt/mc);se=np.sqrt(max(0.,vt/nt/mt**2+vc/nc/mc**2))
        out["roster_mean_975"]+=int(srm and np.expm1(lm)>=.01 and lm>norm.ppf(.975)*se)
    return {"eligible_users":n,"zero_revenue_users":zero,"splits":splits,"q":q,"injected_effect":effect,"seed":seed,
            "rates":{k:{"k":v,"rate":v/splits,"ci95":ci(v,splits)} for k,v in out.items()},
            "limits":"Fixed-outcome rerandomization checks Bernoulli null calibration. It cannot validate real provider assignment, supply response, missing eligible users or injected-effect realism."}

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument("csv");p.add_argument("--splits",type=int,default=10000)
    p.add_argument("--share",type=float,default=.25);p.add_argument("--effect",type=float,default=0.)
    p.add_argument("--seed",type=int,default=318799);p.add_argument("--output",required=True)
    args=p.parse_args();seen=set();vals=[]
    with open(args.csv,newline="") as f:
        for row in csv.DictReader(f):
            user=row["user_id"]
            if not user or user in seen:raise ValueError("Missing or duplicate eligible user id")
            seen.add(user);value=float(row["revenue"])
            if not np.isfinite(value) or value<0:raise ValueError("Invalid revenue")
            vals.append(value)
    if len(vals)<60 or not 0<args.share<1 or args.effect<=-1 or args.splits<1:raise ValueError("Invalid setup")
    result=run(np.array(vals),args.share,args.splits,args.effect,args.seed)
    Path(args.output).write_text(json.dumps(result,indent=2));print(json.dumps(result,indent=2))
if __name__=="__main__":main()
