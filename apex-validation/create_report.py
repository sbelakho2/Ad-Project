from pathlib import Path
import json,math,re,textwrap
import numpy as np
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
from reportlab.pdfgen import canvas
from reportlab.platypus import SimpleDocTemplate,Paragraph,Spacer,Table,TableStyle,PageBreak,Image,KeepTogether
from reportlab.lib.styles import getSampleStyleSheet,ParagraphStyle
from reportlab.lib import colors
from reportlab.lib.enums import TA_LEFT
from reportlab.lib.pagesizes import A4
from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.ttfonts import TTFont
from xml.sax.saxutils import escape

ROOT=Path(__file__).resolve().parent
R=json.loads((ROOT/'results/benchmark.json').read_text())
I=json.loads((ROOT/'results/information-validation.json').read_text())
S=json.loads((ROOT/'results/switchback.json').read_text())
P=json.loads((ROOT/'results/production-parity.json').read_text())
OUT=ROOT/'output/pdf';OUT.mkdir(parents=True,exist_ok=True)
FONT='/usr/share/fonts/truetype/dejavu/'
pdfmetrics.registerFont(TTFont('DV',FONT+'DejaVuSans.ttf'))
pdfmetrics.registerFont(TTFont('DVB',FONT+'DejaVuSans-Bold.ttf'))
pdfmetrics.registerFontFamily('DV',normal='DV',bold='DVB',italic='DV',boldItalic='DVB')
INK=colors.HexColor('#142535');BLUE=colors.HexColor('#1558A6');GREY=colors.HexColor('#617080');LIGHT=colors.HexColor('#EFF3F7')
styles=getSampleStyleSheet()
styles.add(ParagraphStyle(name='TitleA',fontName='DVB',fontSize=27,leading=32,textColor=INK,spaceAfter=16))
styles.add(ParagraphStyle(name='HeadA',fontName='DVB',fontSize=19,leading=24,textColor=INK,spaceAfter=12))
styles.add(ParagraphStyle(name='SubA',fontName='DVB',fontSize=11.5,leading=16,textColor=BLUE,spaceBefore=10,spaceAfter=6))
styles.add(ParagraphStyle(name='BodyA',fontName='DV',fontSize=9.4,leading=13.7,textColor=INK,spaceAfter=8))
styles.add(ParagraphStyle(name='SmallA',fontName='DV',fontSize=8,leading=11.7,textColor=GREY,spaceAfter=7))
styles.add(ParagraphStyle(name='CellA',fontName='DV',fontSize=8.1,leading=11.3,textColor=INK))
styles.add(ParagraphStyle(name='LabelA',fontName='DVB',fontSize=8.5,leading=12,textColor=BLUE,spaceAfter=8))
story=[]
def para(t,style='BodyA'):story.append(Paragraph(t,styles[style]))
def heading(n,t):
    if story:story.append(PageBreak())
    para(f'APEXMEDIATION / VALIDATION {n:02d}','LabelA');para(t,'HeadA')
def sub(t):para(t,'SubA')
def table(rows,widths=None):
    data=[[Paragraph(str(c),styles['CellA']) for c in row]for row in rows]
    tb=Table(data,colWidths=widths or [503/len(rows[0])]*len(rows[0]),hAlign='LEFT',repeatRows=1)
    tb.setStyle(TableStyle([('BACKGROUND',(0,0),(-1,0),LIGHT),('VALIGN',(0,0),(-1,-1),'TOP'),('TOPPADDING',(0,0),(-1,-1),7),('BOTTOMPADDING',(0,0),(-1,-1),7),('LEFTPADDING',(0,0),(-1,-1),7),('RIGHTPADDING',(0,0),(-1,-1),7),('LINEBELOW',(0,0),(-1,0),.8,colors.HexColor('#B8C6D5')),('LINEBELOW',(0,1),(-1,-1),.35,colors.HexColor('#DAE1E9'))]))
    story.append(tb);story.append(Spacer(1,9))
def pct(x,d=2):return f'{100*x:.{d}f}%'
def cell(profile,q,e,method):
    prof=next(p for p in R['monte_carlo']if p['profile']['name']==profile)
    return next(x for x in prof['results']if x['q']==q and x['effect']==e and x['method']==method)
