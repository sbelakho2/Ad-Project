-- Deep EU and MENA Market Sources for Lead Discovery
-- Comprehensive coverage of European Union and Middle East/North Africa regions

-- ============================================
-- EUROPEAN UNION - WESTERN EUROPE
-- ============================================

-- Germany (DE) - Additional sources
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'DE - Hypercasual Spiele', 'hypercasual spiel', 'Games', 'de', 'active', 12),
('itunes', 'DE - Match 3 Spiele', 'match 3 puzzle spiel', 'Games', 'de', 'active', 12),
('itunes', 'DE - Idle Spiele', 'idle clicker spiel', 'Games', 'de', 'active', 12),
('itunes', 'DE - Quiz Spiele', 'quiz trivia wissen', 'Games', 'de', 'active', 12),
('itunes', 'DE - Bubble Shooter', 'bubble shooter', 'Games', 'de', 'active', 12),
('google_play', 'GP DE - Casual Spiele', 'casual spiel kostenlos', 'Games', 'de', 'active', 12),
('google_play', 'GP DE - Puzzle Spiele', 'puzzle gehirn training', 'Games', 'de', 'active', 12),
('google_play', 'GP DE - Kartenspiele', 'karten solitaer', 'Games', 'de', 'active', 12);

-- France (FR) - Additional sources
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'FR - Hypercasual Jeux', 'hypercasual jeu', 'Games', 'fr', 'active', 12),
('itunes', 'FR - Match 3 Jeux', 'match 3 puzzle jeu', 'Games', 'fr', 'active', 12),
('itunes', 'FR - Idle Jeux', 'idle clicker jeu', 'Games', 'fr', 'active', 12),
('itunes', 'FR - Quiz Jeux', 'quiz trivia culture', 'Games', 'fr', 'active', 12),
('itunes', 'FR - Bubble Shooter', 'bubble shooter', 'Games', 'fr', 'active', 12),
('google_play', 'GP FR - Jeux Casual', 'casual jeu gratuit', 'Games', 'fr', 'active', 12),
('google_play', 'GP FR - Jeux Puzzle', 'puzzle cerveau', 'Games', 'fr', 'active', 12),
('google_play', 'GP FR - Jeux Cartes', 'jeux cartes solitaire', 'Games', 'fr', 'active', 12);

-- Italy (IT) - Additional sources
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'IT - Hypercasual Giochi', 'hypercasual gioco', 'Games', 'it', 'active', 12),
('itunes', 'IT - Match 3 Giochi', 'match 3 puzzle gioco', 'Games', 'it', 'active', 12),
('itunes', 'IT - Idle Giochi', 'idle clicker gioco', 'Games', 'it', 'active', 12),
('itunes', 'IT - Quiz Giochi', 'quiz trivia cultura', 'Games', 'it', 'active', 12),
('itunes', 'IT - Giochi Carte', 'carte solitario', 'Games', 'it', 'active', 12),
('google_play', 'GP IT - Giochi Casual', 'casual gioco gratis', 'Games', 'it', 'active', 12),
('google_play', 'GP IT - Giochi Puzzle', 'puzzle mente', 'Games', 'it', 'active', 12);

-- Spain (ES) - Additional sources
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'ES - Hypercasual Juegos', 'hypercasual juego', 'Games', 'es', 'active', 12),
('itunes', 'ES - Match 3 Juegos', 'match 3 puzzle juego', 'Games', 'es', 'active', 12),
('itunes', 'ES - Idle Juegos', 'idle clicker juego', 'Games', 'es', 'active', 12),
('itunes', 'ES - Quiz Juegos', 'quiz trivia cultura', 'Games', 'es', 'active', 12),
('itunes', 'ES - Bubble Shooter', 'bubble shooter', 'Games', 'es', 'active', 12),
('google_play', 'GP ES - Juegos Casual', 'casual juego gratis', 'Games', 'es', 'active', 12),
('google_play', 'GP ES - Juegos Puzzle', 'puzzle cerebro', 'Games', 'es', 'active', 12);

