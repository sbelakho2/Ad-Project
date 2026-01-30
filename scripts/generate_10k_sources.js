#!/usr/bin/env node
/**
 * Generate 10,000+ lead sources for V2 Sales Automation
 * Based on deep research of:
 * - Mobile game developers and publishers worldwide
 * - App store categories and genres
 * - Geographic regions and markets
 * - Download tier targeting
 */

const { Client } = require('pg');

// Research-based data from Wikipedia lists and industry knowledge

// TOP MOBILE GAME DEVELOPERS/PUBLISHERS (from Wikipedia research)
const GAME_COMPANIES = [
  // Major Publishers
  'Activision', 'Blizzard', 'Electronic Arts', 'EA Games', 'Ubisoft', 'Take-Two', 'Rockstar Games',
  'Nintendo', 'Sony Interactive', 'Microsoft Studios', 'Xbox Game Studios', 'Bethesda', 'Square Enix',
  'Capcom', 'Bandai Namco', 'Konami', 'Sega', 'Warner Bros Games', 'THQ Nordic', '2K Games',
  
  // Mobile-Focused
  'Supercell', 'King', 'Rovio', 'Zynga', 'Gameloft', 'Glu Mobile', 'Jam City', 'Scopely',
  'Playrix', 'Peak Games', 'MZ', 'Machine Zone', 'Playtika', 'Moon Active', 'Socialpoint',
  'Kabam', 'Netmarble', 'Nexon', 'miHoYo', 'HoYoverse', 'NetEase', 'Tencent Games', 'TiMi Studios',
  'DeNA', 'GREE', 'CyberAgent', 'Cygames', 'Colopl', 'GungHo', 'Com2uS', 'Gamevil',
  'IGG', 'Lilith Games', 'Yoozoo', 'FunPlus', '4399', 'Giant Network', 'Century Games',
  
  // Notable Indies & Mid-Size
  'Halfbrick', 'Kiloo', 'Ketchapp', 'Voodoo', 'Hypercasual Games', 'Kwalee', 'Azur Games',
  'Lion Studios', 'AppLovin', 'ironSource', 'Unity', 'Applovin Lion', 'Supersonic Studios',
  'CrazyLabs', 'SayGames', 'Good Job Games', 'Tastypill', 'Homa Games', 'TapNation',
  'Yso Corp', 'Amanotes', 'Miniclip', 'Outfit7', 'Imangi Studios', 'Hipster Whale',
  'Noodlecake', 'Devolver Digital', 'Raw Fury', 'Humble Bundle', 'tinyBuild',
  
  // Japanese Developers
  'Atlus', 'FromSoftware', 'Koei Tecmo', 'Level-5', 'Nippon Ichi', 'Falcom', 'Spike Chunsoft',
  'Compile Heart', 'Idea Factory', 'Marvelous', 'Kemco', 'XSEED', 'NIS America', 'Acquire',
  'Grasshopper', 'PlatinumGames', 'Arc System Works', 'SNK', 'Cave', 'Taito', 'Tecmo',
  
  // Korean Developers
  'Smilegate', 'NCSoft', 'Krafton', 'Pearl Abyss', 'Kakao Games', 'Webzen', 'Wemade',
  'Gravity', 'Neowiz', 'PUBG Corp', 'Bluehole', 'Devsisters', 'Longtu', 'Line Games',
  
  // Chinese Developers
  'Tencent', 'NetEase', 'miHoYo', 'Lilith', 'FunPlus', 'IGG', 'Yoozoo', 'Giant Interactive',
  'Perfect World', 'Changyou', 'Shanda', 'Snail Games', '37Games', 'ZQGame', 'JJ World',
  'Kingsoft', 'bilibili', 'Papergames', 'XD Inc', 'Habby', 'Archosaur Games', 'Moonton',
  
  // European Developers  
  'CD Projekt', 'Paradox', 'Coffee Stain', 'Avalanche', 'Frontier', 'Codemasters', 'Creative Assembly',
  'Sports Interactive', 'Playground Games', 'Rare', 'Media Molecule', 'Guerrilla', 'Supermassive',
  'Team17', 'Rebellion', 'Jagex', 'Feral Interactive', 'Wargaming', 'Gaijin',
  
  // US/Canada Indies
  'Riot Games', 'Epic Games', 'Bungie', 'Insomniac', 'Santa Monica', 'Naughty Dog', 'Sucker Punch',
  'Respawn', 'Infinity Ward', 'Treyarch', 'Sledgehammer', 'Firaxis', '2K Marin', 'Irrational',
  'Obsidian', 'inXile', 'Double Fine', 'Telltale', 'Campo Santo', 'Supergiant', 'Klei',
  'DrinkBox', 'Fullbright', 'The Behemoth', 'Yacht Club', 'Team Cherry', 'Moon Studios',
  
  // Casual/Social Game Makers
  'Big Fish', 'Wooga', 'InnoGames', 'Goodgame', 'Plarium', 'Huuuge Games', 'Product Madness',
  'Aristocrat', 'Scientific Games', 'SciPlay', 'Playstudios', 'DoubleU Games', 'Murka',
  'Melsoft', 'Nexters', 'My.Games', 'G5 Entertainment', 'Nordeus', 'Outfit7', 'Talking Tom'
];

