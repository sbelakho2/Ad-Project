"""Independent null counterexample for committed apex-lift HT statistic.

Every eligible player has fixed potential revenue 1 in both arms. Assignment
is independent Bernoulli(q), held fixed through the window. The exact null
law is binomial; no auction/retention/industry assumption is needed.
"""
import json
from pathlib import Path
import numpy as np
from scipy.stats import binom, norm

N = 20_000
Z90 = 1.2815515655446004
SEED = 2026102807
REPS = 1_000_000


def wilson(k, n):
    z = norm.ppf(.975)
    p = k/n
    d = 1+z*z/n
    mid = (p+z*z/(2*n))/d
    half = z*np.sqrt(p*(1-p)/n+z*z/(4*n*n))/d
    return [float(mid-half), float(mid+half)]


def masks(nt, q):
    nc = N-nt
    logratio = np.log(nt*(1-q)/(nc*q))
    # Direct algebraic simplification of stats.rs 287–291 at y_i=1.
    code_se = np.sqrt(q*(1-q)*(1/nt+1/nc))
    lower = np.expm1(logratio-Z90*code_se)
    point = np.expm1(logratio)
    chi2 = (nt-N*q)**2/(N*q*(1-q))
    return lower > 0, (lower > 0)&(point >= .01)&(chi2 <= 10.8276)


results=[]
for q in (.25, .5):
    ks=np.arange(1,N)
    nominal, keep=masks(ks.astype(float), q)
    masses=binom.pmf(ks,N,q)
    draws=np.random.default_rng(SEED+int(q*100)).binomial(N,q,REPS)
    a,b=masks(draws.astype(float),q)
    results.append({
        'q': q, 'N': N, 'reps': REPS,
        'code_se_at_expected_counts': float(np.sqrt(1/N)),
        'correct_randomization_se_first_order':float(np.sqrt(1/(N*q*(1-q)))),
        'exact_nominal_rejection':float(np.sum(masses[nominal])),
        'exact_keep_after_minimum_1pct_and_srm':float(np.sum(masses[keep])),
        'mc_nominal':float(a.mean()), 'mc_nominal_ci95':wilson(int(a.sum()),REPS),
        'mc_keep':float(b.mean()), 'mc_keep_ci95':wilson(int(b.sum()),REPS),
    })
result={'head':'685742ea22bc40f3c78404e4095c1cbf53806bad', 'seed':SEED, 'results':results}
Path(__file__).with_name('verify_ht_results.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps(result,indent=2))