-- Netherlands (NL) - Deep coverage
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'NL - Puzzle Spelletjes', 'puzzel spel gratis', 'Games', 'nl', 'active', 12),
('itunes', 'NL - Woordspellen', 'woord puzzel spel', 'Games', 'nl', 'active', 12),
('itunes', 'NL - Kaartspellen', 'kaart patience solitaire', 'Games', 'nl', 'active', 12),
('itunes', 'NL - Match 3 Spellen', 'match 3 puzzel', 'Games', 'nl', 'active', 12),
('itunes', 'NL - Quiz Spelletjes', 'quiz trivia kennis', 'Games', 'nl', 'active', 12),
('google_play', 'GP NL - Casual Spellen', 'casual spel gratis', 'Games', 'nl', 'active', 12),
('google_play', 'GP NL - Puzzel Spellen', 'puzzel hersenen', 'Games', 'nl', 'active', 12);

-- Belgium (BE)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'BE - Casual Games', 'casual game free', 'Games', 'be', 'active', 12),
('itunes', 'BE - Puzzle Spellen', 'puzzel spel', 'Games', 'be', 'active', 12),
('itunes', 'BE - Jeux Puzzle', 'puzzle jeu gratuit', 'Games', 'be', 'active', 12),
('itunes', 'BE - Woordspellen', 'woord puzzel', 'Games', 'be', 'active', 12),
('google_play', 'GP BE - Casual Games', 'casual game gratis', 'Games', 'be', 'active', 12);

-- Austria (AT)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'AT - Casual Spiele', 'casual spiel kostenlos', 'Games', 'at', 'active', 12),
('itunes', 'AT - Puzzle Spiele', 'puzzle raetsel', 'Games', 'at', 'active', 12),
('itunes', 'AT - Kartenspiele', 'karten solitaer', 'Games', 'at', 'active', 12),
('itunes', 'AT - Quiz Spiele', 'quiz wissen', 'Games', 'at', 'active', 12),
('google_play', 'GP AT - Casual Spiele', 'casual spiel gratis', 'Games', 'at', 'active', 12);

-- Switzerland (CH)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'CH - Casual Spiele', 'casual spiel kostenlos', 'Games', 'ch', 'active', 12),
('itunes', 'CH - Puzzle Spiele', 'puzzle raetsel', 'Games', 'ch', 'active', 12),
('itunes', 'CH - Jeux Puzzle', 'puzzle jeu gratuit', 'Games', 'ch', 'active', 12),
('itunes', 'CH - Giochi Puzzle', 'puzzle gioco gratis', 'Games', 'ch', 'active', 12),
('google_play', 'GP CH - Casual Games', 'casual game gratis', 'Games', 'ch', 'active', 12);

-- Portugal (PT)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'PT - Jogos Casual', 'casual jogo gratis', 'Games', 'pt', 'active', 12),
('itunes', 'PT - Jogos Puzzle', 'puzzle quebra cabeca', 'Games', 'pt', 'active', 12),
('itunes', 'PT - Jogos Palavras', 'palavras cruzadas', 'Games', 'pt', 'active', 12),
('itunes', 'PT - Jogos Cartas', 'cartas paciencia', 'Games', 'pt', 'active', 12),
('google_play', 'GP PT - Jogos Casual', 'casual jogo gratis', 'Games', 'pt', 'active', 12),
('google_play', 'GP PT - Jogos Puzzle', 'puzzle cerebro', 'Games', 'pt', 'active', 12);

-- Ireland (IE)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'IE - Casual Games', 'casual game free', 'Games', 'ie', 'active', 12),
('itunes', 'IE - Puzzle Games', 'puzzle brain game', 'Games', 'ie', 'active', 12),
('itunes', 'IE - Word Games', 'word crossword', 'Games', 'ie', 'active', 12),
('itunes', 'IE - Card Games', 'card solitaire', 'Games', 'ie', 'active', 12),
('google_play', 'GP IE - Casual Games', 'casual game free', 'Games', 'ie', 'active', 12);

-- ============================================
-- EUROPEAN UNION - NORDIC COUNTRIES
-- ============================================

-- Sweden (SE)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'SE - Casual Spel', 'casual spel gratis', 'Games', 'se', 'active', 12),
('itunes', 'SE - Pussel Spel', 'pussel hjarna', 'Games', 'se', 'active', 12),
('itunes', 'SE - Ordspel', 'ord korsord', 'Games', 'se', 'active', 12),
('itunes', 'SE - Kortspel', 'kort patiens', 'Games', 'se', 'active', 12),
('itunes', 'SE - Match 3 Spel', 'match 3 pussel', 'Games', 'se', 'active', 12),
('google_play', 'GP SE - Casual Spel', 'casual spel gratis', 'Games', 'se', 'active', 12);

