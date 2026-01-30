const { Pool } = require("pg");
const pool = new Pool({
  host: "127.0.0.1",
  port: 6432,
  database: "adproject",
  user: "adproject",
  password: "xP5Qa6z+zov+besvN3Fkx2bKgqkb86qtWDYiClttXBtPB44f"
});

// Spintax processor - handles {option1|option2} but not {{VARIABLE}}
function processSpintax(text) {
  let result = text;
  let changed = true;
  
  // Keep processing until no more changes (handles nested spintax)
  while (changed) {
    changed = false;
    // Match single braces with pipe options, but not double braces
    const regex = /\{([^{}|]+(?:\|[^{}|]+)+)\}/g;
    let match;
    
    while ((match = regex.exec(result)) !== null) {
      const options = match[1].split("|");
      const chosen = options[Math.floor(Math.random() * options.length)];
      result = result.substring(0, match.index) + chosen + result.substring(match.index + match[0].length);
      changed = true;
      regex.lastIndex = 0;
      break; // Restart regex after modification
    }
  }
  
  return result;
}

// Sample data for personalization
const sampleApps = [
  { name: "Fitness Tracker Pro", genre: "Health & Fitness", firstName: "Sarah" },
  { name: "Word Puzzle Master", genre: "Casual Games", firstName: "Mike" },
  { name: "Budget Buddy", genre: "Finance", firstName: "Jennifer" },
  { name: "Recipe Box", genre: "Food & Drink", firstName: "David" },
  { name: "Meditation Space", genre: "Health & Fitness", firstName: "Emily" },
  { name: "Photo Editor Plus", genre: "Photography", firstName: "Alex" },
  { name: "Language Learn", genre: "Education", firstName: "Chris" },
  { name: "Weather Now", genre: "Weather", firstName: "Jessica" },
  { name: "Running Tracker", genre: "Sports", firstName: "Tom" },
  { name: "Sleep Better", genre: "Health & Fitness", firstName: "Rachel" },
  { name: "Daily Journal", genre: "Lifestyle", firstName: "Matt" },
  { name: "Music Player Pro", genre: "Music", firstName: "Amanda" },
  { name: "Crypto Wallet", genre: "Finance", firstName: "Kevin" },
  { name: "News Reader", genre: "News", firstName: "Lisa" },
  { name: "Trivia King", genre: "Casual Games", firstName: "Brian" },
  { name: "Workout Planner", genre: "Health & Fitness", firstName: "Michelle" },
  { name: "Video Maker", genre: "Photography", firstName: "Jason" },
  { name: "Task Manager", genre: "Productivity", firstName: "Nicole" },
  { name: "Drawing Pad", genre: "Art & Design", firstName: "Andrew" },
  { name: "Podcast App", genre: "Entertainment", firstName: "Stephanie" },
  { name: "Parking Finder", genre: "Navigation", firstName: "Ryan" },
  { name: "Habit Tracker", genre: "Productivity", firstName: "Lauren" },
  { name: "Stock Watch", genre: "Finance", firstName: "Daniel" },
  { name: "Flashcard Pro", genre: "Education", firstName: "Katie" },
  { name: "Alarm Clock", genre: "Utilities", firstName: "Mark" },
  { name: "Scanner App", genre: "Utilities", firstName: "Anna" },
  { name: "Calorie Counter", genre: "Health & Fitness", firstName: "Steve" },
  { name: "Note Taking", genre: "Productivity", firstName: "Maria" },
  { name: "Radio Player", genre: "Music", firstName: "Paul" },
  { name: "Plant Care", genre: "Lifestyle", firstName: "Julia" },
  { name: "Puzzle Quest", genre: "Casual Games", firstName: "Brandon" },
  { name: "Mindfulness Coach", genre: "Health & Fitness", firstName: "Olivia" },
  { name: "File Manager", genre: "Utilities", firstName: "Tyler" },
  { name: "Voice Recorder", genre: "Utilities", firstName: "Samantha" },
  { name: "Kids Learning", genre: "Education", firstName: "Greg" },
  { name: "Dating Connect", genre: "Social", firstName: "Ashley" },
  { name: "Sports Scores", genre: "Sports", firstName: "Eric" },
  { name: "Travel Planner", genre: "Travel", firstName: "Megan" },
  { name: "Shopping List", genre: "Lifestyle", firstName: "Josh" },
  { name: "VPN Secure", genre: "Utilities", firstName: "Heather" }
];