def inf(e,m):return next(x for x in I['results']if x['effect']==e and x['method']==m)
def interval(v):return f'{pct(v["rate"])} ({pct(v["ci95"][0])}-{pct(v["ci95"][1])})'

# Standalone scientific chart, based only on generated numerical results.
plt.rcParams.update({'font.family':'DejaVu Sans','font.size':9,'axes.spines.top':False,'axes.spines.right':False})
fig,axs=plt.subplots(1,2,figsize=(9.7,3.2),layout='constrained')
names=['moderate','concentrated','extreme_tail','low_pre_overlap'];labels=['Moderate','Concentrated','Extreme tail','Low pre overlap']
for ax,methods,q,title in [(axs[0],[('current_ht_90','Current HT 90%','#BD3F47'),('corrected_ht_975','Corrected HT 97.5%','#1961AA')],.25,'Initial estimator: no-effect changes'),(axs[1],[('current_holdout_full_roster','Current holdout + full roster','#BD3F47'),('roster_cuped_975','Proposed CUPED 97.5%','#28836F')],.9,'90/10 holdout: tail sensitivity')]:
    for j,(m,label,color)in enumerate(methods):
        vals=[cell(n,q,0.,m)['keep']for n in names];x=np.arange(4)+(j-.5)*.18
        y=np.array([v['rate']for v in vals])*100
        er=np.array([[v['rate']-v['ci95'][0]for v in vals],[v['ci95'][1]-v['rate']for v in vals]])*100
        ax.errorbar(x,y,yerr=er,fmt='o-',lw=1.2,ms=4,capsize=2,label=label,color=color)
    ax.set_xticks(range(4),labels,rotation=20,ha='right');ax.set_title(title,loc='left',fontweight='bold');ax.set_ylabel('False positive / keep (%)');ax.set_ylim(bottom=0);ax.grid(axis='y',alpha=.15);ax.legend(fontsize=7.5,loc='upper right')
axs[0].axhline(2.5,color='#6D7D8D',ls='--',lw=.7);axs[1].axhline(2.5,color='#6D7D8D',ls='--',lw=.7)
fig.savefig(OUT/'operating_characteristics.png',dpi=200);plt.close(fig)

heading(1,'What can be established now')
para('<b>Validate the software now. Establish economic lift on someone else\'s authorized publisher traffic.</b> Owning an app is unnecessary. A simulator can establish behavior against known truths; it cannot turn assumed bidder responses into observed publisher gains.')
para('Audit date: 28 October 2026. Reviewed GitHub commit: <b>685742e</b> (full SHA in the kit). This is a new review of the current no-SDK MAX / LevelPlay / AdMob autopilot, not a repetition of the 24 October report. No production files were changed.')
table([['Executed evidence','Result'],['Native Rust: lift, autopilot, connectors, proof, billing','916 passed; 2 ignored; no failures'],['Repository Python reference + holdout rerun tests','19 passed'],['MAX provider simulator HTTP contract suite','11 passed'],['Independent mathematics tests / native parity','4 passed / 63 fixtures agree'],['Repository Rust/Python twin','26 cases agree'],['Initial Monte Carlo grid','120,000 independent synthetic cohorts'],['Additional studies','2,000 larger cohorts; 200,000 sequential trials; 600,000 switchback trials; 1 million held-out auctions']], [265,238])
sub('Two defects that ordinary green tests miss')
para('<b>1. The new initial per-player variance is too small.</b> Under a completely unchanged population of 20,000 equal-revenue users, exact binomial enumeration gives a 26.934% false keep rate at a 25% split after the 1% point-lift and SRM filters. A native window-plus-decision replay actually keeps a null realization.')
para('<b>2. The ongoing holdout selects users after monetization.</b> Excluding people who receive no ads can turn zero true gain into +25% measured lift. The unmodified native monthly billing function then produces a synthetic $2,634.15 fee. No real invoice was issued.')
para('The good result is conditional: with a complete eligible-user roster, 600,000 users, a balanced split and CUPED, +3% power reached 92.75% (95% interval 91.52-93.85%). That does not clear every app or the current 90/10 design.','SmallA')