-- Norway (NO)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'NO - Casual Spill', 'casual spill gratis', 'Games', 'no', 'active', 12),
('itunes', 'NO - Puslespill', 'puslespill hjerne', 'Games', 'no', 'active', 12),
('itunes', 'NO - Ordspill', 'ord kryssord', 'Games', 'no', 'active', 12),
('itunes', 'NO - Kortspill', 'kort kabal', 'Games', 'no', 'active', 12),
('google_play', 'GP NO - Casual Spill', 'casual spill gratis', 'Games', 'no', 'active', 12);

-- Denmark (DK)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'DK - Casual Spil', 'casual spil gratis', 'Games', 'dk', 'active', 12),
('itunes', 'DK - Puslespil', 'puslespil hjerne', 'Games', 'dk', 'active', 12),
('itunes', 'DK - Ordspil', 'ord krydsord', 'Games', 'dk', 'active', 12),
('itunes', 'DK - Kortspil', 'kort kabale', 'Games', 'dk', 'active', 12),
('google_play', 'GP DK - Casual Spil', 'casual spil gratis', 'Games', 'dk', 'active', 12);

-- Finland (FI)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'FI - Casual Pelit', 'casual peli ilmainen', 'Games', 'fi', 'active', 12),
('itunes', 'FI - Palapeli', 'palapeli aivot', 'Games', 'fi', 'active', 12),
('itunes', 'FI - Sanapelit', 'sana ristikko', 'Games', 'fi', 'active', 12),
('itunes', 'FI - Korttipelit', 'kortti pasianssi', 'Games', 'fi', 'active', 12),
('google_play', 'GP FI - Casual Pelit', 'casual peli ilmainen', 'Games', 'fi', 'active', 12);

-- ============================================
-- EUROPEAN UNION - EASTERN EUROPE
-- ============================================

-- Poland (PL) - Deep coverage
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'PL - Gry Puzzle', 'puzzle lamiglowka', 'Games', 'pl', 'active', 12),
('itunes', 'PL - Gry Slowne', 'slowa krzyzowka', 'Games', 'pl', 'active', 12),
('itunes', 'PL - Gry Karciane', 'karty pasjans', 'Games', 'pl', 'active', 12),
('itunes', 'PL - Match 3 Gry', 'match 3 puzzle', 'Games', 'pl', 'active', 12),
('itunes', 'PL - Gry Quiz', 'quiz wiedza', 'Games', 'pl', 'active', 12),
('google_play', 'GP PL - Gry Casual', 'casual gra za darmo', 'Games', 'pl', 'active', 12),
('google_play', 'GP PL - Gry Puzzle', 'puzzle mozg', 'Games', 'pl', 'active', 12);

-- Czech Republic (CZ)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'CZ - Casual Hry', 'casual hra zdarma', 'Games', 'cz', 'active', 12),
('itunes', 'CZ - Puzzle Hry', 'puzzle hlavolam', 'Games', 'cz', 'active', 12),
('itunes', 'CZ - Slovni Hry', 'slova krizovka', 'Games', 'cz', 'active', 12),
('itunes', 'CZ - Karetni Hry', 'karty solitaire', 'Games', 'cz', 'active', 12),
('google_play', 'GP CZ - Casual Hry', 'casual hra zdarma', 'Games', 'cz', 'active', 12);

-- Romania (RO)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'RO - Jocuri Casual', 'casual joc gratis', 'Games', 'ro', 'active', 12),
('itunes', 'RO - Jocuri Puzzle', 'puzzle creier', 'Games', 'ro', 'active', 12),
('itunes', 'RO - Jocuri Cuvinte', 'cuvinte incrucisate', 'Games', 'ro', 'active', 12),
('itunes', 'RO - Jocuri Carti', 'carti solitaire', 'Games', 'ro', 'active', 12),
('google_play', 'GP RO - Jocuri Casual', 'casual joc gratis', 'Games', 'ro', 'active', 12);