const icebreakers = [
  "Came across {{APP_NAME}} while researching {{APP_GENRE}} apps with interesting ad strategies.",
  "Found {{APP_NAME}} mentioned in a dev forum discussion about monetization approaches.",
  "Your recent update to {{APP_NAME}} caught my attention - nice work on the UX improvements.",
  "Saw {{APP_NAME}} ranking well in the {{APP_GENRE}} category - congrats on the traction.",
  "A colleague shared {{APP_NAME}} as an example of solid {{APP_GENRE}} execution.",
  "Been looking at {{APP_GENRE}} apps and {{APP_NAME}} stood out for its clean design.",
  "Your interview on building {{APP_NAME}} was an interesting read.",
  "Noticed {{APP_NAME}} in the top charts for {{APP_GENRE}} apps recently.",
  "A friend recommended {{APP_NAME}} - been using it myself and the experience is solid.",
  "Saw your ProductHunt launch of {{APP_NAME}} - great positioning for the {{APP_GENRE}} market."
];

async function main() {
  // Get all active templates
  const templates = await pool.query(
    "SELECT id, name, subject_spintax, body_spintax FROM sa_spintax_templates WHERE active = true ORDER BY name"
  );
  
  console.log(`Found ${templates.rows.length} active templates\n`);
  console.log("=".repeat(80));
  console.log("GENERATING 40 EMAIL SAMPLES");
  console.log("=".repeat(80));
  
  let sampleIndex = 0;
  const samplesPerTemplate = Math.ceil(40 / templates.rows.length);
  
  for (const template of templates.rows) {
    const numSamples = Math.min(samplesPerTemplate, 40 - sampleIndex);
    
    for (let i = 0; i < numSamples && sampleIndex < 40; i++) {
      const app = sampleApps[sampleIndex % sampleApps.length];
      const icebreaker = icebreakers[Math.floor(Math.random() * icebreakers.length)];
      
      // Replace personalization variables FIRST
      let subject = template.subject_spintax
        .replace(/\{\{APP_NAME\}\}/g, app.name)
        .replace(/\{\{APP_GENRE\}\}/g, app.genre)
        .replace(/\{\{FIRST_NAME\}\}/g, app.firstName);
      
      let body = template.body_spintax
        .replace(/\{\{APP_NAME\}\}/g, app.name)
        .replace(/\{\{APP_GENRE\}\}/g, app.genre)
        .replace(/\{\{FIRST_NAME\}\}/g, app.firstName)
        .replace(/\{\{ICEBREAKER\}\}/g, icebreaker
          .replace(/\{\{APP_NAME\}\}/g, app.name)
          .replace(/\{\{APP_GENRE\}\}/g, app.genre));
      
      // THEN process spintax
      subject = processSpintax(subject);
      body = processSpintax(body);
      
      sampleIndex++;
      console.log(`\n--- SAMPLE ${sampleIndex} (${template.name}) ---`);
      console.log(`TO: ${app.firstName.toLowerCase()}@${app.name.toLowerCase().replace(/\s+/g, "")}.com`);
      console.log(`SUBJECT: ${subject}`);
      console.log(`\nBODY:`);
      console.log(body);
      console.log("-".repeat(60));
    }
  }
  
  console.log("\n" + "=".repeat(80));
  console.log(`TOTAL SAMPLES GENERATED: ${sampleIndex}`);
  console.log("=".repeat(80));
  
  await pool.end();
}

main().catch(e => { console.error(e); process.exit(1); });
