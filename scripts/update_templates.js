const { Pool } = require("pg");
const pool = new Pool({
  host: "127.0.0.1",
  port: 6432,
  database: "adproject",
  user: "adproject",
  password: "xP5Qa6z+zov+besvN3Fkx2bKgqkb86qtWDYiClttXBtPB44f"
});

async function main() {
  // Template 1: Initial Outreach - Focus on Transparency Engine (unique differentiator)
  const initialSubject = "{Transparency for {{APP_NAME}}|Quick question about {{APP_NAME}} ad auctions|{{APP_NAME}} - verifiable ad revenue}";
  
  const initialBody = `{Hi {{FIRST_NAME}},|Hey,}

{{ICEBREAKER}}

{Quick thought on|A question about|Wanted to ask about} {{APP_NAME}}:

{Do you know|Can you verify|Have you ever checked} exactly what happened in your last 100 ad auctions? {Which bids were received|What the winning prices were|Why each network won or lost}?

{Most publishers cannot.|That data usually lives in a black box.|Very few teams have access to that level of detail.}

{At ApexMediation|We built ApexMediation because|This is why we created ApexMediation} - {every auction decision is|we make every decision} {Ed25519-signed|cryptographically signed} {and exportable|so you can verify it independently}. {Bids received, waterfall order, floor prices, winner selection|Complete transparency into your ad stack|No more guessing why revenue changed} - {all verifiable|all provable|no trust required}.

{The SDK|Integration|Setup} is {3 lines of Kotlin|minimal - no XML plumbing|straightforward}: initialize, {and you are live|consent handled automatically|ship}.

{Would a 30-minute technical walkthrough be useful|Want to see how signed auction logs work in practice|Happy to show you the debug panel}?

{No commitment|Just sharing how it works|Genuinely curious about your current setup}.

{Best|Cheers},
Aaron
ApexMediation`;

  await pool.query(
    "UPDATE sa_spintax_templates SET subject_spintax = $1, body_spintax = $2 WHERE id = '5d485d1c-8070-4743-950a-59980abc15e3'",
    [initialSubject, initialBody]
  );
  console.log("Updated: Initial Outreach v2");

  // Template 2: Follow-up - Focus on Migration Studio (reduces switching risk)
  const followupSubject = "{Re: {{APP_NAME}} ad transparency|Following up: {{APP_NAME}} migration|{{APP_NAME}} - switching mediators safely}";
  
  const followupBody = `{Hi {{FIRST_NAME}},|Hey,}

{Following up on|Circling back to} my earlier note about {{APP_NAME}}.

{One thing I did not mention|Something that might be relevant|A concern I hear often}: {switching mediators is risky|the risk of changing your ad stack|revenue drops during migration}.

{That is why we built|This is where} Migration Studio {comes in|helps}:
- {Import your existing waterfall|Bring your current setup in} and {test it against Apex|shadow test side-by-side}
- {See actual eCPM parity|Compare performance} before touching live traffic
- {Staged rollout|Gradual transition}: 5% to 25% to 100% with {automatic rollback|instant rollback if guardrails trigger}

{The goal is simple|The point}: {validate before switching|prove it works before committing|de-risk the transition}.

{Your existing ad network relationships|Network contracts|Payment arrangements} {stay exactly as they are|remain unchanged|are not affected}. {Networks pay you directly|100% of your revenue is yours|We never touch your revenue} - {we charge a platform fee|just a small platform fee for the layer}.

{Would it help to see|Want a demo of|Curious how} {Migration Studio|the shadow testing|this} works for a {{APP_GENRE}} app?

{Best|Cheers},
Aaron`;

  await pool.query(
    "UPDATE sa_spintax_templates SET subject_spintax = $1, body_spintax = $2 WHERE id = '71431009-6d16-4619-b195-c6766959b6db'",
    [followupSubject, followupBody]
  );
  console.log("Updated: Follow-up v2");

  // Template 3: Final Follow-up - Focus on VRA (Verifiable Revenue Auditor)
  const finalSubject = "{Last note: {{APP_NAME}} revenue verification|{{APP_NAME}} - one more thing|Final thought on {{APP_NAME}}}";
  
  const finalBody = `{{{FIRST_NAME}},|Hi,}

{Last message from me|Final thought|One more thing} on this:

{Have you ever had|Ever experienced|A question} - {a discrepancy between|differences between|gaps in} what your {network dashboard shows|ad networks report} vs {what you actually received|bank deposits|actual revenue}?

{This happens more often than people realize|It is surprisingly common|Many publishers do not notice}. {Different counting methodologies|Timing differences|Exchange rate gaps|IVT adjustments} - {hard to reconcile|messy to audit|time-consuming to track down}.

{We built|ApexMediation includes} the {Verifiable Revenue Auditor|VRA|Revenue Auditor}:
- {Cross-reference|Match} network reports with {SDK events|your actual data} and {signed auction receipts|cryptographic proofs}
- {Auto-classify discrepancies|Categorize deltas}: underpay, IVT, timing, FX
- {Generate dispute kits|Create evidence packages} with {confidence scores|supporting data}

{It is not magic|No promises} - {it just gives you|it provides} the {data to have real conversations|evidence for network discussions|tools to audit your own revenue}.

{If this is interesting|If revenue reconciliation is a pain point|Want to see how it works} - {I can show you|happy to demo|let me know}.

{Either way|Regardless} - {good luck with|nice work on|rooting for} {{APP_NAME}}.

{Best|Cheers|Take care},
Aaron
ApexMediation`;

  await pool.query(
    "UPDATE sa_spintax_templates SET subject_spintax = $1, body_spintax = $2 WHERE id = 'fd08680f-91a5-4b90-a7a7-00cb94684231'",
    [finalSubject, finalBody]
  );
  console.log("Updated: Final Follow-up v2");

  // Create new template: CTV/SSAI specific
  const ctvResult = await pool.query(
    "SELECT id FROM sa_spintax_templates WHERE name = 'CTV SSAI Outreach'");
  
  if (ctvResult.rows.length === 0) {
    const ctvSubject = "{Server-side ads for {{APP_NAME}}|{{APP_NAME}} CTV monetization|SSAI for {{APP_NAME}}}";
    const ctvBody = `{Hi {{FIRST_NAME}},|Hey,}

{{ICEBREAKER}}

{Noticed|Saw|I see} {{APP_NAME}} {is on|supports} {CTV|connected TV|streaming platforms}.

{Quick question|Curious}: {how are you handling|what is your approach to} {ad insertion|monetization} for {Apple TV|Fire TV|Roku|your CTV apps}?

{Client-side|Traditional} ad SDKs {can be tricky|have limitations|often struggle} on CTV - {buffering issues|playback conflicts|inconsistent fill}.

{We built|ApexMediation offers} {server-side ad insertion|SSAI} for {HLS and DASH|live and VOD}:
- {Ad decisions happen server-side|Stitched manifests}
- {No client SDK required|Lighter player integration}
- {Works with|Supports} DRM {streams|content}
- {Same pricing model|Same transparency} as mobile - {based on mediated revenue|platform fee on your ad revenue}, {not impressions|not stream hours}

{Every insertion decision|All ad decisions} {is logged|are signed} - {same transparency engine|verifiable just like mobile|cryptographic receipts}.

{Is CTV monetization|Are streaming ads} {a priority for|on the roadmap for} {{APP_NAME}}?

{Best|Cheers},
Aaron
ApexMediation`;

    await pool.query(
      "INSERT INTO sa_spintax_templates (id, name, subject_spintax, body_spintax, template_type, active, created_at, updated_at) VALUES (gen_random_uuid(), 'CTV SSAI Outreach', $1, $2, 'initial', true, NOW(), NOW())",
      [ctvSubject, ctvBody]
    );
    console.log("Created: CTV SSAI Outreach");
  } else {
    console.log("Skipped: CTV SSAI Outreach (exists)");
  }

  // Create new template: SDK Reliability focus
  const reliabilityResult = await pool.query(
    "SELECT id FROM sa_spintax_templates WHERE name = 'SDK Reliability Outreach'");
  
  if (reliabilityResult.rows.length === 0) {
    const reliabilitySubject = "{{{APP_NAME}} ad SDK stability|OTA-safe ad updates for {{APP_NAME}}|Reliability question for {{APP_NAME}}}";
    const reliabilityBody = `{Hi {{FIRST_NAME}},|Hey,}

{{ICEBREAKER}}

{A question for you|Something I have been wondering|Curious about your experience}:

{Have you ever had|Ever experienced|Has {{APP_NAME}} dealt with} an {ad SDK crash|ANR from an ad network|SDK that broke in production}? {The kind that|Where you} {required an app update to fix|could not do anything until the next release|had to hotfix urgently}?

{It happens more than it should|Surprisingly common|A real pain point} - {one slow adapter|a bad network response|misconfigured timeout} can {stall your whole app|cause ANRs|tank your crash rate}.

{This is|We focused on this because} {why we built|core to} ApexMediation {the way we did|differently}:
- {OTA-safe config rollouts|Server-side configuration} - {change networks without app updates|no more waiting for app review}
- {Per-adapter circuit breakers|Automatic isolation} - {slow networks get isolated|one bad adapter never stalls your auction}
- {Kill switches|Instant disable} - {turn off any adapter from the console|take back control immediately}
- {On-device debug panel|Real-time inspection} - {see exactly what happened|reproduce issues fast}

{The goal|The idea}: {your ad layer should not be a reliability risk|ads should not break your app|stability first, then revenue}.

{Is SDK stability|Are ad-related crashes} {a concern|something you track|on your radar} for {{APP_NAME}}?

{Best|Cheers},
Aaron
ApexMediation`;

    await pool.query(
      "INSERT INTO sa_spintax_templates (id, name, subject_spintax, body_spintax, template_type, active, created_at, updated_at) VALUES (gen_random_uuid(), 'SDK Reliability Outreach', $1, $2, 'initial', true, NOW(), NOW())",
      [reliabilitySubject, reliabilityBody]
    );
    console.log("Created: SDK Reliability Outreach");
  } else {
    console.log("Skipped: SDK Reliability Outreach (exists)");
  }

  // Create new template: Privacy/Consent handling focus
  const consentResult = await pool.query(
    "SELECT id FROM sa_spintax_templates WHERE name = 'Privacy Consent Outreach'");
  
  if (consentResult.rows.length === 0) {
    const consentSubject = "{GDPR for {{APP_NAME}}|{{APP_NAME}} consent handling|Privacy compliance question}";
    const consentBody = `{Hi {{FIRST_NAME}},|Hey,}

{{ICEBREAKER}}

{Quick question|Curious about|Wondering}:

{How does|What is} {{APP_NAME}} {handling|doing for} {consent signals|GDPR/CCPA compliance|TCF 2.2} across {your ad networks|all your demand sources}?

{Getting consent right|Privacy compliance} {is complex|takes work} - {passing TCF strings|propagating signals} to {8+ networks|every adapter|all your partners} {correctly|consistently|reliably}.

{With ApexMediation|What we built}:
- {One-liner|Single call} for {Google UMP|consent form} integration
- {Pass TCF v2.2|US Privacy|COPPA signals} {once|to us} - {we propagate to all networks|automatic distribution to adapters}
- {Read IAB-standard consent|Support existing CMP} - {no scraping|you stay in control}
- {Consent signals included|Privacy data handled} {in auction metadata|transparently} - {each network gets what it needs|compliant by default}

{The result|Goal}: {set consent once|single integration point}, {it flows everywhere|automatic propagation}, {you have audit evidence|logged and verifiable}.

{Is privacy compliance|Is consent management} {a pain point|time-consuming|on your mind} for {{APP_NAME}}?

{Best|Cheers},
Aaron
ApexMediation`;

    await pool.query(
      "INSERT INTO sa_spintax_templates (id, name, subject_spintax, body_spintax, template_type, active, created_at, updated_at) VALUES (gen_random_uuid(), 'Privacy Consent Outreach', $1, $2, 'initial', true, NOW(), NOW())",
      [consentSubject, consentBody]
    );
    console.log("Created: Privacy Consent Outreach");
  } else {
    console.log("Skipped: Privacy Consent Outreach (exists)");
  }

  // Create new template: ML Optimization focus
  const mlResult = await pool.query(
    "SELECT id FROM sa_spintax_templates WHERE name = 'ML Waterfall Outreach'");
  
  if (mlResult.rows.length === 0) {
    const mlSubject = "{{{APP_NAME}} waterfall optimization|ML-powered ads for {{APP_NAME}}|Smarter ad auctions for {{APP_NAME}}}";
    const mlBody = `{Hi {{FIRST_NAME}},|Hey,}

{{ICEBREAKER}}

{Quick question about|Curious how} {{APP_NAME}} {handles|manages} {waterfall ordering|ad network priority}:

{Is it|Are you using} {static priority lists|manual configuration|fixed ordering}? Or {something more dynamic|real-time optimization|ML-driven}?

{The challenge|What we have seen}: {static waterfalls leave money on the table|fixed priority misses bid fluctuations|manual tuning cannot keep up with real-time changes}.

{Our|ApexMediation's} ML Waterfall Optimizer:
- {Predicts bid responses|Estimates expected revenue} {per network|in real-time}
- {Reorders waterfall|Optimizes priority} based on {geo, time, device, history|contextual signals}
- {Shadow ML|Test before activating}: validate new models against your traffic
- {A/B testing|Statistical significance}: know what actually works

{The tradeoff|What this means}: {you lose some control|it is less manual} but {the model learns from your traffic|optimizes continuously|adapts automatically}.

{Would it help to see|Want to understand|Curious} how {the ML layer|shadow testing|model evaluation} works for a {{APP_GENRE}} app?

{Best|Cheers},
Aaron
ApexMediation`;

    await pool.query(
      "INSERT INTO sa_spintax_templates (id, name, subject_spintax, body_spintax, template_type, active, created_at, updated_at) VALUES (gen_random_uuid(), 'ML Waterfall Outreach', $1, $2, 'initial', true, NOW(), NOW())",
      [mlSubject, mlBody]
    );
    console.log("Created: ML Waterfall Outreach");
  } else {
    console.log("Skipped: ML Waterfall Outreach (exists)");
  }

  console.log("\nAll templates updated!");
  await pool.end();
}

main().catch(e => { console.error(e); process.exit(1); });
