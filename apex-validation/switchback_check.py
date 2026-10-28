#!/usr/bin/env python3
"""Exact randomization audit of the production 14-block admissible design.

Sharp null, arbitrary block correlation/trends, no carryover. Every schedule
is enumerated. This is an independent statistical oracle, not engine parity.
"""
import itertools,json
from pathlib import Path
import numpy as np
from scipy.stats import t
from benchmark import rate

def schedules():
    out=[]
    for a in itertools.product((0,1),repeat=14):
        if sum(a)!=7 or any(a[i]==a[i+1]==a[i+2] for i in range(12)):continue
        w=np.zeros(7,dtype=int)
        for i,z in enumerate(a):
            if z:w[(2*i)%7]+=1;w[(2*i+1)%7]+=1
        if np.all(w==2):out.append(a)
    return np.array(out,dtype=float)

def main():
    p=schedules();rng=np.random.default_rng(722519);n=100000;result=[]
    for rho in [0.,.5,.9]:
        for drift in [0.,.01]:
            eps=rng.normal(0,.04,(n,14));y=np.empty_like(eps);y[:,0]=eps[:,0]/np.sqrt(1-rho*rho)
            for k in range(1,14):y[:,k]=rho*y[:,k-1]+eps[:,k]
            y+=np.arange(14)*drift
            ix=rng.integers(0,len(p),n);a=p[ix]
            mt=(a*y).sum(axis=1)/7;mc=((1-a)*y).sum(axis=1)/7
            vt=(a*(y-mt[:,None])**2).sum(axis=1)/6
            vc=((1-a)*(y-mc[:,None])**2).sum(axis=1)/6
            se=np.sqrt((vt+vc)/7);df=np.floor((vt+vc)**2/(vt*vt/6+vc*vc/6)).clip(1)
            old=(mt-mc)>t.ppf(.9,df)*se
            # Difference in means is sufficient for the Fisher sharp-null
            # oracle; finite assignment distribution handles serial outcome
            # structure without an iid-block assumption.
            perm=y@((2*p-1).T)/7;obs=(mt-mc)
            pval=np.mean(perm>=obs[:,None]-1e-12,axis=1)
            result.append({"rho":rho,"log_drift_per_block":drift,"trials":n,
                           "current_welch_90":rate(old),"exact_randomization_05":rate(pval<=.05),
                           "exact_randomization_025":rate(pval<=.025)})
    out={"schedules":len(p),"minimum_one_sided_p":1/len(p),"model":"sharp no-effect null; Gaussian correlated log outcomes; no carryover",
         "results":result,"limits":"Exact test handles arbitrary fixed outcome paths only under sharp null/no carryover and the uniform admissible schedule law. Does not certify billing interval coverage under heterogeneous effects."}
    Path(__file__).with_name("results").joinpath("switchback.json").write_text(json.dumps(out,indent=2))
    print(json.dumps(out,indent=2))
if __name__=="__main__":main()
