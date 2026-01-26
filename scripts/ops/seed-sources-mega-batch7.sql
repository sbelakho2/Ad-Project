-- MEGA BATCH 7: Final Push to 3000+ (150 sources)
-- Additional coverage for underrepresented regions + more Google Play

-- ==========================================
-- MEXICO (MX) - New Market (40 sources)
-- ==========================================
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'MX - Hypercasual Juegos', 'hypercasual juegos gratis', 'Games', 'mx', 'active', 12),
('itunes', 'MX - Match 3 Joyas', 'match 3 joyas puzzle', 'Games', 'mx', 'active', 12),
('itunes', 'MX - Match 3 Dulces', 'match 3 dulces candy', 'Games', 'mx', 'active', 12),
('itunes', 'MX - Idle Magnate', 'idle magnate tycoon', 'Games', 'mx', 'active', 12),
('itunes', 'MX - Juegos Palabras', 'juegos de palabras letras', 'Games', 'mx', 'active', 12),
('itunes', 'MX - Crucigramas', 'crucigramas español', 'Games', 'mx', 'active', 12),
('itunes', 'MX - Quiz Cultura', 'quiz cultura general trivia', 'Games', 'mx', 'active', 12),
('itunes', 'MX - Solitario', 'solitario cartas', 'Games', 'mx', 'active', 12),
('itunes', 'MX - Juegos Cartas', 'juegos de cartas mexicanos', 'Games', 'mx', 'active', 12),
('itunes', 'MX - Burbujas', 'burbujas shooter', 'Games', 'mx', 'active', 12),
('itunes', 'MX - Construir Ciudad', 'construir ciudad simulación', 'Games', 'mx', 'active', 12),
('itunes', 'MX - Cocinar', 'cocinar restaurante chef', 'Games', 'mx', 'active', 12),
('itunes', 'MX - Fútbol', 'fútbol juegos deportes', 'Games', 'mx', 'active', 12),
('itunes', 'MX - Carreras', 'carreras carros racing', 'Games', 'mx', 'active', 12),
('itunes', 'MX - Niños Educativo', 'niños juegos educativo', 'Games', 'mx', 'active', 12),
('google_play', 'GP MX - Hypercasual', 'hypercasual juegos', 'Games', 'mx', 'active', 12),
('google_play', 'GP MX - Match 3', 'match 3 puzzle joyas', 'Games', 'mx', 'active', 12),
('google_play', 'GP MX - Idle', 'idle tycoon magnate', 'Games', 'mx', 'active', 12),
('google_play', 'GP MX - Palabras', 'juegos palabras crucigramas', 'Games', 'mx', 'active', 12),
('google_play', 'GP MX - Solitario', 'solitario cartas', 'Games', 'mx', 'active', 12),
('google_play', 'GP MX - Burbujas', 'burbujas shooter', 'Games', 'mx', 'active', 12),
('google_play', 'GP MX - Cocinar', 'cocinar restaurante', 'Games', 'mx', 'active', 12),
('google_play', 'GP MX - Fútbol', 'fútbol juegos', 'Games', 'mx', 'active', 12),
('google_play', 'GP MX - Niños', 'niños juegos educativo', 'Games', 'mx', 'active', 12),
-- Non-Games MX
('itunes', 'MX - Productividad', 'productividad herramientas', 'Productivity', 'mx', 'active', 24),
('itunes', 'MX - Clima', 'clima pronóstico app', 'Weather', 'mx', 'active', 24),
('itunes', 'MX - Salud', 'salud fitness ejercicio', 'Health', 'mx', 'active', 24),
('itunes', 'MX - Foto', 'foto edición filtro', 'Photo', 'mx', 'active', 24),
('google_play', 'GP MX - Clima', 'clima pronóstico', 'Weather', 'mx', 'active', 24),
('google_play', 'GP MX - Salud', 'salud fitness', 'Health', 'mx', 'active', 24),
('google_play', 'GP MX - Foto', 'foto edición', 'Photo', 'mx', 'active', 24);

