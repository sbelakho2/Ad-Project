**🐛 Bug Reports**: [GitHub Issues](https://github.com/sbelakho2/Ad-Project/issues)

---
# Frequently Asked Questions (FAQ)

Common questions about the ApexMediation Platform. Can't find your answer? [Contact support](mailto:support@apexmediation.ee).

---

## Getting Started

### How long does integration take?

**Answer**: Most developers complete integration in **under 10 minutes**. Our SDKs are designed to be plug-and-play with minimal configuration.

**Typical timeline**:
- Install SDK: 2 minutes
- Initialize: 1 minute
- Add first ad placement: 3 minutes
- Test: 4 minutes

**See**: [Quick Start Guide](/docs/getting-started/quickstart)

---

### Do I need a credit card to sign up?

**Answer**: **No**. Sign up is completely free with no credit card required. You only start paying a platform fee once your mediated revenue moves beyond the Starter tier (>$10k/month).

---

### Which platforms do you support?

**Answer**: We support all major platforms:
- ✅ **Unity** (iOS, Android, desktop)
- ✅ **iOS** (Swift, Objective-C)
- ✅ **Android** (Kotlin, Java)
- ✅ **Web** (JavaScript)
- ✅ **React Native** (official bridge — contact support to enable)
- ✅ **Flutter** (official bridge — contact support to enable)

For other engines (Unreal, Godot, custom C++), reach out to support so we can share the latest integration options.

---

### Can I use ApexMediation with other ad networks?

**Answer**: **Yes**. Our platform is a **mediation layer** that works alongside other ad networks. We integrate with:
- AdMob
- Unity Ads
- ironSource
- AppLovin
- Vungle
- And 20+ more

You can use us exclusively or as part of your existing stack.

---


---

### What is your platform fee?

**Answer**: Our BYO tiers charge a percentage of **gross mediated revenue** each month:
- **Starter**: $0 – $10k → **0%** (free)
- **Growth**: $10,001 – $100k → **2.5%**
- **Scale**: $100,001 – $500k → **2.0%**
- **Enterprise**: $500k+ → **1.0–1.5%** with a custom minimum

Because you bring your own demand, these fees simply cover the control plane, observability, and Migration Studio – not a resale margin.

---

### How do I pay my invoice?

**Answer**: We invoice you for the monthly platform fee (NET 30). Send payment using the rails listed on the invoice:
1. **SEPA (EUR)** – Wise Europe SA IBAN, free, same/next-day
2. **ACH (USD)** – Wise US account (Community Federal Savings Bank), 1–3 US business days

Need alternatives?
3. **Stripe card/wallet** – 2.9% + fee, instant confirmation
4. **PayPal** – +2% fee, <48h settlement
5. **Wise multi-currency link** – for CAD/GBP/SGD or rapid FX conversions

Open **Dashboard → Billing → Invoices** to download the PDF/CSV and copy the wiring block. Each invoice email repeats the SEPA + ACH reference to avoid transcription errors.

---

### What happens if our fee is below the minimum invoicing threshold?

**Answer**: We only issue an invoice once your accumulated platform fee reaches **€100**. If a month closes below that amount, the balance rolls forward and appears on the next invoice. Nothing expires—you’ll just see the carryover line item when it crosses the threshold.

**Example**: You owe €75 in January (below €100) → €75 carries over → February fee is €50 → March 1 invoice shows €125 due NET 30.

---

### Are there any hidden fees?

**Answer**: **No**. The only costs are:
- The platform fee for your tier (0–2.5% or negotiated enterprise rate)
- Optional add-ons (white-label console, extended retention, etc.)
- Payment method fees (PayPal: 2%, Crypto: 1%)

No setup fees, no monthly minimums, no surprise surcharges.

---

## Technical Questions

### What is eCPM and how is it calculated?

**Answer**: **eCPM** (effective Cost Per Mille) is the estimated earnings per 1,000 ad impressions.

**Formula**: `eCPM = (Total Revenue / Total Impressions) × 1000`

**Example**: You earned €50 from 25,000 impressions:
```
eCPM = (€50 / 25,000) × 1000 = €2.00
```

**Typical eCPM ranges**:
- Interstitial ads: €5-€15
- Rewarded video ads: €10-€25
- Banner ads: €0.50-€2

---

### How do I increase my eCPM?

**Answer**: Several strategies:

1. **Enable Geographic Discount Optimizer** - Adjusts CPM by country (typically +15-25% revenue)
2. **Use rewarded video ads** - Highest eCPM but requires game mechanic integration
3. **A/B test ad placements** - Find optimal times and locations
4. **Enable Self-Evolving AI** - Automated optimization (typically +20-40% revenue)
5. **Target high-value countries** - US, UK, Canada, Nordic countries have higher CPMs
6. **Improve ad relevance** - More accurate user segments = better match = higher bids

**See**: [Performance Optimization Guide](/docs/advanced/performance-optimization)

---

### Why am I not seeing any ads?

**Common Causes**:

1. **Test Mode Enabled** - Go to Dashboard → App Settings → Disable Test Mode
2. **Invalid API Key** - Check your API key in Dashboard → API Keys
3. **Ad Unit Not Created** - Create ad units in Dashboard → Ad Units
4. **Country Not Supported** - Check [supported countries list](/docs/features/supported-countries)
5. **Ad Blockers** - Ask testers to disable ad blockers
6. **Integration Error** - Check console for error messages

**Still stuck?** [Contact support](mailto:support@apexmediation.ee) with your app ID.

---

### Can I use ApexMediation in a game for children?

**Answer**: **Yes**, but you must comply with COPPA (Children's Online Privacy Protection Act) and GDPR requirements:

1. **Enable Child-Directed Mode** in Dashboard → App Settings
2. **No personalized ads** - Only contextual ads will be shown
3. **Obtain parental consent** before collecting any data
4. **No third-party analytics** - Disable Google Analytics, etc.

**Note**: Child-directed apps typically have 30-50% lower eCPM due to advertising restrictions.

**See**: [COPPA Compliance Guide](/docs/billing-compliance/coppa)

---

### How does fraud detection work?

**Answer**: We use multi-layered fraud detection:

1. **Real-time Analysis** - Each ad impression analyzed in < 50ms
2. **IP Reputation** - Block known bot IPs and data centers
3. **Device Fingerprinting** - Detect emulators and modified devices
4. **Behavioral Patterns** - Flag unusual click/impression ratios
5. **VPN/Proxy Detection** - Identify location spoofing
6. **ML Models** - Continuously learning from fraud patterns

**Fraud Rate**: < 0.5% across our network (industry average: 3-5%)

**Transparency**: See blocked impressions in Dashboard → Analytics → Fraud Report

---

## Account & Billing

### How do I upgrade my plan?

**Answer**: 
1. Go to **Dashboard → Billing**
2. Click "Upgrade Plan"
3. Select your new plan
4. Confirm

Changes take effect immediately. You get instant access to new features.

---

### Can I cancel anytime?

**Answer**: **Yes**, no contracts or cancellation fees.

**What happens when you cancel**:
1. Account remains active until the end of the current billing period
2. Final invoice issued (even if the balance is below €100)
3. Historical data accessible for 1 year
4. Can reactivate within 1 year (data restored)

To cancel: **Dashboard → Billing → Cancel Account**

---

### Do you offer refunds?

**Answer**: Since we use a **platform fee on the revenue you already earn**, there are no upfront payments to refund.

**However**, if you believe we incorrectly calculated our share:
1. Contact billing@apexmediation.ee within 30 days
2. Provide transaction IDs
3. We'll audit and issue credit if error confirmed

**Refund Policy**: Credits issued within 5 business days.

---

### Can I transfer my account to another company?

**Answer**: **Yes**. To transfer account ownership:

1. Email security@apexmediation.ee with:
   - Current owner details
   - New owner details
   - Reason for transfer
   - Signed transfer agreement (we'll provide template)

2. We'll process transfer within 5 business days
3. New owner must verify email and accept Terms of Service

**Note**: All historical data and earnings transfer with the account.

---

## Privacy & Compliance

### Are you GDPR compliant?

**Answer**: **Yes**, fully compliant with GDPR (General Data Protection Regulation).

**What we do**:
- ✅ Obtain user consent before personalized ads
- ✅ Provide cookie consent banner
- ✅ Allow users to delete their data (Right to be Forgotten)
- ✅ Encrypt all personal data in transit and at rest
- ✅ Data Processing Agreements available for enterprise customers
- ✅ EU-based servers (Estonia)

**See**: [GDPR Compliance Guide](/docs/billing-compliance/gdpr)

---

### Where is my data stored?

**Answer**: 
- **Primary Storage**: **Estonia** (EU) - Hetzner data centers
- **Backup Storage**: **Germany** (EU) - AWS Frankfurt
- **Analytics Data**: **Netherlands** (EU) - Managed Postgres read replicas (no ClickHouse dependency)

**No data** stored outside EU. Fully GDPR compliant.

**Data Retention**:
- Transaction logs: 7 years (Estonian legal requirement)
- Analytics data: 2 years
- User accounts: Until deletion requested

---

### Can I delete my data?

**Answer**: **Yes**, you have the Right to be Forgotten under GDPR.

**To delete your data**:
1. Go to **Dashboard → Settings → Privacy**
2. Click "Delete My Data"
3. Confirm deletion (this is irreversible)

**What gets deleted**:
- Personal information (name, email, etc.)
- Analytics data associated with your account
- App settings and configurations

**What is retained** (legal obligation):
- Transaction records (7 years for Estonian tax law)
- Anonymized aggregate statistics

**Processing Time**: 30 days

---

### How do you handle user privacy in my app?

**Answer**: We follow **privacy by design** principles:

1. **Minimal Data Collection** - Only collect what's necessary for ad serving
2. **Consent Required** - Show GDPR consent banner before tracking
3. **Anonymization** - User IDs are hashed and anonymized
4. **No Cross-App Tracking** - Data not shared between apps without consent
5. **Opt-Out** - Users can opt out of personalized ads anytime

**Best Practice**: Implement our consent SDK before showing ads.

**See**: [Privacy Implementation Guide](/docs/features/privacy-implementation)

---

## Advanced Features

### What is the Self-Evolving AI System?

**Answer**: Our **Self-Evolving AI** uses machine learning to automatically optimize your ad strategy:

**What it does**:
- Tests 100+ ad placement combinations daily
- Learns from user behavior patterns
- Adjusts ad timing and frequency
- Optimizes ad formats per user segment
- Predicts best times to show ads

**Results**: Typically **20-40% revenue increase** within 30 days.

**Cost**: Included in Scale & Enterprise. Growth customers can enable it for a **+0.5% platform fee uplift** while it runs in shadow mode.

**See**: [Self-Evolving AI Guide](/docs/advanced/self-evolving-ai)

---

### Can I use my own ad network?

**Answer**: **Yes**, we support **custom ad network integrations** (Enterprise plan only).

**Process**:
1. Contact sales@apexmediation.ee
2. Provide ad network API documentation
3. We build custom adapter (typically 2-4 weeks)
4. You test integration
5. We deploy to production

**Cost**: Included in Enterprise plan (€5,000 setup fee for Growth plan)

---

### Do you support header bidding?

**Answer**: **Yes**, we use **real-time bidding (RTB)** with header bidding capabilities:

- **Client-side bidding** - For web and mobile web
- **Server-side bidding** - For mobile apps (lower latency)
- **Hybrid bidding** - Combines both for optimal results

**Supported Protocols**:
- OpenRTB 2.5
- Prebid.js (web)
- Prebid Mobile (iOS/Android)

**See**: [RTB Integration Guide](/docs/features/rtb)

---

## Support & Resources

### How can I contact support?

**Answer**: Multiple channels available:

- **📧 Email**: support@apexmediation.ee (< 4 hours response time)
- **💬 Live Chat**: Available in dashboard (Mon-Fri, 9AM-5PM EET)
- **📞 Phone**: +372 5XXX XXXX (Enterprise customers only)
- **📚 Documentation**: [docs.apexmediation.ee](https://docs.apexmediation.ee)
- **🐛 Bug Reports**: [GitHub Issues](https://github.com/sbelakho2/Ad-Project/issues)

**Emergency Support** (Production outages, Enterprise only):
- **Phone**: +372 5XXX XXXX (24/7)
- **PagerDuty**: Automatic alerts

---

### Do you have a status page?

**Answer**: **Yes**, check system status anytime:

**URL**: [status.apexmediation.ee](https://status.apexmediation.ee)

**What you'll see**:
- Real-time uptime status
- Incident history
- Scheduled maintenance
- Performance metrics

**Subscribe**: Get email/SMS alerts for outages.

---

### Where can I find code examples?

**Answer**: Multiple resources:

1. **Documentation** - [docs.apexmediation.ee](https://docs.apexmediation.ee) (embedded examples)
2. **GitHub** - [github.com/sbelakho2/Ad-Project](https://github.com/sbelakho2/Ad-Project)
3. **Sample Apps**:
   - Unity: [Sample Game](https://github.com/sbelakho2/Ad-Project)
   - iOS: [Sample App](https://github.com/sbelakho2/Ad-Project)
   - Android: [Sample App](https://github.com/sbelakho2/Ad-Project)

---

### Can I contribute to your documentation?

**Answer**: **Yes!** We welcome contributions:

1. Fork [github.com/sbelakho2/Ad-Project](https://github.com/sbelakho2/Ad-Project)
2. Make improvements (fix typos, add examples, clarify instructions)
3. Submit a Pull Request
4. We'll review within 2 business days

**Contributors** get:
- Name in credits
- Special badge in dashboard
- €100 account credit per accepted PR

---

## Still Have Questions?

**Can't find your answer?** We're here to help!

- **📧 Email**: support@apexmediation.ee
- **💬 Live Chat**: [Dashboard](https://apexmediation.bel-consulting.ee/dashboard)
- **📞 Schedule a Call**: [Book a demo](https://apexmediation.bel-consulting.ee/demo)

**Average Response Time**: < 4 hours (Mon-Fri, 9AM-5PM EET)  
**Enterprise Customers**: < 1 hour, 24/7

---

**Last Updated**: January 2025  
**Questions Answered**: 35+  
**Updated**: Monthly with new common questions
