#!/usr/bin/env python3
"""Replay an A/A population through the current native window+decision code."""
import gzip,json,subprocess,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parent
REPO=ROOT.parent/"apexmediation"
sys.path.insert(0,str(REPO/"tools/lift-validation"))
from cases import arm_day,prereg,window,_day_strings

def main():
    nt,nc=5100,14900
    dates=_day_strings(17)
    control=[arm_day(d,nc*1000000,users=nc,fill=.7) for d in dates]
    test=[arm_day(d,nt*1000000,users=nt,fill=.7) for d in dates]
    rows=[]
    for d in dates[3:]:
        for test_arm,n,prefix in [(False,nc,"c"),(True,nt,"t")]:
            rows.extend({"day":d,"userId":f"{prefix}{i}","idfv":None,"idfa":None,
                         "test":test_arm,"revenueMicros":1000000} for i in range(n))
    case={"id":"constant-revenue-AA-5100-of-20000","prereg":prereg(interimLook=False,extensionAllowed=False),
          "window":window(control,test),"players":{"rows":rows,"pretest":{},"cuped":False}}
    raw=json.dumps(case,separators=(",",":")).encode()
    with gzip.open(ROOT/"results/native-AA-witness.json.gz","wb") as f:f.write(raw)
    binary=REPO/"rust/target/debug/examples/apex-lift-twin"
    result=subprocess.run([str(binary)],input=raw,capture_output=True,check=True)
    out=json.loads(result.stdout)
    (ROOT/"results/native-AA-decision.json").write_text(json.dumps(out,indent=2))
    assert out["decision"]["verdict"]=="keep",out
    print(json.dumps({"true_lift":0.,"assigned_test":nt,"assigned_control":nc,
                      "decision":out["decision"],"playerEstimate":out["playerEstimate"],
                      "guardrails":out["guardrails"]},indent=2))
if __name__=="__main__":main()
