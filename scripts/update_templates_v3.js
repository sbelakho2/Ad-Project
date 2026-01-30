#!/usr/bin/env node
/**
 * Sales Automation Template Update Script - V3 (Fixed Grammar + Shorter)
 * 
 * Applies industry-standard email frameworks with grammar fixes:
 * - AIDA (Attention, Interest, Desire, Action)
 * - PAS (Problem, Agitate, Solution)
 * - BASHO (Personalized research-based)
 * - Observation trigger
 * - Resource/Gift offering
 * - Direct/Short format
 * - Peer comparison
 * 
 * All content based on real ApexMediation features from apexmediation.ee
 */

const { Client } = require('pg');

// Database configuration
const dbConfig = {
  host: process.env.DB_HOST || '127.0.0.1',
  port: parseInt(process.env.DB_PORT || '6432'),
  database: process.env.DB_NAME || 'adproject',
  user: process.env.DB_USER || 'adproject',
  password: process.env.DB_PASSWORD || 'xP5Qa6z+zov+besvN3Fkx2bKgqkb86qtWDYiClttXBtPB44f',
  ssl: false
};

// Template definitions - V3 with grammar fixes and shorter format
const templates = [
  // Template 1: AIDA Framework - Initial Outreach (Transparency Engine)
  {
    name: 'Initial Outreach v3 (AIDA)',
    subject_spintax: '{ad transparency|sdk analytics|monetization clarity}',
    body_spintax: `Hi {{FIRST_NAME}},

{{ICEBREAKER}}

{I noticed|Saw that} {{APP_NAME}} {runs ads|monetizes with advertising}. {Quick question|Curious}: do you {know which ad network|have visibility into which network} {gets credit for|is attributed to} each impression?

Most {{APP_GENRE}} apps {struggle with this|face attribution gaps}. {Ad networks sometimes claim|Networks can double-count} the same impression, {making it hard|which makes it difficult} to {optimize spend|allocate budget effectively}.

{Our Transparency Engine|ApexMediation attribution layer} {shows|provides} impression-level data across {all your|every} demand source – {so you can see|giving you clarity on} {exactly where revenue comes from|actual network performance}.

{Would a quick comparison of your current setup be useful|Happy to share how this works if helpful}?

{Best|Cheers},
Alex`,
    template_type: 'initial'
  },

  // Template 2: PAS Framework - Follow-up (SDK Reliability)
  {
    name: 'Follow-up v3 (PAS)',
    subject_spintax: '{re: sdk stability|following up|quick thought}',
    body_spintax: `{{FIRST_NAME}},

{Following up|Circling back} on my earlier note.

{Here is what I keep hearing|A pattern I see often}: {{APP_GENRE}} publishers {add more SDKs|stack ad networks} to {boost fill rates|maximize coverage}. {Makes sense|Logical approach}.

{But then|The catch}: {app size balloons|APK bloats}, {crashes spike during|startup time suffers on} {cold starts|older devices}, and {users churn|ratings drop} – {wiping out|offsetting} {any revenue gains|the fill rate benefits}.

{We built|ApexMediation offers} a {unified SDK|single integration} that {connects to|mediates across} {30+|dozens of} demand sources. {One SDK|Single codebase}, {no bloat|minimal footprint}, {stable performance|consistent reliability}.

{Would it help to|Want me to} share {how other|what} {{APP_GENRE}} apps {handle this|have done}?

{– Alex|Best, Alex}`,
    template_type: 'follow_up'
  },

  // Template 3: Observation Trigger - Final Follow-up (VRA)
  {
    name: 'Final Follow-up v3 (Observation)',
    subject_spintax: '{noticed something|quick observation|one more thing}',
    body_spintax: `Hi {{FIRST_NAME}},

{Last note|Final follow-up} – {will keep this brief|one more thought}.

{Noticed|Saw} {{APP_NAME}} {has|uses} {rewarded video|rewarded ads}. {Here is a thought|Something to consider}: {most apps|many publishers} {show the same reward value|use static rewards} regardless of {what the ad actually pays|actual CPM variance}.

{Some teams|Forward-thinking publishers} are {now matching|starting to align} {user rewards|reward amounts} to {real-time ad value|live CPM data}. {Users get|Players receive} {variable rewards|dynamic bonuses} – {higher CPM ads|premium impressions} = {bigger rewards|better prizes}. {This tends to|It can} {boost engagement|increase ad completion rates}.

{We call this|It is called} {Variable Reward Alignment|VRA}. {Happy to|Can} explain {how it works|the mechanics} if {you are curious|that sounds interesting}.

{Thanks for your time|Appreciate your patience},
Alex`,
    template_type: 'final_follow_up'
  },

  // Template 4: BASHO Framework - CTV SSAI Focus
  {
    name: 'CTV Outreach v2 (BASHO)',
    subject_spintax: '{ctv question|streaming ads|ssai}',
    body_spintax: `{{FIRST_NAME}},

{{ICEBREAKER}}

{Been researching|Looking into} {{APP_NAME}} {streaming strategy|CTV presence}. {Noticed you are|Looks like you are} {in the OTT space|targeting connected TV}.

{Quick question|Curious}: {how are you handling|what is your approach to} {server-side ad insertion|SSAI} on {smart TVs|CTV platforms}? {Many streaming apps|A lot of OTT publishers} {struggle with|run into issues around} {ad-blocker bypass|seamless ad stitching} and {viewer experience|latency on lower-end devices}.

{We have built|ApexMediation has} {SSAI infrastructure|server-side insertion} that {handles this|addresses these} – {stitches ads|inserts creatives} {at the CDN level|before content delivery}, {works across|compatible with} {Roku, Fire TV, Apple TV|major CTV platforms}.

{Worth a conversation|Would a quick overview help}?

{Best|– Alex}`,
    template_type: 'initial'
  },

  // Template 5: Resource Offering - Privacy & Consent
  {
    name: 'Privacy Resource v2 (Gift)',
    subject_spintax: '{consent management resource|privacy checklist|gdpr + att}',
    body_spintax: `Hi {{FIRST_NAME}},

{{ICEBREAKER}}

{Put together|We compiled} a {checklist|quick guide} on {consent management|privacy compliance} for {{APP_GENRE}} apps – {covers|includes} {ATT opt-in flows|iOS tracking prompts}, {GDPR signal passing|EU consent strings}, and {TCF 2.2 implementation|IAB framework compliance}.

{Thought it might be useful|Figured this could help} since {{APP_NAME}} {likely deals with|probably handles} {users across|players from} {multiple regions|different jurisdictions}.

{Happy to share|Can send it over} – {no strings attached|just thought it would be relevant}.

{Also|Side note}: {we handle|our SDK manages} {consent orchestration|CMP integration} {automatically|built-in} – {pre-bid signals|privacy parameters} {flow to all demand|propagate across networks} {without extra code|natively}.

{Want the checklist|Interested}?

{Cheers|Best},
Alex`,
    template_type: 'initial'
  },

  // Template 6: Short & Direct - ML Waterfall
  {
    name: 'ML Waterfall v2 (Direct)',
    subject_spintax: '{waterfall question|bidding setup|quick ask}',
    body_spintax: `{{FIRST_NAME}},

{Straight to the point|Quick question}: {is|does} {{APP_NAME}} {still using|run on} {static waterfalls|manual floor prices}?

{Many|Most} {{APP_GENRE}} apps {still are|do} – {it works|gets the job done}, but {leaves money on the table|is not optimal}.

{Machine learning waterfalls|ML-optimized mediation} {adjust floors|tune bid thresholds} {per-impression|in real-time} based on {user behavior|session context}, {geography|geo signals}, and {time of day|historical patterns}. {No manual tweaking|Automated optimization}.

{Takes about|Setup is} {30 minutes|half an hour} to {migrate|switch over}.

{Worth exploring|Interested}?

{– Alex|Best}`,
    template_type: 'initial'
  },

  // Template 7: Peer Comparison - Migration Studio
  {
    name: 'Migration v2 (Peer Comparison)',
    subject_spintax: '{migration question|switching platforms|quick thought}',
    body_spintax: `Hi {{FIRST_NAME}},

{{ICEBREAKER}}

{Curious|Quick question}: {have you considered|ever thought about} {switching|migrating} {{APP_NAME}} {mediation platform|ad stack}? {Or is the migration pain|Does the switching cost seem} {too high|not worth it}?

{Totally get it|Understandable} – {re-integrating SDKs|rebuilding waterfall logic} is {a nightmare|painful}. {That is why|Which is why} {most publishers stay put|teams stick with what they have} {even when|despite} {performance lags|suboptimal results}.

{We built|There is} a {Migration Studio|parallel testing tool} that {runs your new setup|tests new configurations} {alongside your current one|in parallel} – {same traffic|real impressions}, {no risk|zero downtime}. {You only switch|Cutover happens} when {you see the data|numbers prove it}.

{Would a walkthrough be useful|Want to see how it works}?

{Best|– Alex}`,
    template_type: 'follow_up'
  }
];