heading(2,'An exact oracle for the variance')
para('Let q be the planned treatment probability, p = 1 - q, X_t and X_c the observed revenue totals, and S_t and S_c the corresponding sums of squared per-user window revenues. The implemented planned-share estimator is:')
para('<b>L = log[(X_t / q) / (X_c / p)]</b>')
para('For independent Bernoulli user assignment, the two expanded arm totals have negative covariance. The current code both omits this covariance and substitutes observed-arm sums into population variance formulae without the extra inverse-probability correction. The implemented variance simplifies to:')
para('<b>V_current = pq (S_t / X_t² + S_c / X_c²)</b>')
para('An observable, conservative first-order Cauchy bound for that log ratio is:')
para('<b>V_bound = [sqrt(p S_t / X_t²) + sqrt(q S_c / X_c²)]²</b>')
para('Use this for a justified Bernoulli design, or use eligible-user arm means with user-clustered inference when a complete roster exists. This correction is an asymptotic plug-in bound, not an exact finite-sample confidence guarantee. Heavy-tail tests on page 4 show why the distinction matters. Randomization and analysis units must align [1].')
sub('A null that needs no advertising assumptions')
para('Every eligible user has revenue 1 under either configuration. N_t follows Binomial(20,000, q). Enumerate every possible N_t; no estimated retention curve, spending distribution or auction response is involved.')
table([['Test share','Current bound positive','Current final keep','Corrected 97.5% final keep'],['25%', '29.184%','26.934%','2.425%'],['50%','25.996%','24.145%','2.457%']], [84,139,139,141])
para('Final keep here includes point lift at least 1% and chi-square at most 10.8276. There is one final look; no lifecycle-wide rate is claimed. The current bound is one-sided 90%; the repair uses one-sided 97.5% to reserve error for another planned look.')
para('At a 25% split, the current standard error is about 0.707%; the correct first-order null standard error is 1.633%. In the native witness, 5,100 users fall into test and 14,900 into control; both configurations pay every user exactly the same amount. The current public window/guardrail/decision stack nevertheless returns Keep.','SmallA')

heading(3,'The independent sensitivity grid')
para('The initial plan was saved before its run. Four spending / pre-period profiles, 20,000 eligible users, three shares and 10,000 independent cohorts per cell generate 120,000 cohorts. Effects -5%, -2%, 0%, +1%, +3%, +5% share each cohort\'s potential outcomes, making method comparisons paired. They are not 720,000 independent datasets.')
para('A persistent lognormal user value drives both periods; independent window shocks and optional pre-period absence / post-period zeros vary predictability. These are deliberately specified sensitivity models, <b>not a calibrated publisher twin</b>. The historical CUPED slope is fitted on a separate 100,000-user sample. No benchmark statistic was used to assert that this represents a particular app.')
table([['Profile','Persistent sigma / shock sigma','Pre zeros / post zeros','Historical correlation'],['Moderate','0.80 / 0.40','0% / 0%', '0.730'],['Concentrated','1.50 / 0.75','0% / 0%','0.538'],['Extreme tail','2.15 / 0.80','0% / 0%','0.520'],['Low pre overlap','1.50 / 0.75','75% / 30%','0.264']], [126,140,119,118])
table([['25% initial split','Current null keeps','Corrected HT null keeps','CUPED power at +3%']]+[[lab,pct(cell(n,.25,0.,'current_ht_90')['keep']['rate']),pct(cell(n,.25,0.,'corrected_ht_975')['keep']['rate']),pct(cell(n,.25,.03,'roster_cuped_975')['keep']['rate'])]for n,lab in zip(names,labels)], [145,119,120,119])
para('Every displayed Monte Carlo interval in the machine-readable results is a pointwise 95% Clopper-Pearson interval over independent trials. It measures simulation sampling error, not realism or all-cell simultaneous confidence. The full grid uses independent formulae; 63 native fixtures validate implementation parity. Every one of the large-grid trials was not executed through the native runner.','SmallA')
para('HT at a 90% share is estimator sensitivity, not a reachable default initial-keep design. Holdout results correspond to production only at 90%; other holdout splits in the raw file are hypothetical comparisons. Daily stops are not replayed in this estimator grid.','SmallA')

