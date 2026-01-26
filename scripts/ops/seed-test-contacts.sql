-- SEED: Realistic Test Contacts for Email Compose Testing
-- These are synthetic contacts based on real company patterns

-- Clear existing test contacts (preserve only production data)
DELETE FROM sa_contacts WHERE email LIKE '%@test.local' OR email LIKE '%@example.%';

-- Insert diverse test contacts with realistic data
INSERT INTO sa_contacts (
  email, first_name, last_name, company_name, job_title,
  app_name, app_genre, app_downloads, source, status, score,
  icebreaker, app_description, website
) VALUES

-- 1. Gaming - Puzzle/Casual
('sarah.chen@puzzlemaster.test.local', 'Sarah', 'Chen', 'PuzzleMaster Studios',
 'Head of Monetization', 'Block Blast Adventure', 'Puzzle', '10M+', 'itunes_scrape',
 'new', 75, 'Block Blast Adventure hit #3 on the US puzzle charts last week!',
 'A relaxing yet challenging block puzzle game with over 5000 levels and daily challenges.',
 'puzzlemasterstudios.com'),

-- 2. Gaming - Hypercasual
('mike.johnson@tapgamesinc.test.local', 'Mike', 'Johnson', 'TapGames Inc',
 'VP Product', 'Helix Stack Ball', 'Hypercasual', '50M+', 'google_play_scrape',
 'new', 82, 'Helix Stack Ball was featured in Google Play Best of 2024!',
 'Addictive one-tap gameplay where you guide a ball through a helix tower.',
 'tapgamesinc.com'),

-- 3. Gaming - Word/Trivia
('emma.wilson@wordplaystudio.test.local', 'Emma', 'Wilson', 'WordPlay Studio',
 'CEO', 'Word Connect Daily', 'Word', '5M+', 'itunes_scrape',
 'new', 68, 'Word Connect Daily has a 4.8 star rating with 500K+ reviews.',
 'Daily word puzzles that challenge your vocabulary and keep your mind sharp.',
 'wordplaystudio.io'),

-- 4. Gaming - Match 3
('carlos.rodriguez@candycrushclone.test.local', 'Carlos', 'Rodriguez', 'Sweet Games LLC',
 'Director of Growth', 'Candy Match Kingdom', 'Match 3', '25M+', 'itunes_scrape',
 'new', 79, 'Candy Match Kingdom just launched Season 15 with 200 new levels.',
 'Match colorful candies in this sweet puzzle adventure with thousands of levels.',
 'sweetgamesllc.com'),

-- 5. Gaming - Idle/Tycoon
('anna.kowalski@idletycoon.test.local', 'Anna', 'Kowalski', 'Idle Empire Games',
 'Monetization Manager', 'Coffee Shop Tycoon', 'Simulation', '8M+', 'google_play_scrape',
 'new', 71, 'Coffee Shop Tycoon reached $2M in IAP revenue last quarter.',
 'Build your coffee empire from a small cart to a global franchise.',
 'idleempiregames.com'),

-- 6. Gaming - Card/Casino
('david.park@cardgamespro.test.local', 'David', 'Park', 'CardGames Pro',
 'Head of Ad Ops', 'Solitaire Grand Harvest', 'Card', '15M+', 'itunes_scrape',
 'new', 85, 'Solitaire Grand Harvest users spend an average of 45 min/day in-app.',
 'Classic solitaire with a farming twist - grow crops as you win hands.',
 'cardgamespro.net'),

-- 7. Gaming - Racing
('tom.miller@speedracegames.test.local', 'Tom', 'Miller', 'SpeedRace Games',
 'Studio Head', 'Drift Kings Racing', 'Racing', '20M+', 'google_play_scrape',
 'new', 77, 'Drift Kings Racing won Mobile Game of the Year at GDC 2024.',
 'High-octane drift racing with stunning graphics and multiplayer mode.',
 'speedracegames.io'),

-- 8. Utility - Weather
('lisa.brown@weathernow.test.local', 'Lisa', 'Brown', 'WeatherNow Inc',
 'Product Manager', 'WeatherNow - Hourly Forecast', 'Weather', '12M+', 'itunes_scrape',
 'new', 62, 'WeatherNow has partnerships with The Weather Channel and AccuWeather.',
 'Accurate hourly weather forecasts with beautiful widgets and radar maps.',
 'weathernowinc.com'),

-- 9. Lifestyle - Fitness
('james.taylor@fitlifepro.test.local', 'James', 'Taylor', 'FitLife Pro',
 'Chief Revenue Officer', 'Home Workout Pro', 'Health & Fitness', '7M+', 'google_play_scrape',
 'new', 64, 'Home Workout Pro has been featured on Apple Fitness+.',
 'No equipment needed - get fit at home with guided video workouts.',
 'fitlifepro.app'),