-- Hungary (HU)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'HU - Casual Jatekok', 'casual jatek ingyenes', 'Games', 'hu', 'active', 12),
('itunes', 'HU - Puzzle Jatekok', 'puzzle agy', 'Games', 'hu', 'active', 12),
('itunes', 'HU - Szo Jatekok', 'szo keresztrejtveny', 'Games', 'hu', 'active', 12),
('itunes', 'HU - Kartya Jatekok', 'kartya pasziansz', 'Games', 'hu', 'active', 12),
('google_play', 'GP HU - Casual Jatekok', 'casual jatek ingyenes', 'Games', 'hu', 'active', 12);

-- Greece (GR)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'GR - Casual Games', 'casual paixnidi dorean', 'Games', 'gr', 'active', 12),
('itunes', 'GR - Puzzle Games', 'puzzle egkefalo', 'Games', 'gr', 'active', 12),
('itunes', 'GR - Word Games', 'lexeis staurolexa', 'Games', 'gr', 'active', 12),
('itunes', 'GR - Card Games', 'trapes pasientza', 'Games', 'gr', 'active', 12),
('google_play', 'GP GR - Casual Games', 'casual paixnidi dorean', 'Games', 'gr', 'active', 12);

-- Slovakia (SK)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'SK - Casual Hry', 'casual hra zadarmo', 'Games', 'sk', 'active', 12),
('itunes', 'SK - Puzzle Hry', 'puzzle hlavolam', 'Games', 'sk', 'active', 12),
('itunes', 'SK - Slovne Hry', 'slova krizovka', 'Games', 'sk', 'active', 12),
('google_play', 'GP SK - Casual Hry', 'casual hra zadarmo', 'Games', 'sk', 'active', 12);

-- Bulgaria (BG)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'BG - Casual Games', 'casual igra bezplatno', 'Games', 'bg', 'active', 12),
('itunes', 'BG - Puzzle Games', 'puzzle mozak', 'Games', 'bg', 'active', 12),
('itunes', 'BG - Word Games', 'dumi krastoslovitsa', 'Games', 'bg', 'active', 12),
('google_play', 'GP BG - Casual Games', 'casual igra bezplatno', 'Games', 'bg', 'active', 12);

-- Croatia (HR)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'HR - Casual Igre', 'casual igra besplatno', 'Games', 'hr', 'active', 12),
('itunes', 'HR - Puzzle Igre', 'puzzle mozak', 'Games', 'hr', 'active', 12),
('itunes', 'HR - Igre Rijeci', 'rijeci krizaljka', 'Games', 'hr', 'active', 12),
('google_play', 'GP HR - Casual Igre', 'casual igra besplatno', 'Games', 'hr', 'active', 12);

-- Slovenia (SI)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'SI - Casual Igre', 'casual igra brezplacno', 'Games', 'si', 'active', 12),
('itunes', 'SI - Puzzle Igre', 'puzzle možgani', 'Games', 'si', 'active', 12),
('google_play', 'GP SI - Casual Igre', 'casual igra brezplacno', 'Games', 'si', 'active', 12);

-- Estonia (EE)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'EE - Casual Mangud', 'casual mang tasuta', 'Games', 'ee', 'active', 12),
('itunes', 'EE - Puzzle Mangud', 'puzzle aju', 'Games', 'ee', 'active', 12),
('google_play', 'GP EE - Casual Mangud', 'casual mang tasuta', 'Games', 'ee', 'active', 12);

-- Latvia (LV)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'LV - Casual Speles', 'casual spele bezmaksas', 'Games', 'lv', 'active', 12),
('itunes', 'LV - Puzzle Speles', 'puzzle smadzenes', 'Games', 'lv', 'active', 12),
('google_play', 'GP LV - Casual Speles', 'casual spele bezmaksas', 'Games', 'lv', 'active', 12);

-- Lithuania (LT)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'LT - Casual Zaidimai', 'casual zaidimas nemokamas', 'Games', 'lt', 'active', 12),
('itunes', 'LT - Puzzle Zaidimai', 'puzzle smegenys', 'Games', 'lt', 'active', 12),
('google_play', 'GP LT - Casual Zaidimai', 'casual zaidimas nemokamas', 'Games', 'lt', 'active', 12);

-- ============================================
-- MENA - GULF COOPERATION COUNCIL (GCC)
-- ============================================