heading(4,'The repair also needs a validity gate')
story.append(Image(str(OUT/'operating_characteristics.png'),width=503,height=166))
para('A corrected formula does not automatically give reliable small-holdout confidence intervals. At 90/10, concentrated revenue leaves too little independent information in the old-setting arm. Even supplying the full eligible roster does not cure a poor normal approximation.')
table([['90/10 null, full roster','Current holdout, 1.697 cutoff','Proposed CUPED, 1.960 cutoff']]+[[lab,interval(cell(n,.9,0.,'current_holdout_full_roster')['keep']),interval(cell(n,.9,0.,'roster_cuped_975')['keep'])]for n,lab in zip(names,labels)],[145,179,179])
para('The current holdout cutoff corresponds to about 4.5% one-sided error asymptotically, despite its "90%" naming. In the extreme-tail profile its observed null fee / positive-bound rate is 10.75%. The proposed 2.5% one-sided CUPED test still falsely keeps 9.52%. These cells do not pass.')
sub('What to implement')
para('Make insufficient information a durable <b>No measurement / No fee</b> state. Register the randomization law, estimand, eligible population, maximum looks and method before starting. Validate tail concentration and small-arm information on independent null replays; do not substitute "30 users" for statistical adequacy.')
para('Use a larger control arm and more unique users where power permits. Exact randomization tests are useful for a sharp no-effect null; they do not by themselves supply a valid lower bound for average revenue under heterogeneous effects. A bounded-outcome method needs a defensible bound and explicit tail treatment; clipping silently changes the financial estimand.','SmallA')

heading(5,'A tested route to useful power')
para('After the original grid exposed low power, a new 600,000-user, 50/50 design was locked and validated on an independent seed. It uses the same concentrated-revenue model, 2,000 fresh cohorts, and a separately trained historical CUPED coefficient. This is a conditional feasibility study, not a retroactive first-grid preregistration.')
table([['600,000 eligible users','No-effect keep rate','Power at true +3%'],['Current HT 90%',interval(inf(0.,'current_ht_90')['keep']),interval(inf(.03,'current_ht_90')['keep'])],['Corrected HT 97.5%',interval(inf(0.,'corrected_ht_975')['keep']),interval(inf(.03,'corrected_ht_975')['keep'])],['Eligible means 97.5%',interval(inf(0.,'roster_unadjusted_975')['keep']),interval(inf(.03,'roster_unadjusted_975')['keep'])],['Eligible means + CUPED 97.5%',interval(inf(0.,'roster_cuped_975')['keep']),interval(inf(.03,'roster_cuped_975')['keep'])]],[178,162,163])
para('The CUPED method kept 0 of 2,000 simulated -2% losers; the two-sided 95% upper bound is 0.184%, not zero. Its null upper bound is 3.845%, supporting a 5% fixed-look target in this model. It does <b>not</b> certify a 2.5% maximum or validity across all populations.')
sub('Plan for information, not a fixed number of days')
para('For a roughly stable coefficient of variation CV and a balanced / unbalanced share q, the usual small-effect planning approximation is:')
para('<b>N ≈ [(z_(1-alpha) + z_power) CV / log(1 + delta)]² / [q(1-q)]</b>')
para('With a useful pre-period covariate, the idealized variance multiplier is 1 - rho². Measure rho on the actual metric; the approximately 50% reduction reported in the original Bing CUPED research is not a universal mobile-app statistic [2]. A 90/10 split needs about 2.78 times the users of a 50/50 split for the same variance when arm variances are similar.')
para('These are unique eligible users over the measurement window, not DAU. More days do not create new independent users when the same people persist. A publisher must supply the information needed to make the chosen measurement valid.','SmallA')