async function main() {
  const client = new Client(dbConfig);
  
  try {
    await client.connect();
    console.log('Connected to database');

    // First, deactivate all existing templates
    const deactivateResult = await client.query(
      'UPDATE sa_spintax_templates SET active = false WHERE active = true'
    );
    console.log(`Deactivated ${deactivateResult.rowCount} existing templates`);

    // Insert new templates (delete existing with same name first to avoid constraint issues)
    for (const template of templates) {
      // Delete if exists
      await client.query('DELETE FROM sa_spintax_templates WHERE name = $1', [template.name]);
      
      // Insert new
      const result = await client.query(`
        INSERT INTO sa_spintax_templates (name, subject_spintax, body_spintax, template_type, active)
        VALUES ($1, $2, $3, $4, true)
        RETURNING id, name
      `, [template.name, template.subject_spintax, template.body_spintax, template.template_type]);
      
      console.log(`✓ Created template: ${result.rows[0].name} (${result.rows[0].id})`);
    }

    // Verify active templates
    const activeTemplates = await client.query(
      'SELECT id, name, template_type, active FROM sa_spintax_templates WHERE active = true ORDER BY template_type'
    );
    
    console.log('\n=== Active Templates ===');
    for (const row of activeTemplates.rows) {
      console.log(`  [${row.template_type}] ${row.name}`);
    }
    console.log(`\nTotal active templates: ${activeTemplates.rowCount}`);

  } catch (err) {
    console.error('Error:', err.message);
    process.exit(1);
  } finally {
    await client.end();
  }
}

main();