// APP STORE CATEGORIES
const GOOGLE_PLAY_CATEGORIES = [
  'GAME', 'GAME_ACTION', 'GAME_ADVENTURE', 'GAME_ARCADE', 'GAME_BOARD', 'GAME_CARD',
  'GAME_CASINO', 'GAME_CASUAL', 'GAME_EDUCATIONAL', 'GAME_MUSIC', 'GAME_PUZZLE',
  'GAME_RACING', 'GAME_ROLE_PLAYING', 'GAME_SIMULATION', 'GAME_SPORTS', 'GAME_STRATEGY',
  'GAME_TRIVIA', 'GAME_WORD', 'APPLICATION', 'ART_AND_DESIGN', 'AUTO_AND_VEHICLES',
  'BEAUTY', 'BOOKS_AND_REFERENCE', 'BUSINESS', 'COMICS', 'COMMUNICATION', 'DATING',
  'EDUCATION', 'ENTERTAINMENT', 'EVENTS', 'FINANCE', 'FOOD_AND_DRINK', 'HEALTH_AND_FITNESS',
  'HOUSE_AND_HOME', 'LIBRARIES_AND_DEMO', 'LIFESTYLE', 'MAPS_AND_NAVIGATION', 'MEDICAL',
  'MUSIC_AND_AUDIO', 'NEWS_AND_MAGAZINES', 'PARENTING', 'PERSONALIZATION', 'PHOTOGRAPHY',
  'PRODUCTIVITY', 'SHOPPING', 'SOCIAL', 'SPORTS', 'TOOLS', 'TRAVEL_AND_LOCAL',
  'VIDEO_PLAYERS', 'WEATHER'
];

const IOS_CATEGORIES = [
  'Games', 'Action Games', 'Adventure Games', 'Arcade Games', 'Board Games', 'Card Games',
  'Casino Games', 'Casual Games', 'Family Games', 'Music Games', 'Puzzle Games',
  'Racing Games', 'Role Playing Games', 'Simulation Games', 'Sports Games', 'Strategy Games',
  'Trivia Games', 'Word Games', 'Books', 'Business', 'Developer Tools', 'Education',
  'Entertainment', 'Finance', 'Food & Drink', 'Graphics & Design', 'Health & Fitness',
  'Kids', 'Lifestyle', 'Magazines & Newspapers', 'Medical', 'Music', 'Navigation',
  'News', 'Photo & Video', 'Productivity', 'Reference', 'Shopping', 'Social Networking',
  'Sports', 'Travel', 'Utilities', 'Weather'
];

// GEOGRAPHIC REGIONS (App Store country codes)
const REGIONS = [
  // North America
  'us', 'ca', 'mx',
  // Europe
  'gb', 'de', 'fr', 'it', 'es', 'nl', 'be', 'at', 'ch', 'se', 'no', 'dk', 'fi',
  'pl', 'cz', 'pt', 'ie', 'gr', 'ro', 'hu', 'ua', 'ru',
  // Asia Pacific
  'jp', 'kr', 'cn', 'tw', 'hk', 'sg', 'au', 'nz', 'in', 'id', 'th', 'my', 'ph', 'vn',
  // Middle East
  'ae', 'sa', 'il', 'tr',
  // Latin America
  'br', 'ar', 'cl', 'co', 'pe',
  // Africa
  'za', 'eg', 'ng', 'ke'
];

// DOWNLOAD TIERS for targeting
const DOWNLOAD_TIERS = [
  { min: '1000', max: '5000', name: 'micro' },
  { min: '5000', max: '10000', name: 'small' },
  { min: '10000', max: '50000', name: 'growing' },
  { min: '50000', max: '100000', name: 'established' },
  { min: '100000', max: '500000', name: 'popular' },
  { min: '500000', max: '1000000', name: 'hit' },
  { min: '1000000', max: '5000000', name: 'mega' },
  { min: '5000000', max: '10000000', name: 'blockbuster' },
  { min: '10000000', max: '50000000', name: 'viral' },
  { min: '50000000', max: '100000000', name: 'unicorn' }
];

