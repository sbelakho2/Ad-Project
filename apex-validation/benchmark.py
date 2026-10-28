#!/usr/bin/env python3
"""Independent validity benchmarks, not a fitted publisher lift forecast.

The production source remains unmodified. Effects share simulated potential
outcomes (common random numbers); independent trials, not users, are the unit
of Monte Carlo uncertainty. See plan.json for the frozen experiment grid.
"""
from __future__ import annotations
import argparse, json, math, os, subprocess, time, shutil
from pathlib import Path
import numpy as np
from scipy.stats import binom, norm, beta

HERE=Path(__file__).resolve().parent
Z90=norm.ppf(.90)
Z975=norm.ppf(.975)
EFFECTS=[-.05,-.02,0.,.01,.03,.05]

def ci(k,n,alpha=.05):
    """Exact two-sided Clopper-Pearson Monte Carlo interval."""
    return [float(beta.ppf(alpha/2,k,n-k+1)) if k else 0.,
            float(beta.ppf(1-alpha/2,k+1,n-k)) if k<n else 1.]

def rate(a):
    k=int(np.sum(a));n=int(np.size(a))
    return {"k":k,"n":n,"rate":k/n,"ci95":ci(k,n)}

def exact():
    ans=[]
    n=20000
    k=np.arange(1,n,dtype=float)
    for q in [.25,.5]:
        prob=binom.pmf(k,n,q)
        ratio=k*(1-q)/((n-k)*q)
        log=np.log(ratio)
        var_old=q*(1-q)*(1/k+1/(n-k))
        var_fix=(np.sqrt((1-q)/k)+np.sqrt(q/(n-k)))**2
        srm=(k-n*q)**2/(n*q*(1-q))<=10.8276
        row={"n":n,"q":q,"enumerated_probability_mass":float(prob.sum()),
             "se_current_at_expected_split":1/np.sqrt(n),
             "se_correct_at_expected_split":1/np.sqrt(n*q*(1-q))}
        row["effects"]=[]
        for d in EFFECTS:
            l=log+np.log1p(d)
            point=np.expm1(l)
            old=np.expm1(l-Z90*np.sqrt(var_old))
            fix=np.expm1(l-Z975*np.sqrt(var_fix))
            row["effects"].append({"effect":d,
                "current_bound_positive":float(prob[old>0].sum()),
                "current_final_keep":float(prob[(old>0)&(point>=.01)&srm].sum()),
                "corrected_975_final_keep":float(prob[(fix>0)&(point>=.01)&srm].sum())})
        ans.append(row)
    return ans

def draw(rng,shape,profile):
    sig,noise,prezero,postzero=profile["sigma"],profile["noise"],profile["prezero"],profile["postzero"]
    v=rng.lognormal(-sig*sig/2,sig,shape)
    x=v*rng.lognormal(-noise*noise/2,noise,shape)
    y=v*rng.lognormal(-noise*noise/2,noise,shape)
    if prezero: x*=rng.random(shape)>=prezero
    if postzero: y*=rng.random(shape)>=postzero
    return x,y

def groupmom(x,y,a):
    af=a.astype(float); nc=af.sum(axis=1)
    return {"n":nc,"x":np.sum(x*af,axis=1),"y":np.sum(y*af,axis=1),
            "xx":np.sum(x*x*af,axis=1),"yy":np.sum(y*y*af,axis=1),
            "xy":np.sum(x*y*af,axis=1)}

def adjusted(g,theta,center,scale):
    total=scale*g["y"]-theta*(g["x"]-g["n"]*center)
    squares=(scale*scale*g["yy"]+theta*theta*(g["xx"]-2*center*g["x"]+g["n"]*center**2)
             -2*scale*theta*(g["xy"]-center*g["y"]))
    mean=total/g["n"]
    var=np.maximum(0,(squares-total*total/g["n"])/(g["n"]-1))
    return mean,var

