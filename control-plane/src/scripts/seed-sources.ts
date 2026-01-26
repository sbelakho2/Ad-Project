
import { queryOperator } from '../database';
import { logger } from '../logger';
import dotenv from 'dotenv';

dotenv.config();

async function seedSources() {
  logger.info('Seeding high-value sources...');
  
  const sources = [
    { type: 'itunes', name: 'Casual Games - US', query: 'match 3', category: 'Games', region: 'us' },
    { type: 'itunes', name: 'Hypercasual - US', query: 'runner game', category: 'Games', region: 'us' },
    { type: 'itunes', name: 'Puzzle Games - US', query: 'puzzle', category: 'Games', region: 'us' },
    { type: 'itunes', name: 'Ad-Supported Apps', query: 'free game with ads', category: 'Games', region: 'us' },
    { type: 'itunes', name: 'Multiplayer Games', query: 'online multiplayer', category: 'Games', region: 'us' }
  ];

  for (const s of sources) {
    try {
      await queryOperator(`
        INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status)
        VALUES ($1, $2, $3, $4, $5, 'active')
        ON CONFLICT DO NOTHING
      `, [s.type, s.name, s.query, s.category, s.region]);
      logger.info(`Seeded source: ${s.name}`);
    } catch (err) {
      logger.error('Failed to seed source', { name: s.name, error: (err as Error).message });
    }
  }
  
  logger.info('Seeding complete.');
  process.exit(0);
}

seedSources();
