"""Zero assigned-user lift, positive feed-only lift: deterministic witness.

The potential-outcome population has 40% (Y0=1,Y1=1.25), 10%
(Y0=1,Y1=0), 50% (Y0=0,Y1=0). Every arm has this same composition.
Both potential revenue means equal .50. Non-monetizing users do not appear
in an impression revenue feed. The fixture supplies complete 28-day data,
100% identity coverage, and a single matching causal scope.
"""
import json
from pathlib import Path
import subprocess
import sys

HERE = Path(__file__).resolve().parent
# Relocated into the deliverable's verifier/ folder; mathematics unchanged.
ROOT = HERE.parents[1]/'apexmediation'
rows=[]
for prefix,count,test,revenue in [('e',36000,True,1250000),('c',5000,False,1000000)]:
    for i in range(count):
        rows.append({'day':f'2026-10-{1+i%28:02}', 'userId':f'{prefix}{i}',
                     'idfv':None,'idfa':None,'test':test,'revenueMicros':revenue})
fixture={'monthEnd':'2026-10-28','exposedIsTest':True,
         'measurementScope':'unit1',
         'windowDays':[f'2026-10-{i:02}' for i in range(1,29)],
         'rows':rows,'pretest':{},'unidentifiedMicros':0,'unidentifiedShareBp':0}
fixture_path=HERE/'holdout_zero_true_lift.json'
fixture_path.write_text(json.dumps(fixture,separators=(',',':'))+'\n')
call=subprocess.run([sys.executable,str(ROOT/'site/static/rerun-kit/holdout_rerun.py'),
                     str(fixture_path),'--treated-revenue','45000000000'],
                    capture_output=True,text=True,check=True)
shipped=json.loads(call.stdout)
share=36000/41000
total_scope=50_000_000_000
treated_base=round(total_scope*share)
incremental=round(treated_base*(1-1/(1+.25)))
monthly_run_fee=round(.30*incremental)
result={
    'head':'685742ea22bc40f3c78404e4095c1cbf53806bad',
    'true_eligible_counts':{'exposed':90000,'control':10000},
    'true_revenue_per_assigned_user':{'exposed':.50,'control':.50},
    'true_lift':0.0,
    'observed_revenue_players':{'exposed':36000,'control':5000},
    'shipped_rerun_output':shipped,
    'source_checked_runner_gates':{
        'scope_matches':True, 'complete_window_days':28,
        'all_identifiers_present':True, 'cross_arm_ids':0,
        'both_arms_over_30':True, 'observed_exposed_share':share,
        'split_90_10_tolerance_passes':abs(share-.9)<=.03,
        'confident_harm':False,
        'assumed_active_confirmed_holdout_with_valid_ledger':True,
    },
    'full_month_run_formula_without_native_execution':{
        'scope_revenue_micros':total_scope,
        'treated_base_micros':treated_base,
        'incremental_micros':incremental,
        'fee_micros':monthly_run_fee,
        'fee_usd':monthly_run_fee/1e6,
    },
    'limits':'Ran shipped Python rerun; full Rust runner was source checked, not executed. Live adapters currently mismatch application/ad-unit scope and LevelPlay has only 14-day feed; those prevent a current live fee rather than fix the estimator.'
}
(HERE/'verify_holdout_selection_results.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps(result,indent=2))