heading(6,'The zero-gain monthly billing witness')
para('Define potential outcomes before assignment. Of 100,000 eligible users, 40% have old/new revenues $1/$1.25; 10% have $1/$0; 50% have $0/$0. A stratified realized 90/10 assignment yields these exact figures:')
table([['Quantity','Kept setting','Old setting'],['Assigned users','90,000','10,000'],['Users appearing in impression feed','36,000','5,000'],['Total revenue','$45,000','$5,000'],['Revenue per assigned user','$0.50','$0.50'],['Revenue per observed ad user','$1.25','$1.00']],[249,127,127])
para('<b>True gain is zero. The current holdout reports +25% with standard error zero.</b> Its observed exposure share is 36,000/41,000 = 87.805%, inside the permitted 90% ± 3 percentage-point range. Identified revenue coverage is 100%, both observed arms exceed 30 users, and rows can cover all 28 days.')
table([['Executed unmodified native function','Synthetic fee'],['holdout::monthly_fee, actual exposed revenue $45,000','$2,700.00'],['run::run_holdout_month, whole-scope revenue $50,000','$2,634.15']],[359,144])
para('The two fees differ because the monthly function treats the fraction of observed ad users as the fraction of total revenue. A user fraction is not a revenue fraction when revenue differs between arms. A variant with $1.20 new revenue is truly harmful (-4%) but still reports +20%.')
sub('Fix both denominators')
para('Use a predefined eligible assignment roster including zero-revenue users, or a justified planned-share total-revenue estimator. Use assignment counts for SRM. Feed counts of positive-impression users cannot replace that roster. Attribute fees to actual scoped exposed revenue for the billed interval, not observed-player share times scope revenue.')
para('This executes the native estimator and core monthly billing function, not a live publisher invoice workflow. Current application/ad-unit scope mismatches and LevelPlay\'s short history can block live fees; those gates must be preserved while correcting the statistics.','SmallA')

heading(7,'Repeated looks and exact switchbacks')
para('A nominal bound is for its stated look. An interim, a selected extension, monthly windows and many simultaneous proposals consume additional error. In 200,000 Gaussian null trials isolating the look schedule, the 99% interim / 90% final / selected 90% extension rule makes at least one false rejection in <b>14.272%</b> (95% interval 14.119-14.426%).')
para('Allocate alpha = 0.005, 0.0225, 0.0225 to the three looks. The union bound is 5% when each test is calibrated. The simulated error falls to <b>3.7135%</b> (3.6310-3.7973%). This is an idealized sequential study, not the full current runner\'s false-keep rate. Error spending cannot repair the heavy-tail failures on page 4. Confidence sequences are an alternative, with their own assumptions [4].')
sub('An unusually small exact oracle already exists')
para('The current 14-block switchback planner permits exactly <b>30 schedules</b> after weekday balancing and the no-three-in-a-row rule. Enumerating them gives a finite randomization distribution for a sharp no-effect null, without assuming independent block outcomes.')
table([['AR(1) block correlation','Current Welch 90%, no trend','Exact 5%, no trend'],['0.0','9.637%','3.358%'],['0.5','5.895%','3.372%'],['0.9','3.828%','3.321%']], [173,165,165])
para('Each row uses 100,000 trials. Another 300,000 add a log trend of 0.01 per block. Exact-test rates stay near 1/30. Its minimum nonrandomized one-sided p-value is 1/30 = 3.333%, so a 2.5% gate can <b>never reject</b> with this design. Expand the assignment space / horizon before demanding that threshold.')
para('A six-hour washout is an assumption, not evidence that bidders settle in six hours. Model carryover explicitly and validate its horizon. Exact sharp-null p-values do not automatically justify average-lift billing intervals; spillover and carryover affect the estimand [8].','SmallA')

heading(8,'Do not bake lift into an auction twin')
para('Three independent model assumptions were exercised on the same 200,000 training auctions and 1,000,000 held-out auctions. The floor was selected on training only. All models use three lognormal bidder values; budgets, adaptation, retention and cross-arm learning are absent.')
table([['Hypothetical market','Chosen floor, baseline 0.6','Held-out gain, 95% CI','Raise floor 20%'],['Fixed first-price bids','0.0','2.691% (2.672-2.709%)','-2.376%'],['Truthful second-price','1.2','2.751% (2.686-2.817%)','+0.851%'],['First-price, imposed reserve response','0.8','1.641% (1.613-1.668%)','+1.297%']], [157,107,139,100])
para('<b>These gains are not ApexMediation lift forecasts.</b> They show that profitable candidate direction changes with market mechanism. With fixed first-price bids and no alternative fallback, raising the reserve cannot increase the payment of an auction that still clears: it can only leave that payment or remove it. A simulator that mechanically raises the winner\'s first-price payment with the reserve manufactures lift.')
para('The Yahoo reserve-price field experiment reports a 2.7% adjusted aggregate gain in its sponsored-search setting [9]. That is evidence that reserve optimization can work there, not a distribution from which modern mobile mediation lift can be copied. The iPinYou documentation describes primarily second-price auctions [10]; replayed DSP winning prices do not reveal every counterfactual publisher revenue or bidder response.')
sub('Make the economics test adversarial')
para('Include demand contraction, strategic shading, changing fallback quality, bidder budgets, timeouts, network eligibility and long-run bidder learning. Keep a never-tune-on-test market and require comparisons with no change, lowering floors, existing manual rules and candidate selection without the learning model. A profitable simulator result is a sensitivity result until connected to real experiments.')
para('For historical policy evaluation, log the candidate set and actual action probabilities at decision time. Doubly robust off-policy evaluation requires overlap / supported actions and suitable logging assumptions; inventing propensities for deterministic history cannot identify untried floors [11].','SmallA')

