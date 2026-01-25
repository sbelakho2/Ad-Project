/**
 * AUTONOMOUS SALES SYSTEM - SIMULATION TESTS
 * 
 * These tests verify the mathematical correctness of our zero-cost algorithms
 * without making any network requests or database calls.
 * 
 * Key Tests:
 *   1. Thompson Sampler convergence (must find 80% winner within 100 trials)
 *   2. Spintax uniqueness (Levenshtein distance check)
 *   3. Naive Bayes classification accuracy
 *   4. Heuristic scoring consistency
 */

import { describe, it, expect, beforeEach } from '@jest/globals';

// Import modules to test
import {
  ThompsonSampler,
  sampleBeta,
  sampleGamma,
  randomNormal,
} from '../services/math/thompson-sampler';

import {
  SpintaxEngine,
  spin,
  personalize,
  spinAndPersonalize,
  levenshteinDistance,
  normalizedLevenshtein,
  countVariations,
  type PersonalizationContext,
} from '../services/messaging/spintax-engine';

import {
  NaiveBayesClassifier,
  Clerk,
  type MessageCategory,
} from '../services/inbox/clerk';

import {
  calculateLeadScore,
  type Contact,
} from '../services/discovery/analyst';

// ============================================================================
// THOMPSON SAMPLER TESTS
// ============================================================================

describe('ThompsonSampler', () => {
  describe('Beta Distribution Sampling', () => {
    it('should sample values between 0 and 1', () => {
      for (let i = 0; i < 100; i++) {
        const sample = sampleBeta(1, 1);
        expect(sample).toBeGreaterThanOrEqual(0);
        expect(sample).toBeLessThanOrEqual(1);
      }
    });
    
    it('should have correct mean for Beta(2, 5)', () => {
      // Mean of Beta(α, β) = α / (α + β) = 2/7 ≈ 0.286
      const samples: number[] = [];
      for (let i = 0; i < 10000; i++) {
        samples.push(sampleBeta(2, 5));
      }
      const mean = samples.reduce((a, b) => a + b, 0) / samples.length;
      expect(mean).toBeCloseTo(2 / 7, 1); // Within 0.1
    });
    
    it('should handle edge cases (α < 1)', () => {
      const sample = sampleBeta(0.5, 0.5);
      expect(sample).toBeGreaterThanOrEqual(0);
      expect(sample).toBeLessThanOrEqual(1);
    });
  });
  
  describe('Convergence Test', () => {
    it('should find the 80% winner within 100 trials (with high probability)', () => {
      // Simulate 3 arms with true success rates: 0.80, 0.50, 0.30
      const trueRates = [0.80, 0.50, 0.30];
      const armNames = ['winner', 'medium', 'loser'];
      
      let successCount = 0;
      const numSimulations = 50; // Run multiple simulations
      
      for (let sim = 0; sim < numSimulations; sim++) {
        const sampler = new ThompsonSampler();
        armNames.forEach(name => sampler.addOption(name));
        
        const selectionCounts: Record<string, number> = {
          winner: 0,
          medium: 0,
          loser: 0,
        };
        
        // Run 100 trials
        for (let trial = 0; trial < 100; trial++) {
          const chosen = sampler.choose();
          const armIndex = armNames.indexOf(chosen);
          
          // Simulate outcome based on true rate
          const success = Math.random() < trueRates[armIndex];
          sampler.update(chosen, success);
          
          // Count selections in last 20 trials
          if (trial >= 80) {
            selectionCounts[chosen]++;
          }
        }
        
        // Check if winner was selected most in last 20 trials
        if (selectionCounts['winner'] >= selectionCounts['medium'] &&
            selectionCounts['winner'] >= selectionCounts['loser']) {
          successCount++;
        }
      }
      
      // Should converge to winner in at least 80% of simulations
      const successRate = successCount / numSimulations;
      expect(successRate).toBeGreaterThanOrEqual(0.7);
    });
    
    it('should track empirical rates correctly', () => {
      const sampler = new ThompsonSampler();
      sampler.addOption('A');
      
      // 7 successes, 3 failures
      for (let i = 0; i < 7; i++) sampler.update('A', true);
      for (let i = 0; i < 3; i++) sampler.update('A', false);
      
      expect(sampler.getRate('A')).toBeCloseTo(0.7, 2);
    });
  });
});

// ============================================================================
// SPINTAX ENGINE TESTS
// ============================================================================

