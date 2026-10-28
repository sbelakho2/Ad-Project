#!/usr/bin/env python3
"""Locked new-seed power validation after the original grid exposed low power.

The sample size is deliberately increased, not the false-positive threshold.
This is a conditional feasibility result for one stated synthetic profile.
"""
import json,time
from pathlib import Path
from benchmark import monte

HERE=Path(__file__).resolve().parent
plan={"players":600000,"trials":2000,"batch":4,"shares":[.5]}
profile={"name":"concentrated_more_information","sigma":1.5,"noise":.75,"prezero":0.,"postzero":0.,"seed":5139912}
if __name__=="__main__":
    (HERE/"results/information-plan.json").write_text(json.dumps({"plan":plan,"profile":profile},indent=2))
    start=time.time();r=monte(profile,plan);r["plan"]=plan;r["elapsed_seconds"]=time.time()-start
    (HERE/"results/information-validation.json").write_text(json.dumps(r,indent=2))
    print(json.dumps(r,indent=2))