heading(9,'Exact changes: statistics and billing')
table([['Priority / location','Concrete change and acceptance witness'],['P0 - apex-lift/src/stats.rs; Python reference','Replace the HT variance with the derived design-aware expression. Pin an estimator ID. Add the exact binomial oracle and the native A/A witness; matching a copied reference is insufficient.'],['P0 - holdout.rs: estimate / from_player_feed','Define eligibility before treatment. Include eligible zeros or use planned-share totals with valid design-based inference. The 100,000-user zero-gain witness must have no positive billable lower bound.'],['P0 - run.rs: treated_share / attribution','Carry scoped exposed revenue by billed interval. Do not approximate it with observed monetized-player share. Both direct fee and monthly-run rerun must use the same basis to the micro.'],['P0 - settle.rs: precise estimate / .ok fallback','Register the statistic before collecting outcomes. A missing, invalid or contradictory registered feed pauses / voids; do not silently switch to daily ratios when per-user evidence or an estimator fails.'],['P0 - player_store.rs / postbacks.rs','CUPED residuals can be negative. Use a signed-outcome-capable estimator with proper eligible population and pre-period handling. Negative residuals must not trigger the nonnegative raw-revenue rejection and fallback.'],['P1 - decision.rs / guardrails.rs / holdout.rs','Store one error budget for interim, final, extension and repeated harm / billing looks. Harm must use a valid clustered statistic. Current day-7/day-14 checks still estimate daily ratios.'],['P1 - guardrails.rs / calibration.rs','Use assigned unique players for SRM; pooled player-days and averaged daily counts are not binomial assignment counts. A/A calibration and sample planning need the same clustering as the decision.']],[157,346])
para('Do not enable or loosen a capability / coverage gate to make these tests pass. The current source remains unmodified by this review; the runnable kit contains evidence and acceptance examples, not a production implementation.','SmallA')

heading(10,'Exact changes: obtaining sufficient evidence')
table([['Location','Concrete change'],['live_max.rs; live_levelplay.rs; durable store','Both live adapters currently return an empty pretest map. Persist daily source files and immutable observations; attach the A/A pre-period to the same stable identity and assignment epoch. Do not fit CUPED on cherry-picked monetized survivors.'],['LevelPlay player-history path','The provider retains 14 days, while the ongoing holdout requires 28 complete days. A fresh in-memory feed cannot supply that window. Pull daily after 10:00 UTC, re-pull D-2, retain the stable reconciled history locally [6].'],['MAX scope and identity parser','The user-revenue endpoint has a 45-day request window. Nonaggregated rows include ad-unit and Waterfall fields; retain the causal ad-unit scope. Waterfall name is not by itself proof of experimental assignment semantics [7].'],['Platform capability report','Publish independent statuses for scoped outcomes, arm labels, assignment roster, pre-period, finality, retention and monetary precision. Application-scoped evidence must not masquerade as unit-scoped measurement.'],['Tier / runner policy and public claims','AdMob arm-level measured lift is unavailable on the current path; keep no-fee behavior. Reflect the current 30% ongoing holdout model consistently; the README still describes the retired 15-20% snapshot model.'],['switchback.rs; validation toolchain','Use the actual admissible assignment law; enumerate the 30-schedule sharp-null oracle. Add a larger design if a 2.5% exact test is required. Validate carryover rather than assuming a six-hour horizon.']],[157,346])
sub('Evidence integrity is part of the statistical test')
para('The evidence key should include publisher, platform, app, ad unit, experiment instance, arm-assignment epoch, UTC date, currency, source schema and revision. Persist raw-hash provenance plus accepted revisions. Re-pulls replace a scoped day rather than double-counting; a restart or mid-test split edit must not silently redefine the cohort.')
para('Official LevelPlay documentation says yesterday becomes available after 10:00 UTC and is updated again the following day; files are available for only 14 days. MAX says its user reports become available after 08:00 UTC and offers a 45-day window [6,7]. These are operational constraints to implement, not assumed source completeness.')