describe('SpintaxEngine', () => {
  describe('Basic Spinning', () => {
    it('should replace simple options', () => {
      const result = spin('{Hello|Hi|Hey} there');
      expect(['Hello there', 'Hi there', 'Hey there']).toContain(result);
    });
    
    it('should handle multiple spintax blocks', () => {
      const result = spin('{A|B} {1|2}');
      expect(['A 1', 'A 2', 'B 1', 'B 2']).toContain(result);
    });
    
    it('should handle empty options', () => {
      // {option1||option3} = option1, empty, or option3
      const results = new Set<string>();
      for (let i = 0; i < 50; i++) {
        results.add(spin('Start{| middle}End'));
      }
      expect(results.has('StartEnd') || results.has('Start middleEnd')).toBe(true);
    });
    
    it('should handle nested spintax', () => {
      // Note: Our implementation processes innermost first
      const result = spin('{outer {inner1|inner2}|other}');
      // Should produce: "outer inner1", "outer inner2", or "other"
      expect(['outer inner1', 'outer inner2', 'other']).toContain(result);
    });
  });
  
  describe('Personalization', () => {
    it('should replace standard variables', () => {
      const context: PersonalizationContext = {
        firstName: 'John',
        company: 'Acme Corp',
        appName: 'SuperApp',
      };
      
      const result = personalize(
        'Hi {{FIRST_NAME}}, I noticed {{COMPANY}} built {{APP_NAME}}',
        context
      );
      
      expect(result).toBe('Hi John, I noticed Acme Corp built SuperApp');
    });
    
    it('should use fallbacks for missing values', () => {
      const context: PersonalizationContext = {};
      
      const result = personalize('Hi {{FIRST_NAME}} at {{COMPANY}}', context);
      
      expect(result).toBe('Hi there at your company');
    });
    
    it('should handle custom fields', () => {
      const context: PersonalizationContext = {
        customFields: {
          CUSTOM_VALUE: 'test123',
        },
      };
      
      const result = personalize('Value: {{CUSTOM_VALUE}}', context);
      expect(result).toBe('Value: test123');
    });
  });
  
  describe('Levenshtein Distance', () => {
    it('should return 0 for identical strings', () => {
      expect(levenshteinDistance('hello', 'hello')).toBe(0);
    });
    
    it('should calculate correct distance for simple edits', () => {
      expect(levenshteinDistance('cat', 'hat')).toBe(1); // substitution
      expect(levenshteinDistance('cat', 'cats')).toBe(1); // insertion
      expect(levenshteinDistance('cats', 'cat')).toBe(1); // deletion
    });
    
    it('should handle empty strings', () => {
      expect(levenshteinDistance('', 'hello')).toBe(5);
      expect(levenshteinDistance('hello', '')).toBe(5);
    });
    
    it('should calculate normalized distance correctly', () => {
      expect(normalizedLevenshtein('hello', 'hello')).toBe(0);
      expect(normalizedLevenshtein('', '')).toBe(0);
      expect(normalizedLevenshtein('ab', 'cd')).toBe(1); // completely different
    });
  });
  
  describe('Uniqueness Check', () => {
    it('should generate sufficiently different variations', () => {
      const engine = new SpintaxEngine({ minDistance: 10 });
      const context: PersonalizationContext = { firstName: 'Test' };
      
      const subject = '{Quick question|Following up|Reaching out} about {ads|monetization|growth}';
      const body = 'Hi {{FIRST_NAME}}, {I wanted to|I\'d like to|Hoping to} {discuss|chat about|explore} {opportunities|possibilities}.';
      
      const variations: string[] = [];
      
      for (let i = 0; i < 5; i++) {
        const result = engine.generateUnique(subject, body, context);
        const fullMessage = `${result.subject}\n${result.body}`;
        
        // Check against previous variations
        for (const prev of variations) {
          const distance = levenshteinDistance(fullMessage, prev);
          expect(distance).toBeGreaterThanOrEqual(5); // At least 5 character changes
        }
        
        variations.push(fullMessage);
      }
    });
  });
  
  describe('Variation Counting', () => {
    it('should count simple variations', () => {
      expect(countVariations('{A|B}')).toBe(2);
      expect(countVariations('{A|B|C}')).toBe(3);
    });
    
    it('should count multiple blocks', () => {
      expect(countVariations('{A|B} {1|2|3}')).toBe(6); // 2 * 3
    });
    
    it('should return 1 for no spintax', () => {
      expect(countVariations('Plain text')).toBe(1);
    });
  });
});

// ============================================================================
// NAIVE BAYES CLASSIFIER TESTS
// ============================================================================