-- Saudi Arabia (SA)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'SA - Casual Games', 'casual game free', 'Games', 'sa', 'active', 12),
('itunes', 'SA - Puzzle Games', 'puzzle brain', 'Games', 'sa', 'active', 12),
('itunes', 'SA - العاب كاجوال', 'العاب مجانية كاجوال', 'Games', 'sa', 'active', 12),
('itunes', 'SA - العاب الغاز', 'الغاز ذكاء', 'Games', 'sa', 'active', 12),
('itunes', 'SA - العاب كلمات', 'كلمات متقاطعة', 'Games', 'sa', 'active', 12),
('itunes', 'SA - العاب ورق', 'ورق سوليتير', 'Games', 'sa', 'active', 12),
('google_play', 'GP SA - Casual Games', 'casual game free', 'Games', 'sa', 'active', 12),
('google_play', 'GP SA - العاب كاجوال', 'العاب مجانية', 'Games', 'sa', 'active', 12);

-- United Arab Emirates (AE)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'AE - Casual Games', 'casual game free', 'Games', 'ae', 'active', 12),
('itunes', 'AE - Puzzle Games', 'puzzle brain', 'Games', 'ae', 'active', 12),
('itunes', 'AE - العاب كاجوال', 'العاب مجانية كاجوال', 'Games', 'ae', 'active', 12),
('itunes', 'AE - العاب الغاز', 'الغاز ذكاء تفكير', 'Games', 'ae', 'active', 12),
('itunes', 'AE - العاب كلمات', 'كلمات متقاطعة عربي', 'Games', 'ae', 'active', 12),
('google_play', 'GP AE - Casual Games', 'casual game free', 'Games', 'ae', 'active', 12),
('google_play', 'GP AE - العاب كاجوال', 'العاب مجانية', 'Games', 'ae', 'active', 12);

-- Qatar (QA)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'QA - Casual Games', 'casual game free', 'Games', 'qa', 'active', 12),
('itunes', 'QA - Puzzle Games', 'puzzle brain', 'Games', 'qa', 'active', 12),
('itunes', 'QA - العاب كاجوال', 'العاب مجانية', 'Games', 'qa', 'active', 12),
('itunes', 'QA - العاب الغاز', 'الغاز ذكاء', 'Games', 'qa', 'active', 12),
('google_play', 'GP QA - Casual Games', 'casual game free', 'Games', 'qa', 'active', 12);

-- Kuwait (KW)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'KW - Casual Games', 'casual game free', 'Games', 'kw', 'active', 12),
('itunes', 'KW - Puzzle Games', 'puzzle brain', 'Games', 'kw', 'active', 12),
('itunes', 'KW - العاب كاجوال', 'العاب مجانية', 'Games', 'kw', 'active', 12),
('itunes', 'KW - العاب الغاز', 'الغاز ذكاء', 'Games', 'kw', 'active', 12),
('google_play', 'GP KW - Casual Games', 'casual game free', 'Games', 'kw', 'active', 12);

-- Bahrain (BH)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'BH - Casual Games', 'casual game free', 'Games', 'bh', 'active', 12),
('itunes', 'BH - Puzzle Games', 'puzzle brain', 'Games', 'bh', 'active', 12),
('itunes', 'BH - العاب كاجوال', 'العاب مجانية', 'Games', 'bh', 'active', 12),
('google_play', 'GP BH - Casual Games', 'casual game free', 'Games', 'bh', 'active', 12);

-- Oman (OM)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'OM - Casual Games', 'casual game free', 'Games', 'om', 'active', 12),
('itunes', 'OM - Puzzle Games', 'puzzle brain', 'Games', 'om', 'active', 12),
('itunes', 'OM - العاب كاجوال', 'العاب مجانية', 'Games', 'om', 'active', 12),
('google_play', 'GP OM - Casual Games', 'casual game free', 'Games', 'om', 'active', 12);

-- ============================================
-- MENA - LEVANT & FERTILE CRESCENT
-- ============================================

-- Turkey (TR) - Deep coverage
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'TR - Puzzle Oyunlar', 'bulmaca oyun', 'Games', 'tr', 'active', 12),
('itunes', 'TR - Kelime Oyunlar', 'kelime bulmaca', 'Games', 'tr', 'active', 12),
('itunes', 'TR - Kart Oyunlar', 'kart solitaire', 'Games', 'tr', 'active', 12),
('itunes', 'TR - Match 3 Oyunlar', 'eslesme 3 puzzle', 'Games', 'tr', 'active', 12),
('itunes', 'TR - Quiz Oyunlar', 'bilgi yarismasi', 'Games', 'tr', 'active', 12),
('google_play', 'GP TR - Casual Oyunlar', 'casual oyun ucretsiz', 'Games', 'tr', 'active', 12),
('google_play', 'GP TR - Puzzle Oyunlar', 'bulmaca beyin', 'Games', 'tr', 'active', 12);

