#!/usr/bin/env node
/**
 * Sales Automation Template Update Script - V4 (Psychology-Backed)
 * 
 * Uses proven email frameworks based on behavioral psychology:
 * - Before-After-Bridge (BAB) - pleasure/pain motivation
 * - "But You Are Free" - 42 studies show this doubles acceptance
 * - Praise-Picture-Push (3P) - compliment activates reward center
 * - 3-B Plan - Brevity, Blunt, Basic
 * - Curiosity Gap - reveal some but not all
 * 
 * All content stays within ad-tech industry norms and credibility
 */

const { Client } = require('pg');

const dbConfig = {
  host: process.env.DB_HOST || '127.0.0.1',
  port: parseInt(process.env.DB_PORT || '6432'),
  database: process.env.DB_NAME || 'adproject',
  user: process.env.DB_USER || 'adproject',
  password: process.env.DB_PASSWORD || 'xP5Qa6z+zov+besvN3Fkx2bKgqkb86qtWDYiClttXBtPB44f',
  ssl: false
};

const templates = [
  // Template 1: Before-After-Bridge (BAB) - Transparency
  {
    name: 'Initial v4 (Before-After-Bridge)',
    subject_spintax: '{quick question|ad attribution|transparency}',
    body_spintax: `Hi {{FIRST_NAME}},

{{ICEBREAKER}}

{Right now|Currently}, most {{APP_GENRE}} publishers {spend hours|waste time} {reconciling ad reports|trying to figure out} which network {actually drove|deserves credit for} each impression. {Your data never matches|Numbers from different dashboards rarely align}.

{Imagine if|What if} you could see {exactly|precisely} which demand source {earned|was responsible for} every one of your impressions – {with cryptographic proof|signed and verifiable} that {no one can dispute|networks cannot contest}.

{That is what our|ApexMediation} Transparency Engine {does|provides}. {One dashboard|Single view}, {impression-level clarity|complete visibility}, {no more guesswork|full data transparency} for your team.

{Curious if this|Would this} be {relevant for|useful to} {{APP_NAME}}? {Either way, no pressure|Totally fine if not} – {just thought it might help you|figured I would ask}.

{Best|Cheers},
Alex`,
    template_type: 'initial'
  },

  // Template 2: "But You Are Free" Framework - SDK Reliability
  {
    name: 'Initial v4 (But You Are Free)',
    subject_spintax: '{sdk question|quick thought|app stability}',
    body_spintax: `{{FIRST_NAME}},

{{ICEBREAKER}}

{Curious|Wondering} – {is|does} {{APP_NAME}} {running multiple ad SDKs|using several ad networks}?

{Most|Many} {{APP_GENRE}} apps {do|are}. {The challenge is|Problem is}: {each SDK adds weight to your APK|more SDKs mean your app gets heavier}, and {your cold starts get slower|startup times suffer for your users}. {Some publishers see|We have seen} {crash rates climb|stability issues} when {SDK conflicts happen|networks do not play nice together}.

{Our unified SDK|ApexMediation} {connects to|mediates} {30+ demand sources|dozens of networks} {through one integration|with a single codebase}. {Smaller footprint for you|Lighter}, {more stable|fewer conflicts}, {same coverage|full demand access}.

{That said|Of course}, {you are completely free to|feel free to} {ignore this if it is not relevant|pass if this is not a fit for you} – {I am just curious if|just wondering whether} SDK stability is {something you think about|on your radar}.

{– Alex|Best}`,
    template_type: 'initial'
  },

  // Template 3: Praise-Picture-Push (3P) - CTV/SSAI
  {
    name: 'Initial v4 (Praise-Picture-Push)',
    subject_spintax: '{impressed by|nice work on|ctv question}',
    body_spintax: `Hi {{FIRST_NAME}},

{{ICEBREAKER}}

{Been looking at|Checked out} {{APP_NAME}} – {really like|impressed by} {what you have built|your execution}. {The {{APP_GENRE}} space is competitive|Not easy to stand out in {{APP_GENRE}}}, and {you are doing it well|your approach is solid}.

{Here is a thought|Something to consider}: {as|when} {{APP_NAME}} {scales to|expands into} {CTV or streaming|connected TV}, {server-side ad insertion|SSAI} {becomes critical for you|really matters}. {Ads stitched at the CDN level|Server-side insertion} means {no ad blockers affecting your revenue|bypass ad blocking}, {seamless viewer experience for your users|smooth playback}, and {works on|compatibility with} {Roku, Fire TV, Apple TV|all major platforms}.

{If streaming is|Should CTV be} on your roadmap, {happy to|I can} share {how other publishers|what teams} {are handling this|have done}. {No obligation|No strings attached} – {just offering|genuinely want to help} if {it would be useful to you|that would help}.

{Best|Cheers},
Alex`,
    template_type: 'initial'
  },

  // Template 4: 3-B Plan (Brevity, Blunt, Basic) - ML Waterfall
  {
    name: 'Initial v4 (3-B Blunt)',
    subject_spintax: '{waterfall question|floor prices|quick ask}',
    body_spintax: `{{FIRST_NAME}},

{Blunt question|Straight up}: {are you still|is {{APP_NAME}}} {manually setting floor prices|using static waterfalls}?

{If yes|If so}: {machine learning|ML-based} floor optimization {adjusts in real-time|tunes per-impression} based on {user signals, device type, time of day|context like geography and session depth}. {Most publishers|Teams who switched} {see better results|find it works better} than {manual tuning|static setups} – {without the spreadsheet work|no constant tweaking needed}.

{If no|If not}, {ignore this|disregard} – {I will not follow up|no further emails}.

{Setup takes|Integration is} {about 30 minutes|under an hour}. {Happy to show|Want to see} {how it works|a quick demo}?

{– Alex|Best}`,
    template_type: 'initial'
  },

  // Template 5: Curiosity Gap - VRA
  {
    name: 'Follow-up v4 (Curiosity Gap)',
    subject_spintax: '{one more thing|quick idea|follow-up}',
    body_spintax: `{{FIRST_NAME}},

{Following up|Circling back} – {had one more thought|one more idea}.

{Noticed|Saw} {{APP_NAME}} {uses rewarded ads|has rewarded video}. {Most publishers|Many apps} {give the same reward|use fixed rewards} regardless of {what the ad paid|actual CPM}.

{Some teams|Forward-thinking publishers} {are now|started} {matching rewards to ad value|aligning user rewards with real-time CPM}. {Higher-paying ad|Premium impression} = {bigger reward|better prize}. {Users love it|Engagement goes up}, and {it is fully transparent|completely verifiable}.

{Curious how|Want to know how} {this works|it is done}? {Happy to explain|Can share more} – {or feel free to pass|no pressure if not relevant}.

{– Alex|Best}`,
    template_type: 'follow_up'
  },

  // Template 6: Problem-Solution with Exit - Privacy/Consent
  {
    name: 'Initial v4 (Problem-Exit)',
    subject_spintax: '{privacy question|consent management|gdpr + att}',
    body_spintax: `Hi {{FIRST_NAME}},

{{ICEBREAKER}}

{Quick question|Curious}: {how is|how does} {{APP_NAME}} {handling|managing} {consent signals|privacy compliance} across {different regions|EU, US, etc}?

{ATT prompts|iOS tracking}, {GDPR consent strings|EU requirements}, {TCF 2.2|IAB frameworks} – {it is a lot to coordinate|lots of moving pieces}. {And if signals do not propagate|When consent does not reach} {to all demand sources|every network}, {you risk compliance issues|there can be problems}.

{Our SDK|ApexMediation} {handles consent orchestration|manages this} {natively|built-in}. {One integration|Single setup}, {all signals propagate automatically|consent flows to all demand} – {no extra code|nothing else to build}.

{If privacy compliance is|Should this be} {something you think about|on your radar}, {happy to share more|I can explain how it works}. {If not, totally fine|No worries if not relevant} – {just thought I would ask|figured it was worth mentioning}.

{Best|Cheers},
Alex`,
    template_type: 'initial'
  },

  // Template 7: Migration with Social Proof - Migration Studio
  {
    name: 'Follow-up v4 (Migration Story)',
    subject_spintax: '{switching platforms|migration question|quick thought}',
    body_spintax: `Hi {{FIRST_NAME}},

{{ICEBREAKER}}

{Curious|Quick question}: {have you ever considered|thought about} {switching|migrating} {{APP_NAME}} to {a different mediation platform|another ad stack}?

{Most publishers|Many teams} {stay put|do not switch} – {even when performance lags|despite suboptimal results} – because {migration is painful|the switching cost seems too high}. {Re-integrating SDKs|Rebuilding waterfall logic}, {risking downtime|potential revenue loss}, {hoping the new setup works|uncertainty about results}.

{What if you could|Imagine if you could} {test the new configuration|run a parallel setup} {alongside your current one|without touching production} – {same traffic|real impressions}, {no risk|zero downtime}? {You only switch|Cutover happens} when {you see the data|numbers prove it is better}.

{That is what|This is} Migration Studio {does|enables}. {Happy to walk through|Want to see how it works}? {Or feel free to pass|No pressure either way}.

{Best|– Alex}`,
    template_type: 'follow_up'
  },

  // Template 8: Final Follow-up - Soft Close
  {
    name: 'Final v4 (Soft Close)',
    subject_spintax: '{last note|closing the loop|final follow-up}',
    body_spintax: `Hi {{FIRST_NAME}},

{Last note|Final follow-up} – {I will keep this brief|do not want to crowd your inbox}.

{I reached out|Sent a few notes} about {how ApexMediation|our platform} {might help|could be useful for} {{APP_NAME}} with {transparency|ad attribution}, {reliability|SDK stability}, or {monetization optimization|revenue}.

{If the timing is not right|If this is not relevant right now}, {totally understand|no problem}. {If you are curious|If you would like to learn more}, {I am happy to|would be glad to} {share more details|set up a quick walkthrough} – {no pressure|completely your call}.

{Either way|Regardless}, {thanks for your time|appreciate you reading this}. {Wishing you and|Best of luck to} {{APP_NAME}} {continued success|all the best}.

{Cheers|Best},
Alex`,
    template_type: 'final_follow_up'
  }
];

