import { generateLandscapeId } from '../auctionIdempotency';

describe('auctionIdempotency', () => {
  describe('generateLandscapeId', () => {
    it('returns a string starting with ls_', () => {
      const id = generateLandscapeId('req-123', 'bid-456');
      expect(id).toMatch(/^ls_[a-f0-9]{24}$/);
    });

    it('generates different IDs for different inputs', () => {
      const id1 = generateLandscapeId('req-1', 'bid-1');
      const id2 = generateLandscapeId('req-2', 'bid-2');
      expect(id1).not.toBe(id2);
    });

    it('handles missing bidId', () => {
      const id = generateLandscapeId('req-123');
      expect(id).toMatch(/^ls_[a-f0-9]{24}$/);
    });

    it('is deterministic - same inputs produce same output', () => {
      const id1 = generateLandscapeId('req-123', 'bid-456');
      const id2 = generateLandscapeId('req-123', 'bid-456');
      const id3 = generateLandscapeId('req-123', 'bid-456');
      expect(id1).toBe(id2);
      expect(id2).toBe(id3);
    });

    it('is deterministic for no-bid case', () => {
      const id1 = generateLandscapeId('req-123');
      const id2 = generateLandscapeId('req-123');
      expect(id1).toBe(id2);
    });
  });
});