-- Israel (IL)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'IL - Casual Games', 'casual game free', 'Games', 'il', 'active', 12),
('itunes', 'IL - Puzzle Games', 'puzzle brain', 'Games', 'il', 'active', 12),
('itunes', 'IL - משחקי מילים', 'משחק מילים תשבץ', 'Games', 'il', 'active', 12),
('itunes', 'IL - משחקי חשיבה', 'חידות חשיבה', 'Games', 'il', 'active', 12),
('google_play', 'GP IL - Casual Games', 'casual game free', 'Games', 'il', 'active', 12);

-- Jordan (JO)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'JO - Casual Games', 'casual game free', 'Games', 'jo', 'active', 12),
('itunes', 'JO - Puzzle Games', 'puzzle brain', 'Games', 'jo', 'active', 12),
('itunes', 'JO - العاب كاجوال', 'العاب مجانية', 'Games', 'jo', 'active', 12),
('itunes', 'JO - العاب الغاز', 'الغاز ذكاء', 'Games', 'jo', 'active', 12),
('google_play', 'GP JO - Casual Games', 'casual game free', 'Games', 'jo', 'active', 12);

-- Lebanon (LB)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'LB - Casual Games', 'casual game free', 'Games', 'lb', 'active', 12),
('itunes', 'LB - Puzzle Games', 'puzzle brain', 'Games', 'lb', 'active', 12),
('itunes', 'LB - العاب كاجوال', 'العاب مجانية', 'Games', 'lb', 'active', 12),
('google_play', 'GP LB - Casual Games', 'casual game free', 'Games', 'lb', 'active', 12);

-- Iraq (IQ)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'IQ - Casual Games', 'casual game free', 'Games', 'iq', 'active', 12),
('itunes', 'IQ - العاب كاجوال', 'العاب مجانية', 'Games', 'iq', 'active', 12),
('itunes', 'IQ - العاب الغاز', 'الغاز ذكاء', 'Games', 'iq', 'active', 12),
('google_play', 'GP IQ - Casual Games', 'casual game free', 'Games', 'iq', 'active', 12);

-- ============================================
-- MENA - NORTH AFRICA
-- ============================================

-- Egypt (EG)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'EG - Casual Games', 'casual game free', 'Games', 'eg', 'active', 12),
('itunes', 'EG - Puzzle Games', 'puzzle brain', 'Games', 'eg', 'active', 12),
('itunes', 'EG - العاب كاجوال', 'العاب مجانية كاجوال', 'Games', 'eg', 'active', 12),
('itunes', 'EG - العاب الغاز', 'الغاز ذكاء تفكير', 'Games', 'eg', 'active', 12),
('itunes', 'EG - العاب كلمات', 'كلمات متقاطعة', 'Games', 'eg', 'active', 12),
('itunes', 'EG - العاب ورق', 'ورق كوتشينة', 'Games', 'eg', 'active', 12),
('google_play', 'GP EG - Casual Games', 'casual game free', 'Games', 'eg', 'active', 12),
('google_play', 'GP EG - العاب كاجوال', 'العاب مجانية', 'Games', 'eg', 'active', 12);

-- Morocco (MA)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'MA - Jeux Casual', 'casual jeu gratuit', 'Games', 'ma', 'active', 12),
('itunes', 'MA - Jeux Puzzle', 'puzzle cerveau', 'Games', 'ma', 'active', 12),
('itunes', 'MA - العاب كاجوال', 'العاب مجانية', 'Games', 'ma', 'active', 12),
('itunes', 'MA - العاب الغاز', 'الغاز ذكاء', 'Games', 'ma', 'active', 12),
('google_play', 'GP MA - Jeux Casual', 'casual jeu gratuit', 'Games', 'ma', 'active', 12),
('google_play', 'GP MA - العاب كاجوال', 'العاب مجانية', 'Games', 'ma', 'active', 12);