def monte(profile,plan):
    rng=np.random.default_rng(profile["seed"])
    tx,ty=draw(np.random.default_rng(profile["seed"]+100000),100000,profile)
    theta=float(np.cov(tx,ty,ddof=1)[0,1]/np.var(tx,ddof=1))
    corr=float(np.corrcoef(tx,ty)[0,1])
    rows=[];summaries=[]
    for q in plan.get("shares",[.25,.5,.9]):
        stores={}
        for offset in range(0,plan["trials"],plan["batch"]):
            b=min(plan["batch"],plan["trials"]-offset)
            x,y=draw(rng,(b,plan["players"]),profile)
            arm=rng.random(x.shape)<q
            t=groupmom(x,y,arm);c=groupmom(x,y,~arm)
            center=np.mean(x,axis=1)
            srm=(t["n"]-plan["players"]*q)**2/(plan["players"]*q*(1-q))<=10.8276
            ratio=t["y"]*(1-q)/(c["y"]*q)
            oldse=np.sqrt(q*(1-q)*(t["yy"]/t["y"]**2+c["yy"]/c["y"]**2))
            fixse=np.sqrt((1-q)*t["yy"]/t["y"]**2)+np.sqrt(q*c["yy"]/c["y"]**2)
            for d in EFFECTS:
                l=np.log(ratio)+np.log1p(d)
                point=np.expm1(l)
                for method,z,se in [("current_ht_90",Z90,oldse),("corrected_ht_975",Z975,fixse)]:
                    lower=np.expm1(l-z*se)
                    key=(d,method)
                    stores.setdefault(key,[]).append(np.column_stack([
                        (lower>0)&(point>=.01)&srm,lower>d,np.maximum(lower,0),se]))
                for name,beta_ in [("roster_unadjusted_975",0.),("roster_cuped_975",theta)]:
                    mt,vt=adjusted(t,beta_,center,1+d);mc,vc=adjusted(c,beta_,center,1.)
                    valid=(mt>0)&(mc>0)
                    logm=np.log(np.maximum(mt,1e-300)/np.maximum(mc,1e-300))
                    se=np.sqrt(vt/t["n"]/mt**2+vc/c["n"]/mc**2)
                    lower=np.expm1(logm-Z975*se)
                    key=(d,name)
                    stores.setdefault(key,[]).append(np.column_stack([
                        valid&(lower>0)&(np.expm1(logm)>=.01)&srm,valid&(lower>d),np.maximum(lower,0)*valid,se]))
                # Mirror current holdout's ratio-of-means and 1.697 critical
                # value on FULL eligible roster. Separate deterministic tests
                # cover the current feed's post-treatment selection defect.
                mt,vt=adjusted(t,0.,center,1+d);mc,vc=adjusted(c,0.,center,1.)
                hpoint=mt/mc-1
                hse=np.sqrt(vt/t["n"]/mc**2+mt**2*vc/c["n"]/mc**4)
                hlow=hpoint-1.697*hse
                split=(np.abs(t["n"]/plan["players"]-.9)<=.03) if q==.9 else np.ones(b,dtype=bool)
                stores.setdefault((d,"current_holdout_full_roster"),[]).append(np.column_stack([
                    (hlow>0)&split,hlow>d,np.maximum(hlow,0),hse]))
            if offset==0:
                summaries.append({"q":q,"sample_y_cv":float(np.std(y)/np.mean(y)),
                    "sample_pre_post_correlation":float(np.corrcoef(x.ravel(),y.ravel())[0,1])})
        for (d,name),pieces in stores.items():
            arr=np.concatenate(pieces)
            rows.append({"profile":profile["name"],"q":q,"effect":d,"method":name,
                "keep":rate(arr[:,0]),"lower_above_truth":rate(arr[:,1]),
                "mean_positive_lower":float(arr[:,2].mean()),"mean_se":float(arr[:,3].mean())})
    return {"profile":profile,"historical_theta":theta,"historical_pre_post_correlation":corr,
            "sample_diagnostics":summaries,"results":rows}

def sequential(plan):
    rng=np.random.default_rng(721117)
    days=np.array([7,14,21.])
    cov=np.sqrt(np.minimum.outer(days,days)/np.maximum.outer(days,days))
    z=rng.multivariate_normal(np.zeros(3),cov,size=plan["sequential_trials"])
    old=(z[:,0]>norm.ppf(.99))|(z[:,1]>Z90)|((z[:,1]>0)&(z[:,1]<=Z90)&(z[:,2]>Z90))
    crit=np.array([norm.ppf(.995),norm.ppf(.9775),norm.ppf(.9775)])
    new=np.any(z>crit,axis=1)
    return {"model":"Gaussian independent daily increments; isolates repeated looks only; no guardrails/minimum effect",
            "old_99_90_90":rate(old),"alpha_spent_005_0225_0225":rate(new),
            "critical_values":crit.tolist(),"union_bound_alpha":.05}