// SEARCH QUERY TEMPLATES
const SEARCH_TEMPLATES = [
  // Game Genre Searches
  'mobile {genre} games', '{genre} games android', '{genre} games ios', 'best {genre} games 2024',
  'top {genre} games', 'new {genre} games', 'free {genre} games', 'popular {genre} games',
  'offline {genre} games', 'multiplayer {genre} games', 'single player {genre} games',
  
  // Company-specific
  '{company} games', '{company} mobile games', '{company} new games', '{company} apps',
  'games like {company}', '{company} alternatives', '{company} competitors',
  
  // Feature-based
  'games with ads', 'games with in-app purchases', 'games with rewarded video',
  'games monetization', 'games with interstitial ads', 'games ad revenue',
  
  // Platform specific  
  'android games {region}', 'ios games {region}', 'google play games {region}',
  'app store games {region}', 'mobile games {region}', 'tablet games',
  
  // Trend-based
  'trending games', 'viral games', 'hit games 2024', 'top grossing games',
  'most downloaded games', 'rising games', 'chart topping games',
  
  // Ad-tech related
  'games using admob', 'games using unity ads', 'games using ironsource',
  'games using applovin', 'games using facebook ads', 'games using mopub',
  'games ad mediation', 'games programmatic ads', 'games video ads'
];

// GAME GENRES for search queries
const GAME_GENRES = [
  'action', 'adventure', 'arcade', 'puzzle', 'strategy', 'rpg', 'role playing',
  'simulation', 'sports', 'racing', 'casual', 'hypercasual', 'idle', 'clicker',
  'shooter', 'fps', 'battle royale', 'moba', 'card', 'board', 'casino', 'slots',
  'match 3', 'match three', 'bubble shooter', 'word', 'trivia', 'quiz',
  'tower defense', 'city builder', 'farm', 'farming', 'cooking', 'restaurant',
  'fashion', 'dress up', 'makeover', 'pet', 'virtual pet', 'dating sim',
  'horror', 'survival', 'zombie', 'platformer', 'runner', 'endless runner',
  'fighting', 'wrestling', 'boxing', 'martial arts', 'ninja', 'samurai',
  'war', 'military', 'tank', 'airplane', 'ship', 'submarine', 'space',
  'sci-fi', 'fantasy', 'medieval', 'kingdom', 'empire', 'civilization',
  'tycoon', 'management', 'business', 'real estate', 'restaurant management',
  'music', 'rhythm', 'dance', 'karaoke', 'piano', 'guitar', 'dj',
  'educational', 'kids', 'learning', 'brain training', 'memory', 'logic',
  'sandbox', 'crafting', 'building', 'minecraft style', 'voxel',
  'anime', 'manga', 'gacha', 'waifu', 'otome', 'visual novel'
];

// APP TYPES
const APP_TYPES = [
  'utility', 'productivity', 'social', 'messaging', 'photo', 'video', 'camera',
  'fitness', 'health', 'meditation', 'sleep', 'workout', 'running', 'cycling',
  'finance', 'banking', 'investing', 'budget', 'expense', 'crypto', 'trading',
  'shopping', 'ecommerce', 'deals', 'coupons', 'marketplace', 'auction',
  'travel', 'hotel', 'flight', 'vacation', 'maps', 'navigation', 'gps',
  'food', 'delivery', 'restaurant', 'recipe', 'diet', 'nutrition', 'calorie',
  'news', 'magazine', 'newspaper', 'blog', 'reading', 'ebook', 'audiobook',
  'music', 'streaming', 'podcast', 'radio', 'audio', 'sound',
  'dating', 'relationship', 'meetup', 'networking', 'professional',
  'education', 'language', 'learning', 'courses', 'tutorial', 'skill',
  'weather', 'clock', 'alarm', 'timer', 'calendar', 'reminder', 'notes',
  'vpn', 'security', 'antivirus', 'cleaner', 'booster', 'battery',
  'wallpaper', 'theme', 'launcher', 'keyboard', 'emoji', 'sticker'
];

function generateUUID() {
  return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, function(c) {
    const r = Math.random() * 16 | 0;
    const v = c === 'x' ? r : (r & 0x3 | 0x8);
    return v.toString(16);
  });
}