-- Algeria (DZ)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'DZ - Jeux Casual', 'casual jeu gratuit', 'Games', 'dz', 'active', 12),
('itunes', 'DZ - Jeux Puzzle', 'puzzle cerveau', 'Games', 'dz', 'active', 12),
('itunes', 'DZ - العاب كاجوال', 'العاب مجانية', 'Games', 'dz', 'active', 12),
('google_play', 'GP DZ - Jeux Casual', 'casual jeu gratuit', 'Games', 'dz', 'active', 12);

-- Tunisia (TN)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'TN - Jeux Casual', 'casual jeu gratuit', 'Games', 'tn', 'active', 12),
('itunes', 'TN - Jeux Puzzle', 'puzzle cerveau', 'Games', 'tn', 'active', 12),
('itunes', 'TN - العاب كاجوال', 'العاب مجانية', 'Games', 'tn', 'active', 12),
('google_play', 'GP TN - Jeux Casual', 'casual jeu gratuit', 'Games', 'tn', 'active', 12);

-- Libya (LY)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'LY - Casual Games', 'casual game free', 'Games', 'ly', 'active', 12),
('itunes', 'LY - العاب كاجوال', 'العاب مجانية', 'Games', 'ly', 'active', 12),
('google_play', 'GP LY - Casual Games', 'casual game free', 'Games', 'ly', 'active', 12);

-- Sudan (SD)
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'SD - Casual Games', 'casual game free', 'Games', 'sd', 'active', 12),
('itunes', 'SD - العاب كاجوال', 'العاب مجانية', 'Games', 'sd', 'active', 12),
('google_play', 'GP SD - Casual Games', 'casual game free', 'Games', 'sd', 'active', 12);

-- ============================================
-- BONUS: POPULAR GAME GENRES IN EU/MENA
-- ============================================

-- Localized Match-3 Games
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'EU - Match 3 Gems', 'match 3 jewels gems', 'Games', 'de', 'active', 12),
('itunes', 'EU - Match 3 Candy', 'match 3 candy sweet', 'Games', 'fr', 'active', 12),
('itunes', 'MENA - Match 3 Arabic', 'العاب match 3', 'Games', 'sa', 'active', 12);

-- Idle/Tycoon Games Popular in Region
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'EU - Idle Tycoon DE', 'idle tycoon manager', 'Games', 'de', 'active', 12),
('itunes', 'EU - Idle Tycoon FR', 'idle tycoon manager', 'Games', 'fr', 'active', 12),
('itunes', 'MENA - Idle Tycoon', 'idle tycoon manager', 'Games', 'ae', 'active', 12);

-- Bubble Shooter Regional
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('google_play', 'GP EU - Bubble Shooter DE', 'bubble shooter pop', 'Games', 'de', 'active', 12),
('google_play', 'GP EU - Bubble Shooter FR', 'bubble shooter pop', 'Games', 'fr', 'active', 12),
('google_play', 'GP MENA - Bubble Shooter', 'bubble shooter', 'Games', 'sa', 'active', 12);

-- Solitaire Regional
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'EU - Solitaire DE', 'solitaer patience karten', 'Games', 'de', 'active', 12),
('itunes', 'EU - Solitaire FR', 'solitaire patience carte', 'Games', 'fr', 'active', 12),
('itunes', 'EU - Solitaire NL', 'patience kaart spel', 'Games', 'nl', 'active', 12);

-- Word Games Regional
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'EU - Word DE', 'wortspiel kreuzwortraetsel', 'Games', 'de', 'active', 12),
('itunes', 'EU - Word FR', 'jeu mots croises', 'Games', 'fr', 'active', 12),
('itunes', 'MENA - Word Arabic', 'كلمات متقاطعة عربي', 'Games', 'eg', 'active', 12);

-- Trivia/Quiz Regional  
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'EU - Trivia DE', 'quiz wissen allgemeinwissen', 'Games', 'de', 'active', 12),
('itunes', 'EU - Trivia FR', 'quiz culture generale', 'Games', 'fr', 'active', 12),
('itunes', 'MENA - Trivia Arabic', 'مسابقات ثقافية اسئلة', 'Games', 'eg', 'active', 12);

-- Final count verification
SELECT 'EU/MENA sources inserted: ' || COUNT(*) FROM sa_sources WHERE created_at > NOW() - INTERVAL '1 minute';