-- 10. Gaming - Strategy
('nina.garcia@strategykings.test.local', 'Nina', 'Garcia', 'Strategy Kings Studio',
 'General Manager', 'Kingdom Defense TD', 'Strategy', '3M+', 'itunes_scrape',
 'new', 73, 'Kingdom Defense TD has an ARPDAU of $0.28, well above category average.',
 'Epic tower defense with RPG elements - defend your kingdom from waves of enemies.',
 'strategykings.com'),

-- 11. Gaming - Kids
('robert.lee@kidsfunapp.test.local', 'Robert', 'Lee', 'KidsFun Apps',
 'Founder', 'ABCs Learning Adventure', 'Education', '4M+', 'itunes_scrape',
 'new', 58, 'ABCs Learning Adventure is used by 10,000+ preschools worldwide.',
 'Fun educational games that teach letters, numbers, and basic reading.',
 'kidsfunapps.com'),

-- 12. Gaming - Merge
('jennifer.white@mergegames.test.local', 'Jennifer', 'White', 'MergeGames Studio',
 'VP Monetization', 'Merge Dragons Paradise', 'Puzzle', '18M+', 'google_play_scrape',
 'new', 81, 'Merge Dragons Paradise D7 retention is 35%, best in category.',
 'Discover the magic of merging - combine items to create new wonders.',
 'mergegamesstudio.com'),

-- 13. Gaming - Sports
('kevin.smith@sportsgamesinc.test.local', 'Kevin', 'Smith', 'SportsGames Inc',
 'Head of Publishing', 'Basketball Stars Showdown', 'Sports', '6M+', 'itunes_scrape',
 'new', 70, 'Basketball Stars has official NBA player likenesses.',
 '1v1 basketball action with realistic physics and online multiplayer.',
 'sportsgamesinc.net'),

-- 14. Utility - Photo
('amanda.davis@photoeditorpro.test.local', 'Amanda', 'Davis', 'PhotoEditor Pro',
 'Marketing Director', 'Photo Magic Editor', 'Photo & Video', '9M+', 'google_play_scrape',
 'new', 66, 'Photo Magic Editor AI features are driving 40% of premium conversions.',
 'Professional photo editing with AI-powered enhancements and filters.',
 'photoeditorpro.io'),

-- 15. Gaming - Simulation
('chris.thomas@farmgamesllc.test.local', 'Chris', 'Thomas', 'Farm Games LLC',
 'Director of Revenue', 'Harvest Valley Farm', 'Simulation', '11M+', 'itunes_scrape',
 'new', 74, 'Harvest Valley Farm has 2.1M DAU with 65% D30 retention.',
 'Build your dream farm, raise animals, and trade with neighbors.',
 'farmgamesllc.com'),

-- 16. DE - German Gaming Company
('markus.schneider@spielewelt.test.local', 'Markus', 'Schneider', 'SpielWelt GmbH',
 'Geschäftsführer', 'Wortspiel Meister', 'Word', '2M+', 'itunes_scrape',
 'new', 69, 'Wortspiel Meister is #1 word game in Germany, Austria, and Switzerland.',
 'Das beliebte deutsche Wortspiel mit täglichen Rätseln.',
 'spielewelt.de'),

-- 17. FR - French Gaming Company
('marie.dubois@jeuxmobile.test.local', 'Marie', 'Dubois', 'JeuxMobile SAS',
 'Directrice Commerciale', 'Mots Croisés Français', 'Word', '3M+', 'itunes_scrape',
 'new', 67, 'Mots Croisés Français won Best French App at Paris Games Week.',
 'Les meilleurs mots croisés en français avec des grilles quotidiennes.',
 'jeuxmobile.fr'),

-- 18. JP - Japanese Gaming Company
('yuki.tanaka@mobilegamejp.test.local', 'Yuki', 'Tanaka', 'MobileGame JP',
 'Marketing Manager', 'Puzzle Quest Japan', 'Puzzle', '8M+', 'google_play_scrape',
 'new', 76, 'Puzzle Quest Japan collaborated with popular anime franchises.',
 '人気アニメとコラボしたパズルRPG。',
 'mobilegamejp.co.jp'),

-- 19. BR - Brazilian Gaming Company
('lucas.silva@jogosbrasil.test.local', 'Lucas', 'Silva', 'JogosBrasil Ltda',
 'Gerente de Produto', 'Palavra Cruzada BR', 'Word', '4M+', 'google_play_scrape',
 'new', 63, 'Palavra Cruzada BR is top 10 in Brazil for 52 consecutive weeks.',
 'O jogo de palavras cruzadas mais popular do Brasil.',
 'jogosbrasil.com.br'),

-- 20. SA - Saudi Gaming Company
('ahmed.al-rashid@arabgames.test.local', 'Ahmed', 'Al-Rashid', 'Arab Games Studio',
 'CEO', 'Desert Racing Champions', 'Racing', '5M+', 'google_play_scrape',
 'new', 72, 'Desert Racing Champions is the #1 racing game in GCC countries.',
 'سباق صحراوي مثير مع سيارات ودراجات نارية حقيقية.',
 'arabgamesstudio.com');

SELECT 'Inserted 20 realistic test contacts' as status;