-- ==========================================
-- CANADA (CA) - New Market (35 sources)
-- ==========================================
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'CA - Hypercasual', 'hypercasual games free', 'Games', 'ca', 'active', 12),
('itunes', 'CA - Match 3 Jewels', 'match 3 jewels puzzle', 'Games', 'ca', 'active', 12),
('itunes', 'CA - Idle Tycoon', 'idle tycoon manager', 'Games', 'ca', 'active', 12),
('itunes', 'CA - Word Games', 'word games puzzle', 'Games', 'ca', 'active', 12),
('itunes', 'CA - Crossword', 'crossword puzzle', 'Games', 'ca', 'active', 12),
('itunes', 'CA - Brain Puzzle', 'brain puzzle logic', 'Games', 'ca', 'active', 12),
('itunes', 'CA - Quiz Trivia', 'quiz trivia canadian', 'Games', 'ca', 'active', 12),
('itunes', 'CA - Solitaire', 'solitaire card', 'Games', 'ca', 'active', 12),
('itunes', 'CA - Bubble Shooter', 'bubble shooter', 'Games', 'ca', 'active', 12),
('itunes', 'CA - Merge Games', 'merge puzzle combine', 'Games', 'ca', 'active', 12),
('itunes', 'CA - City Building', 'city building simulation', 'Games', 'ca', 'active', 12),
('itunes', 'CA - Cooking', 'cooking restaurant chef', 'Games', 'ca', 'active', 12),
('itunes', 'CA - Hockey', 'hockey games nhl', 'Games', 'ca', 'active', 12),
('itunes', 'CA - Racing', 'racing car', 'Games', 'ca', 'active', 12),
('itunes', 'CA - Kids', 'kids games educational', 'Games', 'ca', 'active', 12),
('google_play', 'GP CA - Hypercasual', 'hypercasual games', 'Games', 'ca', 'active', 12),
('google_play', 'GP CA - Match 3', 'match 3 puzzle', 'Games', 'ca', 'active', 12),
('google_play', 'GP CA - Idle', 'idle tycoon', 'Games', 'ca', 'active', 12),
('google_play', 'GP CA - Word', 'word games crossword', 'Games', 'ca', 'active', 12),
('google_play', 'GP CA - Solitaire', 'solitaire card', 'Games', 'ca', 'active', 12),
('google_play', 'GP CA - Bubble', 'bubble shooter', 'Games', 'ca', 'active', 12),
('google_play', 'GP CA - Cooking', 'cooking restaurant', 'Games', 'ca', 'active', 12),
('google_play', 'GP CA - Hockey', 'hockey games nhl', 'Games', 'ca', 'active', 12),
('google_play', 'GP CA - Kids', 'kids games', 'Games', 'ca', 'active', 12),
-- Non-Games CA
('itunes', 'CA - Productivity', 'productivity tools', 'Productivity', 'ca', 'active', 24),
('itunes', 'CA - Weather', 'weather forecast canada', 'Weather', 'ca', 'active', 24),
('itunes', 'CA - Fitness', 'fitness workout tracker', 'Health', 'ca', 'active', 24),
('itunes', 'CA - Photo', 'photo editor filter', 'Photo', 'ca', 'active', 24),
('google_play', 'GP CA - Weather', 'weather canada', 'Weather', 'ca', 'active', 24),
('google_play', 'GP CA - Fitness', 'fitness tracker', 'Health', 'ca', 'active', 24);

-- ==========================================
-- AUSTRALIA (AU) - New Market (35 sources)
-- ==========================================
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'AU - Hypercasual', 'hypercasual games free', 'Games', 'au', 'active', 12),
('itunes', 'AU - Match 3 Jewels', 'match 3 jewels puzzle', 'Games', 'au', 'active', 12),
('itunes', 'AU - Idle Tycoon', 'idle tycoon manager', 'Games', 'au', 'active', 12),
('itunes', 'AU - Word Games', 'word games puzzle', 'Games', 'au', 'active', 12),
('itunes', 'AU - Crossword', 'crossword puzzle', 'Games', 'au', 'active', 12),
('itunes', 'AU - Brain Puzzle', 'brain puzzle logic', 'Games', 'au', 'active', 12),
('itunes', 'AU - Quiz Trivia', 'quiz trivia australian', 'Games', 'au', 'active', 12),
('itunes', 'AU - Solitaire', 'solitaire card', 'Games', 'au', 'active', 12),
('itunes', 'AU - Bubble Shooter', 'bubble shooter', 'Games', 'au', 'active', 12),
('itunes', 'AU - Merge Games', 'merge puzzle combine', 'Games', 'au', 'active', 12),
('itunes', 'AU - City Building', 'city building simulation', 'Games', 'au', 'active', 12),
('itunes', 'AU - Cooking', 'cooking restaurant chef', 'Games', 'au', 'active', 12),
('itunes', 'AU - Rugby', 'rugby games nrl afl', 'Games', 'au', 'active', 12),
('itunes', 'AU - Racing', 'racing car', 'Games', 'au', 'active', 12),
('itunes', 'AU - Kids', 'kids games educational', 'Games', 'au', 'active', 12),
('google_play', 'GP AU - Hypercasual', 'hypercasual games', 'Games', 'au', 'active', 12),
('google_play', 'GP AU - Match 3', 'match 3 puzzle', 'Games', 'au', 'active', 12),
('google_play', 'GP AU - Idle', 'idle tycoon', 'Games', 'au', 'active', 12),
('google_play', 'GP AU - Word', 'word games crossword', 'Games', 'au', 'active', 12),
('google_play', 'GP AU - Solitaire', 'solitaire card', 'Games', 'au', 'active', 12),
('google_play', 'GP AU - Bubble', 'bubble shooter', 'Games', 'au', 'active', 12),
('google_play', 'GP AU - Cooking', 'cooking restaurant', 'Games', 'au', 'active', 12),
('google_play', 'GP AU - Rugby', 'rugby afl games', 'Games', 'au', 'active', 12),
('google_play', 'GP AU - Kids', 'kids games', 'Games', 'au', 'active', 12),
-- Non-Games AU
('itunes', 'AU - Productivity', 'productivity tools', 'Productivity', 'au', 'active', 24),
('itunes', 'AU - Weather', 'weather forecast australia', 'Weather', 'au', 'active', 24),
('itunes', 'AU - Fitness', 'fitness workout tracker', 'Health', 'au', 'active', 24),
('itunes', 'AU - Photo', 'photo editor filter', 'Photo', 'au', 'active', 24),
('google_play', 'GP AU - Weather', 'weather australia', 'Weather', 'au', 'active', 24),
('google_play', 'GP AU - Fitness', 'fitness tracker', 'Health', 'au', 'active', 24);