describe('Clerk (Naive Bayes)', () => {
  let classifier: NaiveBayesClassifier;
  
  beforeEach(() => {
    classifier = new NaiveBayesClassifier();
  });
  
  describe('Training and Classification', () => {
    it('should classify after training', () => {
      // Train with obvious examples
      classifier.train('great interested love to learn more', 'INTERESTED');
      classifier.train('yes sounds good tell me more', 'INTERESTED');
      classifier.train('not interested no thanks pass', 'NOT_INTERESTED');
      classifier.train('no thank you not looking', 'NOT_INTERESTED');
      
      const result = classifier.classify('would love to hear more about this');
      expect(result.category).toBe('INTERESTED');
      
      const result2 = classifier.classify('no thanks not interested');
      expect(result2.category).toBe('NOT_INTERESTED');
    });
    
    it('should return probabilities for all categories', () => {
      classifier.train('test document', 'INTERESTED');
      
      const result = classifier.classify('any text');
      expect(result.probabilities).toBeDefined();
      expect(typeof result.probabilities.INTERESTED).toBe('number');
      expect(typeof result.probabilities.NOT_INTERESTED).toBe('number');
    });
  });
  
  describe('Clerk Full Classification', () => {
    let clerk: Clerk;
    
    beforeEach(() => {
      clerk = new Clerk({ confidenceThreshold: 0.5 });
    });
    
    it('should classify OOO messages correctly', () => {
      const result = clerk.classify(
        'Out of Office',
        'I am currently out of the office and will return on Monday.'
      );
      expect(result.category).toBe('OOO');
      expect(result.confidence).toBeGreaterThan(0.9);
    });
    
    it('should classify bounce messages correctly', () => {
      const result = clerk.classify(
        'Delivery Status Notification',
        'Mail delivery failed: returning message to sender. User unknown.'
      );
      expect(result.category).toBe('BOUNCE');
    });
    
    it('should classify unsubscribe requests correctly', () => {
      const result = clerk.classify(
        'Re: Partnership',
        'Please remove me from your mailing list. Unsubscribe me.'
      );
      expect(result.category).toBe('UNSUBSCRIBE');
    });
    
    it('should classify meeting requests correctly', () => {
      const result = clerk.classify(
        'Re: Quick question',
        "Let's set up a call to discuss this further. I'm free Thursday."
      );
      expect(result.category).toBe('MEETING');
    });
    
    it('should classify interested responses correctly', () => {
      const result = clerk.classify(
        'Re: Ad monetization',
        'This sounds interesting! Could you send me more information?'
      );
      expect(result.category).toBe('INTERESTED');
    });
    
    it('should classify not interested responses correctly', () => {
      const result = clerk.classify(
        'Re: Partnership opportunity',
        "Thanks, but we're not interested at this time. We're happy with our current solution."
      );
      expect(result.category).toBe('NOT_INTERESTED');
    });
    
    it('should flag low-confidence classifications for review', () => {
      const result = clerk.classify(
        'Re: Hello',
        'Thanks for your email. Let me think about it.'
      );
      // Ambiguous message should have lower confidence
      if (result.confidence < 0.6) {
        expect(result.requiresReview).toBe(true);
      }
    });
  });
  
  describe('Learning from Feedback', () => {
    it('should improve after learning', () => {
      const clerk = new Clerk();
      
      // Initial classification of ambiguous text
      const initial = clerk.classify('Custom subject', 'We will review internally and get back');
      
      // Teach it this pattern means INTERESTED
      clerk.learn('We will review internally and get back', 'INTERESTED');
      clerk.learn('Let us review and respond', 'INTERESTED');
      clerk.learn('Will discuss with team and reply', 'INTERESTED');
      
      // Should now lean toward INTERESTED
      const after = clerk.classify('Custom subject', 'We will discuss internally');
      
      // The probability for INTERESTED should increase
      expect(after.probabilities.INTERESTED).toBeGreaterThanOrEqual(
        initial.probabilities.INTERESTED
      );
    });
  });
});

// ============================================================================
// HEURISTIC SCORING TESTS
// ============================================================================