function generateSources() {
  const sources = [];
  const usedKeys = new Set();
  
  // Helper to create unique key
  const makeKey = (type, name, query, cat, region) => 
    `${type}|${name}|${query}|${cat}|${region}`;
  
  // 1. Company-based sources (for each company, create google_play and itunes sources)
  for (const company of GAME_COMPANIES) {
    for (const region of REGIONS.slice(0, 20)) { // Top 20 regions
      // Google Play
      for (const cat of GOOGLE_PLAY_CATEGORIES.slice(0, 10)) { // Top 10 categories
        const key = makeKey('google_play', company, `${company} games`, cat, region);
        if (!usedKeys.has(key)) {
          usedKeys.add(key);
          const tier = DOWNLOAD_TIERS[Math.floor(Math.random() * DOWNLOAD_TIERS.length)];
          sources.push({
            id: generateUUID(),
            source_type: 'google_play',
            source_name: `${company} - ${cat}`,
            search_query: `${company.toLowerCase()} games ${cat.toLowerCase().replace(/_/g, ' ')}`,
            category: cat,
            region: region,
            min_downloads: tier.min,
            max_downloads: tier.max,
            status: 'active',
            scrape_interval_hours: 24 + Math.floor(Math.random() * 48)
          });
        }
      }
      
      // iTunes
      for (const cat of IOS_CATEGORIES.slice(0, 10)) {
        const key = makeKey('itunes', company, `${company} games`, cat, region);
        if (!usedKeys.has(key)) {
          usedKeys.add(key);
          const tier = DOWNLOAD_TIERS[Math.floor(Math.random() * DOWNLOAD_TIERS.length)];
          sources.push({
            id: generateUUID(),
            source_type: 'itunes',
            source_name: `${company} - ${cat}`,
            search_query: `${company.toLowerCase()} ${cat.toLowerCase()}`,
            category: cat,
            region: region,
            min_downloads: tier.min,
            max_downloads: tier.max,
            status: 'active',
            scrape_interval_hours: 24 + Math.floor(Math.random() * 48)
          });
        }
      }
      
      if (sources.length >= 10000) break;
    }
    if (sources.length >= 10000) break;
  }
  
  // 2. Genre-based sources
  for (const genre of GAME_GENRES) {
    for (const region of REGIONS.slice(0, 15)) {
      for (const cat of GOOGLE_PLAY_CATEGORIES.filter(c => c.startsWith('GAME')).slice(0, 5)) {
        if (sources.length >= 12000) break;
        const key = makeKey('google_play', genre, `${genre} games`, cat, region);
        if (!usedKeys.has(key)) {
          usedKeys.add(key);
          const tier = DOWNLOAD_TIERS[Math.floor(Math.random() * DOWNLOAD_TIERS.length)];
          sources.push({
            id: generateUUID(),
            source_type: 'google_play',
            source_name: `${genre.charAt(0).toUpperCase() + genre.slice(1)} Games - ${region.toUpperCase()}`,
            search_query: `${genre} mobile games`,
            category: cat,
            region: region,
            min_downloads: tier.min,
            max_downloads: tier.max,
            status: 'active',
            scrape_interval_hours: 24 + Math.floor(Math.random() * 72)
          });
        }
      }
    }
  }
  
  // 3. App type sources (non-game)
  for (const appType of APP_TYPES) {
    for (const region of REGIONS.slice(0, 10)) {
      for (const cat of GOOGLE_PLAY_CATEGORIES.filter(c => !c.startsWith('GAME')).slice(0, 5)) {
        if (sources.length >= 15000) break;
        const key = makeKey('google_play', appType, `${appType} apps`, cat, region);
        if (!usedKeys.has(key)) {
          usedKeys.add(key);
          const tier = DOWNLOAD_TIERS[Math.floor(Math.random() * DOWNLOAD_TIERS.length)];
          sources.push({
            id: generateUUID(),
            source_type: 'google_play',
            source_name: `${appType.charAt(0).toUpperCase() + appType.slice(1)} Apps - ${region.toUpperCase()}`,
            search_query: `best ${appType} app`,
            category: cat,
            region: region,
            min_downloads: tier.min,
            max_downloads: tier.max,
            status: 'active',
            scrape_interval_hours: 48 + Math.floor(Math.random() * 72)
          });
        }
      }
    }
  }
  
  // 4. Trending/Chart sources
  const chartTypes = [
    'top free', 'top paid', 'top grossing', 'trending', 'new releases',
    'editors choice', 'featured', 'recommended', 'rising'
  ];
  
  for (const chartType of chartTypes) {
    for (const region of REGIONS) {
      for (const cat of GOOGLE_PLAY_CATEGORIES) {
        if (sources.length >= 18000) break;
        const key = makeKey('google_play', chartType, `${chartType} ${cat}`, cat, region);
        if (!usedKeys.has(key)) {
          usedKeys.add(key);
          sources.push({
            id: generateUUID(),
            source_type: 'google_play',
            source_name: `${chartType.charAt(0).toUpperCase() + chartType.slice(1)} - ${cat} - ${region.toUpperCase()}`,
            search_query: `${chartType} ${cat.toLowerCase().replace(/_/g, ' ')} apps`,
            category: cat,
            region: region,
            min_downloads: '10000',
            max_downloads: '100000000',
            status: 'active',
            scrape_interval_hours: 12 + Math.floor(Math.random() * 24)
          });
        }
      }
    }
  }
  
  // 5. Ad-network related sources
  const adNetworks = [
    'admob', 'unity ads', 'ironsource', 'applovin', 'facebook audience network',
    'mopub', 'chartboost', 'vungle', 'tapjoy', 'adcolony', 'inmobi', 'fyber'
  ];
  
  for (const adNetwork of adNetworks) {
    for (const genre of GAME_GENRES.slice(0, 20)) {
      for (const region of REGIONS.slice(0, 10)) {
        if (sources.length >= 20000) break;
        const key = makeKey('google_play', adNetwork, `${genre} ${adNetwork}`, 'GAME', region);
        if (!usedKeys.has(key)) {
          usedKeys.add(key);
          sources.push({
            id: generateUUID(),
            source_type: 'google_play',
            source_name: `${adNetwork.toUpperCase()} ${genre} - ${region.toUpperCase()}`,
            search_query: `${genre} games monetization ${adNetwork}`,
            category: 'GAME',
            region: region,
            min_downloads: '50000',
            max_downloads: '10000000',
            status: 'active',
            scrape_interval_hours: 24 + Math.floor(Math.random() * 48)
          });
        }
      }
    }
  }
  
  return sources.slice(0, 10000); // Return exactly 10,000
}