-- ==========================================
-- RUSSIA (RU) - New Market (40 sources)
-- ==========================================
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'RU - Гиперказуальные', 'гиперказуальные игры бесплатно', 'Games', 'ru', 'active', 12),
('itunes', 'RU - Три в Ряд', 'три в ряд головоломка', 'Games', 'ru', 'active', 12),
('itunes', 'RU - Три в Ряд Шарики', 'три в ряд шарики', 'Games', 'ru', 'active', 12),
('itunes', 'RU - Idle Магнат', 'idle магнат тайкун', 'Games', 'ru', 'active', 12),
('itunes', 'RU - Игры Слова', 'игры в слова буквы', 'Games', 'ru', 'active', 12),
('itunes', 'RU - Кроссворды', 'кроссворды русские', 'Games', 'ru', 'active', 12),
('itunes', 'RU - Логические Игры', 'логические игры головоломки', 'Games', 'ru', 'active', 12),
('itunes', 'RU - Викторина', 'викторина эрудит знания', 'Games', 'ru', 'active', 12),
('itunes', 'RU - Пасьянс', 'пасьянс карточные игры', 'Games', 'ru', 'active', 12),
('itunes', 'RU - Дурак', 'дурак карточная игра онлайн', 'Games', 'ru', 'active', 12),
('itunes', 'RU - Преферанс', 'преферанс карточная игра', 'Games', 'ru', 'active', 12),
('itunes', 'RU - Шарики', 'шарики стрелялка', 'Games', 'ru', 'active', 12),
('itunes', 'RU - Merge', 'merge объединение игры', 'Games', 'ru', 'active', 12),
('itunes', 'RU - Строить Город', 'строить город симулятор', 'Games', 'ru', 'active', 12),
('itunes', 'RU - Ферма', 'ферма симулятор', 'Games', 'ru', 'active', 12),
('itunes', 'RU - Готовка', 'готовка ресторан игры', 'Games', 'ru', 'active', 12),
('itunes', 'RU - Футбол', 'футбол игры', 'Games', 'ru', 'active', 12),
('itunes', 'RU - Гонки', 'гонки машины игры', 'Games', 'ru', 'active', 12),
('itunes', 'RU - Судоку', 'судоку числа', 'Games', 'ru', 'active', 12),
('itunes', 'RU - Дети', 'детские игры обучающие', 'Games', 'ru', 'active', 12),
('google_play', 'GP RU - Гиперказуальные', 'гиперказуальные игры', 'Games', 'ru', 'active', 12),
('google_play', 'GP RU - Три в Ряд', 'три в ряд головоломка', 'Games', 'ru', 'active', 12),
('google_play', 'GP RU - Idle', 'idle магнат тайкун', 'Games', 'ru', 'active', 12),
('google_play', 'GP RU - Слова', 'игры слова кроссворды', 'Games', 'ru', 'active', 12),
('google_play', 'GP RU - Викторина', 'викторина эрудит', 'Games', 'ru', 'active', 12),
('google_play', 'GP RU - Пасьянс', 'пасьянс карточные', 'Games', 'ru', 'active', 12),
('google_play', 'GP RU - Дурак', 'дурак карточная', 'Games', 'ru', 'active', 12),
('google_play', 'GP RU - Шарики', 'шарики стрелялка', 'Games', 'ru', 'active', 12),
('google_play', 'GP RU - Симулятор', 'симулятор город ферма', 'Games', 'ru', 'active', 12),
('google_play', 'GP RU - Готовка', 'готовка ресторан', 'Games', 'ru', 'active', 12),
('google_play', 'GP RU - Футбол', 'футбол игры', 'Games', 'ru', 'active', 12),
('google_play', 'GP RU - Дети', 'детские игры', 'Games', 'ru', 'active', 12),
-- Non-Games RU
('itunes', 'RU - Продуктивность', 'продуктивность инструменты', 'Productivity', 'ru', 'active', 24),
('itunes', 'RU - Погода', 'погода прогноз', 'Weather', 'ru', 'active', 24),
('itunes', 'RU - Фитнес', 'фитнес тренировки', 'Health', 'ru', 'active', 24),
('itunes', 'RU - Фото', 'фото редактор фильтр', 'Photo', 'ru', 'active', 24),
('google_play', 'GP RU - Погода', 'погода прогноз', 'Weather', 'ru', 'active', 24),
('google_play', 'GP RU - Фитнес', 'фитнес тренировки', 'Health', 'ru', 'active', 24),
('google_play', 'GP RU - Фото', 'фото редактор', 'Photo', 'ru', 'active', 24);