async function main() {
  const client = new Client(dbConfig);
  
  try {
    await client.connect();
    console.log('Connected to database');

    const deactivateResult = await client.query(
      'UPDATE sa_spintax_templates SET active = false WHERE active = true'
    );
    console.log(`Deactivated ${deactivateResult.rowCount} existing templates`);

    for (const template of templates) {
      await client.query('DELETE FROM sa_spintax_templates WHERE name = $1', [template.name]);
      
      const result = await client.query(`
        INSERT INTO sa_spintax_templates (name, subject_spintax, body_spintax, template_type, active)
        VALUES ($1, $2, $3, $4, true)
        RETURNING id, name
      `, [template.name, template.subject_spintax, template.body_spintax, template.template_type]);
      
      console.log(`✓ Created: ${result.rows[0].name}`);
    }

    const activeTemplates = await client.query(
      'SELECT name, template_type FROM sa_spintax_templates WHERE active = true ORDER BY template_type, name'
    );
    
    console.log('\n=== Active Templates ===');
    for (const row of activeTemplates.rows) {
      console.log(`  [${row.template_type}] ${row.name}`);
    }
    console.log(`\nTotal: ${activeTemplates.rowCount} templates`);

  } catch (err) {
    console.error('Error:', err.message);
    process.exit(1);
  } finally {
    await client.end();
  }
}

main();