async function insertSources() {
  const client = new Client({
    host: 'localhost',
    port: 6432,
    database: 'adproject',
    user: 'adproject',
    password: 'adproject123'
  });
  
  try {
    await client.connect();
    console.log('Connected to database');
    
    // Get current count
    const countResult = await client.query('SELECT COUNT(*) FROM sa_sources');
    console.log(`Current sources: ${countResult.rows[0].count}`);
    
    // Generate sources
    console.log('Generating 10,000 sources...');
    const sources = generateSources();
    console.log(`Generated ${sources.length} sources`);
    
    // Insert in batches
    const batchSize = 500;
    let inserted = 0;
    
    for (let i = 0; i < sources.length; i += batchSize) {
      const batch = sources.slice(i, i + batchSize);
      
      const values = batch.map((s, idx) => {
        const offset = i + idx;
        return `($${offset * 9 + 1}, $${offset * 9 + 2}, $${offset * 9 + 3}, $${offset * 9 + 4}, $${offset * 9 + 5}, $${offset * 9 + 6}, $${offset * 9 + 7}, $${offset * 9 + 8}, $${offset * 9 + 9})`;
      }).join(',');
      
      // Use simpler approach - one insert per source for reliability
      for (const source of batch) {
        try {
          await client.query(`
            INSERT INTO sa_sources (
              id, source_type, source_name, search_query, category, 
              region, min_downloads, max_downloads, status, scrape_interval_hours
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (id) DO NOTHING
          `, [
            source.id,
            source.source_type,
            source.source_name,
            source.search_query,
            source.category,
            source.region,
            source.min_downloads,
            source.max_downloads,
            source.status,
            source.scrape_interval_hours
          ]);
          inserted++;
        } catch (err) {
          // Skip duplicates
        }
      }
      
      console.log(`Inserted batch ${Math.floor(i / batchSize) + 1}/${Math.ceil(sources.length / batchSize)} (${inserted} total)`);
    }
    
    // Get final count
    const finalCount = await client.query('SELECT COUNT(*) FROM sa_sources');
    console.log(`\n✅ Final source count: ${finalCount.rows[0].count}`);
    
    // Show breakdown
    const breakdown = await client.query(`
      SELECT source_type, COUNT(*) as count 
      FROM sa_sources 
      GROUP BY source_type 
      ORDER BY count DESC
    `);
    console.log('\nSource breakdown:');
    breakdown.rows.forEach(r => console.log(`  ${r.source_type}: ${r.count}`));
    
  } catch (err) {
    console.error('Error:', err);
  } finally {
    await client.end();
  }
}

// Run
insertSources().catch(console.error);