heading(11,'Testing without an app: the executable program')
table([['Layer','What can be established','This review'],['1. Independent oracles','Exact null / invariance / adversarial billing semantics','Executed'],['2. Native differential checks','Unmodified Rust code agrees with independent numeric inputs','63 fixtures; 26 repo twin cases'],['3. Statistical operating curves','False keeps, coverage and power under declared worlds','120,000 + 2,000 cohorts'],['4. Sequential and assignment law','Look inflation; exact schedule distribution','200,000 + 600,000 trials'],['5. Provider-sim integration','HTTP shape and faults; lifecycle mechanics with recorded truth','MAX 11 tests; native autopilot suite. Full external HTTP truth-to-invoice chaos replay remains to build'],['6. Real read-only replay','Noise, identities, coverage and calibration on a prospect\'s authorized exports','Tool delivered; no prospect data accessed'],['7. Real treatment / holdout','Platform assignment, bidder response and sustained economic lift','Needs an authorized publisher; owning an app is unnecessary']],[58,253,192])
sub('The highest-value next acquisition is data access')
para('Offer design partners a fee-free approval-mode pilot: an A/A check, then a preregistered native experiment, then a persistent holdout with the corrected metric. Obtain authorized real data through the free audit or a data-sharing partner. Historical native A/B exports are more useful than buying install traffic for a purpose-built demo app.')
para('The delivered rerandomize.py consumes a complete eligible-user outcome roster, generates sticky Bernoulli splits and returns false-keep / injected-power rates with exact Monte Carlo intervals. An impression-only file cannot certify its missing zero users. Re-randomization checks a model of assignment; it does not verify the platform\'s actual assignment or supply an unobserved treatment outcome.')
para('Do not substitute simulated gains, a read-only shadow recommendation, a public second-price log, or network test-mode inventory for a real causal lift claim. They establish different pieces of evidence.','SmallA')

heading(12,'Acceptance gates and handoff')
para('Set the target before judging a run. For example, a fixed-look validation target may be false keep ≤5%, harmful-change keep ≤1%, and +3% power ≥80%. The Monte Carlo <b>upper</b> confidence bound must clear an error ceiling; the <b>lower</b> confidence bound must clear a power floor. An observed 10.8% with an interval spanning 10% is not proof of a ≤10% requirement.')
table([['Gate','Required behavior'],['Inference','Exact constant-revenue oracle passes; tail / skew / missing-zero scenarios either calibrate or return No fee. Validate the combined look schedule, not each look alone.'],['Coverage and causality','No bill when scope, epoch, assigned population or identity coverage is unknown. Unknown eligible zeros are a capability gap, not implicitly zero missingness.'],['Accounting','Zero / negative true gain witnesses do not bill. No billing on void, rollback, incomplete feed or unconfirmed state. Same scoped data reruns to identical micros.'],['Chaos','Retry/restart; D-2 restatement; lost and duplicated day; UTC boundary; 0.5% finality edge; cross-arm ID; publisher edit; split change; partial file; out-of-order page; failed holdout opening; concurrent job.'],['Independence','Keep calibration/tuning seeds separate from validation. Store per-trial outcomes for paired comparisons. Use multiplicity-adjusted simulation intervals when asserting a joint release gate.'],['Release','Run the repository\'s diff-scoped implementation, independent verifier, mutation and relevant live gates after a production fix. This read-only review does not claim those change gates were completed.']],[105,398])
sub('What the deliverables let you do immediately')
para('Reproduce every reported result; run the native zero-lift decision and false-fee witnesses; feed authorized user rosters into the rerandomization tool; implement the listed source changes against exact regression examples; and turn provider simulators into a declared-truth release harness.')
para('<b>The current engine is not cleared for arbitrary live lift billing.</b> The conditional high-information test demonstrates that high power and controlled false positives are compatible. The remaining work is to make the live evidence and registered inference satisfy those conditions, then measure real economics on publisher traffic.')