-- ==========================================
-- INDONESIA (ID) - New Market (30 sources)
-- ==========================================
INSERT INTO sa_sources (source_type, source_name, search_query, category, region, status, scrape_interval_hours) VALUES
('itunes', 'ID - Hypercasual', 'hypercasual games gratis', 'Games', 'id', 'active', 12),
('itunes', 'ID - Match 3', 'match 3 puzzle permata', 'Games', 'id', 'active', 12),
('itunes', 'ID - Idle Tycoon', 'idle tycoon pengusaha', 'Games', 'id', 'active', 12),
('itunes', 'ID - Kata', 'permainan kata teka teki', 'Games', 'id', 'active', 12),
('itunes', 'ID - Teka Teki', 'teka teki silang indonesia', 'Games', 'id', 'active', 12),
('itunes', 'ID - Kuis', 'kuis pengetahuan umum', 'Games', 'id', 'active', 12),
('itunes', 'ID - Kartu', 'permainan kartu solitaire', 'Games', 'id', 'active', 12),
('itunes', 'ID - Balon', 'tembak balon bubble', 'Games', 'id', 'active', 12),
('itunes', 'ID - Simulasi', 'simulasi kota pertanian', 'Games', 'id', 'active', 12),
('itunes', 'ID - Masak', 'masak restoran permainan', 'Games', 'id', 'active', 12),
('itunes', 'ID - Sepak Bola', 'sepak bola permainan', 'Games', 'id', 'active', 12),
('itunes', 'ID - Balap', 'balap mobil motor', 'Games', 'id', 'active', 12),
('itunes', 'ID - Anak', 'permainan anak edukasi', 'Games', 'id', 'active', 12),
('google_play', 'GP ID - Hypercasual', 'hypercasual games', 'Games', 'id', 'active', 12),
('google_play', 'GP ID - Match 3', 'match 3 puzzle', 'Games', 'id', 'active', 12),
('google_play', 'GP ID - Idle', 'idle tycoon', 'Games', 'id', 'active', 12),
('google_play', 'GP ID - Kata', 'permainan kata', 'Games', 'id', 'active', 12),
('google_play', 'GP ID - Kuis', 'kuis pengetahuan', 'Games', 'id', 'active', 12),
('google_play', 'GP ID - Kartu', 'permainan kartu', 'Games', 'id', 'active', 12),
('google_play', 'GP ID - Balon', 'tembak balon', 'Games', 'id', 'active', 12),
('google_play', 'GP ID - Masak', 'masak restoran', 'Games', 'id', 'active', 12),
('google_play', 'GP ID - Sepak Bola', 'sepak bola', 'Games', 'id', 'active', 12),
('google_play', 'GP ID - Anak', 'permainan anak', 'Games', 'id', 'active', 12),
-- Non-Games ID
('itunes', 'ID - Cuaca', 'cuaca prakiraan indonesia', 'Weather', 'id', 'active', 24),
('itunes', 'ID - Kesehatan', 'kesehatan kebugaran', 'Health', 'id', 'active', 24),
('itunes', 'ID - Foto', 'foto editor filter', 'Photo', 'id', 'active', 24),
('google_play', 'GP ID - Cuaca', 'cuaca prakiraan', 'Weather', 'id', 'active', 24),
('google_play', 'GP ID - Kesehatan', 'kesehatan kebugaran', 'Health', 'id', 'active', 24),
('google_play', 'GP ID - Foto', 'foto editor', 'Photo', 'id', 'active', 24);

SELECT 'Batch 7 (Final Push - MX/CA/AU/RU/ID) inserted' as status;