def auctions(plan):
    """Train-selected floor on an independent test market; hypothetical models."""
    rng=np.random.default_rng(651926)
    floors=np.array([0.,.1,.2,.4,.6,.8,1.,1.2,1.5,2.,2.5,3.])
    train=np.sort(rng.lognormal(0.,.9,(plan["auction_train"],3)),axis=1)
    test=np.sort(rng.lognormal(0.,.9,(plan["auction_test"],3)),axis=1)
    out=[]
    for kind in ["first_price_fixed_bids","second_price_truthful","first_price_reserve_response"]:
        def revenue(v,f):
            top=v[:,-1]
            if kind=="second_price_truthful":
                return np.where(top>=f,np.maximum(v[:,-2],f),0.)
            scale=.65 if kind=="first_price_fixed_bids" else .45+.35*(1-np.exp(-f))
            bid=scale*top
            return np.where(bid>=f,bid,0.)
        basefloor=.6
        base=revenue(test,basefloor)
        means=np.array([np.mean(revenue(train,f)) for f in floors])
        best=float(floors[np.argmax(means)])
        selected=revenue(test,best)
        # Delta-method paired log ratio across independent auctions.
        mu0=base.mean();mu1=selected.mean(); influence=selected/mu1-base/mu0
        se=np.std(influence,ddof=1)/np.sqrt(len(base))
        lift=mu1/mu0-1
        out.append({"model":kind,"baseline_floor":basefloor,"train_selected_floor":best,
            "train_auctions":len(train),"locked_test_auctions":len(test),
            "test_lift":float(lift),"ci95_lift":np.expm1(np.log1p(lift)+np.array([-1,1])*norm.ppf(.975)*se).tolist(),
            "raise_floor_20pct_test_lift":float(np.mean(revenue(test,basefloor*1.2))/mu0-1),
            "assumptions":"3 bidders, lognormal private values; no budgets, no learning, no app retention; response model imposed, not fitted"})
    return out

def parity(plan):
    binary=HERE/"driver/target/release/apex-validation-driver"
    if not binary.exists():
        cargo=shutil.which("cargo") or "/root/.cargo/bin/cargo"
        subprocess.run([cargo,"build","--release","--manifest-path",str(HERE/"driver/Cargo.toml")],check=True)
    rng=np.random.default_rng(991721)
    fixtures=[];expected=[]
    for q in [.25,.5,.9]:
        for _ in range(20):
            y=rng.lognormal(0,1.5,2000);a=rng.random(len(y))<q
            t=y[a];c=y[~a]
            v=q*(1-q)*((t*t).sum()/t.sum()**2+(c*c).sum()/c.sum()**2)
            l=np.log(t.sum()*(1-q)/(c.sum()*q))
            fixtures.append({"kind":"ht","q":q,"control":c.tolist(),"test":t.tolist()})
            expected.append({"lift":np.expm1(l),"variance":v,"lower90":np.expm1(l-Z90*np.sqrt(v))})
    for price in [1200000,1250000,1300000]:
        fixtures.append({"kind":"holdout","control":[1000000]*5000,"test":[price]*36000,
                         "scope_revenue":5000*1000000+36000*price})
        lift=price/1000000-1
        expected.append({"lift":lift,"se":0.,"lower90":lift,"split_ok":True})
    inp="".join(json.dumps(f)+"\n" for f in fixtures)
    res=subprocess.run([str(binary)],input=inp,text=True,capture_output=True,check=True)
    actual=[json.loads(l) for l in res.stdout.splitlines()]
    assert len(actual)==len(expected)
    for i,(e,a) in enumerate(zip(expected,actual)):
        for k,x in e.items():
            assert math.isclose(float(x),float(a[k]),rel_tol=2e-10,abs_tol=2e-10),(i,k,x,a[k])
    return {"fixtures":len(fixtures),"passed":len(fixtures),"production_source":"unmodified apex-lift dependency",
            "selection_witnesses":actual[-3:]}

def main():
    p=argparse.ArgumentParser();p.add_argument("--quick",action="store_true");p.add_argument("--skip-parity",action="store_true");args=p.parse_args()
    plan=json.loads((HERE/"plan.json").read_text())
    if args.quick: plan["trials"]=200;plan["auction_test"]=100000;plan["auction_train"]=50000
    start=time.time();result={"plan":plan,"exact_binomial":exact()}
    for profile in plan["profiles"]:
        print("running",profile["name"],flush=True)
        result.setdefault("monte_carlo",[]).append(monte(profile,plan))
        (HERE/"results/checkpoint.json").write_text(json.dumps(result,indent=2))
    result["sequential"]=sequential(plan);result["auctions"]=auctions(plan)
    if not args.skip_parity: result["production_parity"]=parity(plan)
    result["elapsed_seconds"]=time.time()-start
    result["versions"]={"numpy":np.__version__,"scipy":__import__("scipy").__version__}
    (HERE/"results/benchmark.json").write_text(json.dumps(result,indent=2,allow_nan=False))
    print("done",result["elapsed_seconds"],flush=True)
if __name__=="__main__":main()
