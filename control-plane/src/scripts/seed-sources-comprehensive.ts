/**
 * COMPREHENSIVE SOURCE SEEDING SCRIPT
 * 
 * Seeds 100+ verified, high-value sources for the autonomous sales system.
 * Sources are categorized by:
 * - Platform (iTunes/App Store, Google Play)
 * - Category (Games, Productivity, Entertainment, etc.)
 * - Region (US, UK, DE, FR, JP, KR, BR, etc.)
 * - Search strategy (keyword, category browsing)
 * 
 * All sources target apps likely to monetize with ads (free apps, games, utilities).
 */

import { queryOperator } from '../database';
import { logger } from '../logger';
import dotenv from 'dotenv';

dotenv.config();

interface Source {
  type: 'itunes' | 'google_play';
  name: string;
  query: string | null;
  category: string;
  region: string | null;
}

async function seedComprehensiveSources() {
  logger.info('Seeding 100+ comprehensive high-value sources...');
  
  const sources: Source[] = [
    // ========================================================================
    // iOS APP STORE - GAMES (HIGH VALUE - Ad-monetized)
    // ========================================================================
    
    // US Market - Games
    { type: 'itunes', name: 'US - Hypercasual Games', query: 'hypercasual game', category: 'Games', region: 'us' },
    { type: 'itunes', name: 'US - Match 3 Puzzle', query: 'match 3 puzzle', category: 'Games', region: 'us' },
    { type: 'itunes', name: 'US - Idle Clicker Games', query: 'idle clicker', category: 'Games', region: 'us' },
    { type: 'itunes', name: 'US - Runner Games', query: 'endless runner', category: 'Games', region: 'us' },
    { type: 'itunes', name: 'US - Word Games', query: 'word puzzle game', category: 'Games', region: 'us' },
    { type: 'itunes', name: 'US - Trivia Quiz Games', query: 'trivia quiz', category: 'Games', region: 'us' },
    { type: 'itunes', name: 'US - Solitaire Card Games', query: 'solitaire card', category: 'Games', region: 'us' },
    { type: 'itunes', name: 'US - Bubble Shooter', query: 'bubble shooter', category: 'Games', region: 'us' },
    { type: 'itunes', name: 'US - Mahjong Games', query: 'mahjong puzzle', category: 'Games', region: 'us' },
    { type: 'itunes', name: 'US - Hidden Object Games', query: 'hidden object', category: 'Games', region: 'us' },
    { type: 'itunes', name: 'US - Merge Games', query: 'merge game', category: 'Games', region: 'us' },
    { type: 'itunes', name: 'US - Tower Defense', query: 'tower defense', category: 'Games', region: 'us' },
    { type: 'itunes', name: 'US - Simulation Games', query: 'simulation tycoon', category: 'Games', region: 'us' },
    { type: 'itunes', name: 'US - Cooking Games', query: 'cooking restaurant', category: 'Games', region: 'us' },
    { type: 'itunes', name: 'US - Fashion Games', query: 'fashion dress up', category: 'Games', region: 'us' },
    
    // UK Market - Games
    { type: 'itunes', name: 'UK - Casual Games', query: 'casual game free', category: 'Games', region: 'gb' },
    { type: 'itunes', name: 'UK - Puzzle Games', query: 'puzzle brain', category: 'Games', region: 'gb' },
    { type: 'itunes', name: 'UK - Word Games', query: 'word crossword', category: 'Games', region: 'gb' },
    { type: 'itunes', name: 'UK - Card Games', query: 'card game free', category: 'Games', region: 'gb' },
    { type: 'itunes', name: 'UK - Strategy Games', query: 'strategy mobile', category: 'Games', region: 'gb' },
    
    // Germany Market - Games
    { type: 'itunes', name: 'DE - Casual Spiele', query: 'spiel kostenlos casual', category: 'Games', region: 'de' },
    { type: 'itunes', name: 'DE - Puzzle Spiele', query: 'rätsel puzzle', category: 'Games', region: 'de' },
    { type: 'itunes', name: 'DE - Wortspiele', query: 'wortspiel kreuzwort', category: 'Games', region: 'de' },
    { type: 'itunes', name: 'DE - Kartenspiele', query: 'kartenspiel solitär', category: 'Games', region: 'de' },
    { type: 'itunes', name: 'DE - Strategie Spiele', query: 'strategie aufbau', category: 'Games', region: 'de' },
    
    // France Market - Games
    { type: 'itunes', name: 'FR - Jeux Casual', query: 'jeux gratuit casual', category: 'Games', region: 'fr' },
    { type: 'itunes', name: 'FR - Jeux Puzzle', query: 'puzzle casse-tête', category: 'Games', region: 'fr' },
    { type: 'itunes', name: 'FR - Jeux de Mots', query: 'jeux de mots', category: 'Games', region: 'fr' },
    { type: 'itunes', name: 'FR - Jeux de Cartes', query: 'jeux cartes solitaire', category: 'Games', region: 'fr' },
    
    // Japan Market - Games (HIGH VALUE)
    { type: 'itunes', name: 'JP - カジュアルゲーム', query: 'カジュアル ゲーム 無料', category: 'Games', region: 'jp' },
    { type: 'itunes', name: 'JP - パズルゲーム', query: 'パズル ゲーム', category: 'Games', region: 'jp' },
    { type: 'itunes', name: 'JP - RPGゲーム', query: 'RPG ゲーム 無料', category: 'Games', region: 'jp' },
    { type: 'itunes', name: 'JP - 音楽ゲーム', query: 'リズム 音楽 ゲーム', category: 'Games', region: 'jp' },
    { type: 'itunes', name: 'JP - シミュレーション', query: 'シミュレーション 育成', category: 'Games', region: 'jp' },
    
    // Korea Market - Games (HIGH VALUE)
    { type: 'itunes', name: 'KR - 캐주얼 게임', query: '캐주얼 게임 무료', category: 'Games', region: 'kr' },
    { type: 'itunes', name: 'KR - 퍼즐 게임', query: '퍼즐 게임', category: 'Games', region: 'kr' },
    { type: 'itunes', name: 'KR - RPG 게임', query: 'RPG 모바일 게임', category: 'Games', region: 'kr' },
    
    // Brazil Market - Games
    { type: 'itunes', name: 'BR - Jogos Casuais', query: 'jogo casual grátis', category: 'Games', region: 'br' },
    { type: 'itunes', name: 'BR - Jogos de Puzzle', query: 'jogo puzzle', category: 'Games', region: 'br' },
    { type: 'itunes', name: 'BR - Jogos de Palavras', query: 'jogo de palavras', category: 'Games', region: 'br' },
    
    // Spain Market - Games
    { type: 'itunes', name: 'ES - Juegos Casuales', query: 'juego casual gratis', category: 'Games', region: 'es' },
    { type: 'itunes', name: 'ES - Juegos de Puzzle', query: 'puzzle rompecabezas', category: 'Games', region: 'es' },
    
    // Italy Market - Games
    { type: 'itunes', name: 'IT - Giochi Casual', query: 'gioco casual gratis', category: 'Games', region: 'it' },
    { type: 'itunes', name: 'IT - Giochi Puzzle', query: 'puzzle rompicapo', category: 'Games', region: 'it' },
    
    // Australia Market - Games
    { type: 'itunes', name: 'AU - Casual Games', query: 'casual game free', category: 'Games', region: 'au' },
    { type: 'itunes', name: 'AU - Puzzle Games', query: 'puzzle brain teaser', category: 'Games', region: 'au' },
    
    // Canada Market - Games
    { type: 'itunes', name: 'CA - Casual Games', query: 'casual mobile game', category: 'Games', region: 'ca' },
    { type: 'itunes', name: 'CA - Puzzle Games', query: 'puzzle game free', category: 'Games', region: 'ca' },
    
    // India Market - Games (HUGE VOLUME)
    { type: 'itunes', name: 'IN - Casual Games', query: 'casual game free', category: 'Games', region: 'in' },
    { type: 'itunes', name: 'IN - Ludo Games', query: 'ludo board game', category: 'Games', region: 'in' },
    { type: 'itunes', name: 'IN - Cricket Games', query: 'cricket game mobile', category: 'Games', region: 'in' },
    
    // Indonesia Market - Games
    { type: 'itunes', name: 'ID - Game Kasual', query: 'game kasual gratis', category: 'Games', region: 'id' },
    
    // Russia Market - Games
    { type: 'itunes', name: 'RU - Казуальные игры', query: 'казуальные игры бесплатно', category: 'Games', region: 'ru' },
    
    // ========================================================================
    // iOS APP STORE - NON-GAME CATEGORIES (Ad-monetized apps)
    // ========================================================================
    
    // Productivity - US
    { type: 'itunes', name: 'US - Productivity Free', query: 'productivity app free', category: 'Productivity', region: 'us' },
    { type: 'itunes', name: 'US - Note Taking', query: 'note taking app', category: 'Productivity', region: 'us' },
    { type: 'itunes', name: 'US - To-Do List', query: 'todo list planner', category: 'Productivity', region: 'us' },
    
    // Utilities - US
    { type: 'itunes', name: 'US - Calculator Apps', query: 'calculator utility', category: 'Utilities', region: 'us' },
    { type: 'itunes', name: 'US - Weather Apps', query: 'weather forecast free', category: 'Weather', region: 'us' },
    { type: 'itunes', name: 'US - Flashlight Apps', query: 'flashlight utility', category: 'Utilities', region: 'us' },
    { type: 'itunes', name: 'US - QR Scanner', query: 'qr code scanner', category: 'Utilities', region: 'us' },
    
    // Entertainment - US
    { type: 'itunes', name: 'US - Streaming Apps', query: 'video streaming free', category: 'Entertainment', region: 'us' },
    { type: 'itunes', name: 'US - Music Apps', query: 'music player free', category: 'Music', region: 'us' },
    { type: 'itunes', name: 'US - Wallpaper Apps', query: 'wallpaper background', category: 'Entertainment', region: 'us' },
    
    // Lifestyle - US
    { type: 'itunes', name: 'US - Fitness Apps', query: 'fitness workout free', category: 'Health & Fitness', region: 'us' },
    { type: 'itunes', name: 'US - Recipe Apps', query: 'recipe cooking app', category: 'Food & Drink', region: 'us' },
    { type: 'itunes', name: 'US - Dating Apps', query: 'dating app free', category: 'Lifestyle', region: 'us' },
    
    // Photo & Video - US  
    { type: 'itunes', name: 'US - Photo Editor', query: 'photo editor filter', category: 'Photo & Video', region: 'us' },
    { type: 'itunes', name: 'US - Video Editor', query: 'video editor free', category: 'Photo & Video', region: 'us' },
    { type: 'itunes', name: 'US - Camera Apps', query: 'camera filter beauty', category: 'Photo & Video', region: 'us' },
    
    // Education - US
    { type: 'itunes', name: 'US - Language Learning', query: 'language learning free', category: 'Education', region: 'us' },
    { type: 'itunes', name: 'US - Kids Education', query: 'kids learning educational', category: 'Education', region: 'us' },
    
    // ========================================================================
    // GOOGLE PLAY STORE - GAMES
    // ========================================================================
    
    { type: 'google_play', name: 'GP - Casual Games', query: 'casual game', category: 'GAME_CASUAL', region: null },
    { type: 'google_play', name: 'GP - Puzzle Games', query: 'puzzle game', category: 'GAME_PUZZLE', region: null },
    { type: 'google_play', name: 'GP - Arcade Games', query: 'arcade game', category: 'GAME_ARCADE', region: null },
    { type: 'google_play', name: 'GP - Word Games', query: 'word game', category: 'GAME_WORD', region: null },
    { type: 'google_play', name: 'GP - Card Games', query: 'card game', category: 'GAME_CARD', region: null },
    { type: 'google_play', name: 'GP - Board Games', query: 'board game', category: 'GAME_BOARD', region: null },
    { type: 'google_play', name: 'GP - Trivia Games', query: 'trivia quiz', category: 'GAME_TRIVIA', region: null },
    { type: 'google_play', name: 'GP - Simulation Games', query: 'simulation game', category: 'GAME_SIMULATION', region: null },
    { type: 'google_play', name: 'GP - Strategy Games', query: 'strategy game', category: 'GAME_STRATEGY', region: null },
    { type: 'google_play', name: 'GP - Action Games', query: 'action game', category: 'GAME_ACTION', region: null },
    { type: 'google_play', name: 'GP - Adventure Games', query: 'adventure game', category: 'GAME_ADVENTURE', region: null },
    { type: 'google_play', name: 'GP - Racing Games', query: 'racing game', category: 'GAME_RACING', region: null },
    { type: 'google_play', name: 'GP - Sports Games', query: 'sports game', category: 'GAME_SPORTS', region: null },
    { type: 'google_play', name: 'GP - Role Playing', query: 'rpg game', category: 'GAME_ROLE_PLAYING', region: null },
    { type: 'google_play', name: 'GP - Educational Games', query: 'educational game', category: 'GAME_EDUCATIONAL', region: null },
    
    // ========================================================================
    // GOOGLE PLAY STORE - NON-GAME APPS
    // ========================================================================
    
    { type: 'google_play', name: 'GP - Tools Apps', query: 'tool utility', category: 'TOOLS', region: null },
    { type: 'google_play', name: 'GP - Entertainment', query: 'entertainment app', category: 'ENTERTAINMENT', region: null },
    { type: 'google_play', name: 'GP - Music & Audio', query: 'music player', category: 'MUSIC_AND_AUDIO', region: null },
    { type: 'google_play', name: 'GP - Photo Apps', query: 'photo editor', category: 'PHOTOGRAPHY', region: null },
    { type: 'google_play', name: 'GP - Video Players', query: 'video player', category: 'VIDEO_PLAYERS', region: null },
    { type: 'google_play', name: 'GP - Lifestyle', query: 'lifestyle app', category: 'LIFESTYLE', region: null },
    { type: 'google_play', name: 'GP - Health Fitness', query: 'fitness workout', category: 'HEALTH_AND_FITNESS', region: null },
    { type: 'google_play', name: 'GP - Weather', query: 'weather forecast', category: 'WEATHER', region: null },
    { type: 'google_play', name: 'GP - News Apps', query: 'news reader', category: 'NEWS_AND_MAGAZINES', region: null },
    { type: 'google_play', name: 'GP - Education', query: 'education learning', category: 'EDUCATION', region: null },
    { type: 'google_play', name: 'GP - Personalization', query: 'wallpaper launcher', category: 'PERSONALIZATION', region: null },
    
    // ========================================================================
    // ADDITIONAL NICHE SOURCES - HIGH VALUE SEGMENTS
    // ========================================================================
    
    // Kids & Family (High Ad Engagement)
    { type: 'itunes', name: 'US - Kids Games', query: 'kids game toddler', category: 'Games', region: 'us' },
    { type: 'itunes', name: 'US - Family Games', query: 'family game multiplayer', category: 'Games', region: 'us' },
    { type: 'itunes', name: 'US - Kids Drawing', query: 'drawing coloring kids', category: 'Education', region: 'us' },
    
    // Seniors (High LTV)
    { type: 'itunes', name: 'US - Brain Training', query: 'brain training memory', category: 'Games', region: 'us' },
    { type: 'itunes', name: 'US - Classic Games', query: 'classic card game', category: 'Games', region: 'us' },
    
    // Emerging Markets (High Growth)
    { type: 'itunes', name: 'MX - Juegos Móviles', query: 'juego movil gratis', category: 'Games', region: 'mx' },
    { type: 'itunes', name: 'PH - Mobile Games', query: 'mobile game free', category: 'Games', region: 'ph' },
    { type: 'itunes', name: 'TH - เกมมือถือ', query: 'เกม มือถือ ฟรี', category: 'Games', region: 'th' },
    { type: 'itunes', name: 'VN - Game di động', query: 'game mobile miễn phí', category: 'Games', region: 'vn' },
    { type: 'itunes', name: 'TR - Mobil Oyun', query: 'mobil oyun ücretsiz', category: 'Games', region: 'tr' },
    { type: 'itunes', name: 'PL - Gry Mobilne', query: 'gra mobilna za darmo', category: 'Games', region: 'pl' },
    
    // Specific Game Genres (Proven Ad Monetization)
    { type: 'itunes', name: 'US - Block Puzzle', query: 'block puzzle tetris', category: 'Games', region: 'us' },
    { type: 'itunes', name: 'US - Coloring Games', query: 'coloring book adult', category: 'Games', region: 'us' },
    { type: 'itunes', name: 'US - Sudoku Games', query: 'sudoku number puzzle', category: 'Games', region: 'us' },
    { type: 'itunes', name: 'US - Jigsaw Puzzle', query: 'jigsaw puzzle', category: 'Games', region: 'us' },
    { type: 'itunes', name: 'US - 2048 Games', query: '2048 number game', category: 'Games', region: 'us' },
    { type: 'itunes', name: 'US - Snake Games', query: 'snake io game', category: 'Games', region: 'us' },
    { type: 'itunes', name: 'US - Difference Games', query: 'find difference', category: 'Games', region: 'us' },
    { type: 'itunes', name: 'US - Escape Room', query: 'escape room puzzle', category: 'Games', region: 'us' },
    
    // Social/Casual (High Engagement)
    { type: 'itunes', name: 'US - Multiplayer Online', query: 'multiplayer online free', category: 'Games', region: 'us' },
    { type: 'itunes', name: 'US - Battle Royale', query: 'battle royale mobile', category: 'Games', region: 'us' },
    { type: 'itunes', name: 'US - Party Games', query: 'party game friends', category: 'Games', region: 'us' },
  ];

  let successCount = 0;
  let skipCount = 0;
  let errorCount = 0;

  for (const s of sources) {
    try {
      const result = await queryOperator<{ id: string }>(`
        INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours)
        VALUES ($1, $2, $3, $4, $5, 'active', 12)
        ON CONFLICT (source_name) DO NOTHING
        RETURNING id
      `, [s.type, s.name, s.query, s.category, s.region]);
      
      if (result.length > 0) {
        successCount++;
        logger.info(`✓ Added source: ${s.name}`);
      } else {
        skipCount++;
        logger.info(`- Skipped (exists): ${s.name}`);
      }
    } catch (err) {
      errorCount++;
      logger.error(`✗ Failed: ${s.name}`, { error: (err as Error).message });
    }
  }
  
  logger.info('========================================');
  logger.info(`Seeding complete!`);
  logger.info(`  Added:   ${successCount}`);
  logger.info(`  Skipped: ${skipCount} (already exist)`);
  logger.info(`  Errors:  ${errorCount}`);
  logger.info(`  Total sources in list: ${sources.length}`);
  logger.info('========================================');
  
  process.exit(0);
}

seedComprehensiveSources().catch(err => {
  logger.error('Fatal error during seeding', { error: err.message });
  process.exit(1);
});