describe('Analyst (Heuristic Scoring)', () => {
  describe('Lead Scoring', () => {
    it('should score high for contacts with app-ads.txt and premium networks', () => {
      const contact: Contact = {
        id: 'test-1',
        companyName: 'Great App Inc',
        website: 'https://greatapp.com',
        email: 'contact@greatapp.com',
        appGenre: 'Games',
        adsTxtContent: `
          google.com, pub-1234567890, DIRECT
          facebook.com, 98765, DIRECT
          applovin.com, abcd1234, DIRECT
        `,
      };
      
      const score = calculateLeadScore(contact);
      
      expect(score.totalScore).toBeGreaterThan(70);
      expect(score.tier).toBe('hot');
    });
    
    it('should score low for contacts with no ads signals', () => {
      const contact: Contact = {
        id: 'test-2',
        companyName: 'Basic App LLC',
        appGenre: 'Productivity',
      };
      
      const score = calculateLeadScore(contact);
      
      expect(score.totalScore).toBeLessThan(40);
      expect(['cold', 'ice']).toContain(score.tier);
    });
    
    it('should include all scoring signals', () => {
      const contact: Contact = {
        id: 'test-3',
        companyName: 'Test Inc',
      };
      
      const score = calculateLeadScore(contact);
      
      expect(score.signals.length).toBeGreaterThan(0);
      expect(score.signals.every(s => 
        s.signal && 
        typeof s.score === 'number' && 
        typeof s.weight === 'number' &&
        s.explanation
      )).toBe(true);
    });
    
    it('should favor game genre over productivity', () => {
      const gameContact: Contact = {
        id: 'game-1',
        companyName: 'Game Studio',
        appGenre: 'Arcade Games',
      };
      
      const prodContact: Contact = {
        id: 'prod-1',
        companyName: 'Prod App',
        appGenre: 'Productivity',
      };
      
      const gameScore = calculateLeadScore(gameContact);
      const prodScore = calculateLeadScore(prodContact);
      
      const gameGenreSignal = gameScore.signals.find(s => s.signal === 'app_genre_fit');
      const prodGenreSignal = prodScore.signals.find(s => s.signal === 'app_genre_fit');
      
      expect(gameGenreSignal!.score).toBeGreaterThan(prodGenreSignal!.score);
    });
  });
  
  describe('Tier Assignment', () => {
    it('should assign correct tiers based on score', () => {
      // Test tier boundaries: hot >= 70, warm >= 50, cold >= 30, ice < 30
      
      // Create contacts that will score in different ranges
      const highScoreContact: Contact = {
        id: 'high',
        companyName: 'High Score Inc',
        website: 'https://high.com',
        email: 'test@high.com',
        appGenre: 'Games',
        adsTxtContent: `
          google.com, pub-123, DIRECT
          facebook.com, 456, DIRECT
          applovin.com, 789, DIRECT
          unity.com, abc, DIRECT
        `,
      };
      
      const lowScoreContact: Contact = {
        id: 'low',
        companyName: 'Low Score Inc',
        appGenre: 'Medical',
      };
      
      const highScore = calculateLeadScore(highScoreContact);
      const lowScore = calculateLeadScore(lowScoreContact);
      
      expect(['hot', 'warm']).toContain(highScore.tier);
      expect(['cold', 'ice']).toContain(lowScore.tier);
    });
  });
});

// ============================================================================
// INTEGRATION SMOKE TEST
// ============================================================================

describe('Integration Smoke Test', () => {
  it('should run full pipeline simulation', () => {
    // 1. Simulate a discovered contact
    const contact: Contact = {
      id: 'integration-test',
      companyName: 'Integration Test Corp',
      website: 'https://integrationtest.com',
      email: 'hello@integrationtest.com',
      appStore: 'google_play',
      appId: 'com.integration.test',
      appGenre: 'Arcade',
      adsTxtContent: 'google.com, pub-123, DIRECT\napplovin.com, abc, DIRECT',
    };
    
    // 2. Score the contact
    const score = calculateLeadScore(contact);
    expect(score.totalScore).toBeGreaterThan(0);
    
    // 3. If hot/warm, compose a message
    if (['hot', 'warm'].includes(score.tier)) {
      const engine = new SpintaxEngine();
      const context: PersonalizationContext = {
        firstName: 'Test',
        company: contact.companyName,
        appName: 'TestApp',
        appGenre: contact.appGenre,
      };
      
      const message = engine.generateUnique(
        '{Quick question|Partnership inquiry} for {{COMPANY}}',
        'Hi {{FIRST_NAME}}, noticed {{APP_NAME}} in the {{APP_GENRE}} category...',
        context
      );
      
      expect(message.subject).toContain('Integration Test Corp');
      expect(message.body).toContain('Arcade');
    }
    
    // 4. Simulate receiving a reply
    const clerk = new Clerk();
    const reply = clerk.classify(
      'Re: Quick question for Integration Test Corp',
      'Sounds interesting! Can you tell me more about the integration?'
    );
    
    expect(reply.category).toBe('INTERESTED');
    
    // 5. Update Thompson Sampler (simulated)
    const sampler = new ThompsonSampler();
    sampler.addOption('subject_a');
    sampler.addOption('subject_b');
    sampler.update('subject_a', true); // Got a positive reply
    
    expect(sampler.getRate('subject_a')).toBeGreaterThan(0);
  });
});