heading(13,'Research and source trail')
refs=[
('1. Deng, Knoblich & Lu (2018)','Applying the Delta Method in Metric Analytics. Randomization / aggregation units and ratio inference.','https://alexdeng.github.io/public/files/kdd2018-dm.pdf'),
('2. Deng, Xu, Kohavi & Walker (2013)','CUPED. The paper reports about 50% variance reduction at Bing; transfer to a new app must be measured.','https://ai.stanford.edu/~ronnyk/2013-02CUPEDImprovingSensitivityOfControlledExperiments.pdf'),
('3. Lin (2013)','Agnostic notes on regression adjustments. Treatment interactions and robust sandwich inference, under large-sample conditions.','https://arxiv.org/abs/1208.2301'),
('4. Howard, Ramdas, McAuliffe & Sekhon (2021)','Time-uniform confidence sequences. Optional stopping with explicit assumptions.','https://arxiv.org/abs/1810.08240'),
('5. Microsoft Experimentation Platform (2020)','Diagnosing Sample Ratio Mismatch. Missing users and conditioning can reverse conclusions; statistical detection requires counts.','https://www.microsoft.com/en-us/research/articles/diagnosing-sample-ratio-mismatch-in-a-b-testing/'),
('6. Unity LevelPlay documentation','Impression-level server API, AB Testing field, D-2 restatements and 14-day retention.','https://docs.unity.com/en-us/grow/levelplay/platform/api/impression-level-revenue-server-side'),
('7. AppLovin MAX documentation','User-Level Ad Revenue API. Availability, request window, identities, ad units and Waterfall field.','https://support.applovin.com/en/max/reporting-apis/user-level-ad-revenue-api'),
('8. Bojinov, Simchi-Levi & Zhao (2023)','Design and Analysis of Switchback Experiments. Assignment-law inference and carryover.','https://arxiv.org/abs/2009.00148'),
('9. Ostrovsky & Schwarz','Reserve Prices in Internet Advertising Auctions: A Field Experiment. Sponsored-search field evidence, not mobile calibration.','https://web.stanford.edu/~ost/papers/rp1.pdf'),
('10. iPinYou dataset documentation','Describes the auction / log process; counterfactual limitations must be respected.','https://contest.ipinyou.com/ipinyou-dataset.pdf'),
('11. Dudik, Langford & Li (2011)','Doubly Robust Policy Evaluation and Learning. Reward / logging model assumptions and policy support.','https://arxiv.org/abs/1103.4601')]
for title,desc,url in refs:
    para(f'<b>{escape(title)}</b> - {escape(desc)}','SmallA')
    para(f'<link href="{url}" color="#1558A6">{escape(url)}</link>','SmallA')
para('Production source: https://github.com/sbelakho2/Ad-Project/tree/685742ea22bc40f3c78404e4095c1cbf53806bad. Full commands, seeds, independent review and raw test logs are in the companion kit. All claimed experimental results in this report are from executed code; none are publisher revenue observations.','SmallA')

def footer(c,doc):
    c.setStrokeColor(colors.HexColor('#D8E1EA'));c.line(46,39,549,39)
    c.setFont('DV',7.3);c.setFillColor(GREY);c.drawString(46,27,'APEXMEDIATION  |  Independent validation  |  28 October 2026')
    c.drawRightString(549,27,str(doc.page))

doc=SimpleDocTemplate(str(OUT/'ApexMediation_Validation.pdf'),pagesize=A4,rightMargin=46,leftMargin=46,topMargin=44,bottomMargin=52,title='ApexMediation: proving the engine before publisher lift',author='Independent technical review')
doc.build(story,onFirstPage=footer,onLaterPages=footer)
print(OUT/'ApexMediation_Validation.pdf')
