# 🎯 Sales Automation Quick Reference (V1 - Cialdini)

> **📢 For Outbound Prospecting:** See [Autonomous Sales V2](AUTONOMOUS_SALES_V2.md)

## The 6 Principles (1-Page Cheat Sheet)

### 1. RECIPROCITY
**Rule:** Give first, get later  
**Application:** Day 0-6 gifts ($2,000+ value)  
**Result:** 15% conversion lift  
**Table:** `reciprocity_gifts`

### 2. COMMITMENT & CONSISTENCY  
**Rule:** Small yeses lead to big yeses  
**Application:** 5-15 micro-commitments during trial  
**Result:** 20% conversion lift (strongest)  
**Table:** `commitment_milestones`

### 3. SOCIAL PROOF
**Rule:** Follow the crowd  
**Application:** Case studies, testimonials, usage stats  
**Result:** 12% conversion lift  
**Table:** `social_proof_events`

### 4. AUTHORITY
**Rule:** Trust the experts  
**Application:** Data citations, credentials, expertise  
**Result:** 8% conversion lift  
**Table:** `authority_signals`

### 5. LIKING
**Rule:** Say yes to those like us  
**Application:** Similarity matching, genuine compliments  
**Result:** 7% conversion lift  
**Applied:** Throughout all touchpoints

### 6. SCARCITY
**Rule:** Want what's running out  
**Application:** Trial ending countdown, limited bonuses  
**Result:** 18% conversion lift  
**Table:** `scarcity_triggers`

---

## 14-Day Sequence (Timeline)

| Day | Action | Principle | Expected Behavior |
|-----|--------|-----------|-------------------|
| 0 | Welcome gift | Reciprocity | Open rate: 60% |
| 1 | Benchmark report | Authority | Build trust |
| 3 | Surprise bonus | Reciprocity | Engagement spike |
| 6 | Premium unlock | Reciprocity | Feature usage begins |
| 8 | Case study | Social Proof | Inspiration |
| 11 | 3-day warning | Scarcity | Urgency kicks in |
| 13 | Final push | ALL 6 | Conversion spike |
| 14 | Trial ends | - | Convert or nurture |

---

## Key SQL Queries

### Check Conversion Rate
```sql
SELECT 
  COUNT(DISTINCT CASE WHEN subscription_status = 'active' THEN id END)::FLOAT /
  COUNT(DISTINCT id) * 100 as conversion_rate_percent
FROM publishers
WHERE created_at > NOW() - INTERVAL '30 days';
```

### Best Performing Principle
```sql
SELECT * FROM principle_effectiveness 
ORDER BY conversion_rate DESC LIMIT 1;
```

### Sales Funnel Overview
```sql
SELECT * FROM sales_funnel;
```

### Recent Conversions
```sql
SELECT 
  customer_id,
  conversion_type,
  mrr_increase,
  days_in_trial,
  primary_influence_principle
FROM sales_conversions
WHERE converted_at > NOW() - INTERVAL '7 days'
ORDER BY converted_at DESC;
```

---

## Manual Triggers (Testing)

### Run Sales Automation Now
```bash
node dist/services/sales/InfluenceBasedSalesService.js
```

### Track Custom Milestone
```typescript
await influenceBasedSalesService.trackCommitmentMilestone(
  customerId,
  'feature_used',
  'First ad optimization',
  'easy',
  10 // minutes
);
```

### Check Customer Probability
```sql
SELECT 
  email,
  calculate_engagement_score(id) as engagement,
  predict_conversion_probability(id) as probability
FROM publishers
WHERE subscription_status = 'trial';
```

---

## Monitoring Dashboard

### Daily Checks (2 minutes)
1. Conversion rate: Target 40-45%
2. Touchpoints sent: Should match trial customer count
3. Any errors in logs: `grep ERROR backend.log | grep Sales`

### Weekly Reviews (5 minutes)
1. Principle effectiveness: Which is winning?
2. A/B test results: Any clear winners?
3. Journey stage distribution: Any bottlenecks?
4. Conversion by day: Is Day 13 push working?

### Monthly Optimization (15 minutes)
1. Run AI optimization: GPT-4o-mini suggestions
2. Implement winning A/B tests
3. Adjust timing based on open rates
4. Refine segmentation

---

## Common Issues & Fixes

### Issue: Low conversion rate (<30%)
**Diagnosis:** Check `principle_effectiveness` table  
**Fix:** Emphasize best-performing principle more

### Issue: Low engagement scores
**Diagnosis:** Check `commitment_milestones` completion rate  
**Fix:** Make onboarding easier, reduce friction

### Issue: High Day 14 churn
**Diagnosis:** Scarcity not triggering urgency  
**Fix:** Strengthen Day 11-13 messaging, add more specificity

### Issue: Touchpoints not sending
**Diagnosis:** Check cron job status  
**Fix:** `ps aux | grep cron-jobs` should show process running

---

## Optimization Playbook

### To Increase Conversion Rate
1. **More gifts earlier** (Reciprocity)
2. **More micro-commitments** (Commitment)
3. **More specific social proof** (Social Proof)
4. **Stronger urgency** (Scarcity)

### To Reduce Time-to-Convert
1. **Move premium unlock to Day 3** (earlier value)
2. **Add Day 7 urgency check-in** (mid-trial push)
3. **Increase scarcity messaging frequency**

### To Increase Deal Size
1. **Show higher-tier features during trial**
2. **Case studies from high-ARPU customers**
3. **Annual plan messaging** (save 20%)

---

## Success Metrics (Targets)

| Metric | Target | Check Frequency |
|--------|--------|-----------------|
| Trial → Paid | 40-45% | Daily |
| Engagement Score | >70 | Daily |
| Conversion Probability Accuracy | >80% | Weekly |
| Time to Convert | 10-12 days | Weekly |
| MRR at Conversion | $250 | Weekly |
| Reciprocation Rate | >60% | Weekly |
| Milestone Completion Rate | >80% | Weekly |

---

## Files Reference

| File | Purpose |
|------|---------|
| `CIALDINI_SALES_STRATEGY.md` | 35-page complete strategy |
| `SALES_AUTOMATION_SUMMARY.md` | Implementation summary |
| `backend/services/sales/InfluenceBasedSalesService.ts` | 870 lines of code |
| `backend/database/migrations/014_influence_based_sales.sql` | 13 tables + functions |
| `backend/scripts/cron-jobs.ts` | Schedule (8 PM UTC daily) |

---

## Key Contacts / Resources

**Cialdini's 6 Principles:** `CIALDINI_SALES_STRATEGY.md` (read the entire thing)  
**Technical Docs:** Code comments in `InfluenceBasedSalesService.ts`  
**Database Schema:** `014_influence_based_sales.sql` (all tables documented)  

---

**Remember:** This is AUTOMATED. It runs every day at 8 PM UTC without human intervention. You just monitor the dashboards and optimize based on data. <5 minutes/week. 🚀
