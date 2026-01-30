#!/usr/bin/env python3
"""
Email Quality Analyzer V2 - More Meaningful Metrics

Instead of counting keyword occurrences, this measures:
1. No unsubstantiated claims (critical)
2. Clear value proposition per email
3. Natural call-to-action presence
4. Appropriate length (50-150 words ideal)
5. Personalization depth
6. Professional tone (no sleazy tactics)
7. Grammar/flow quality
"""

import re
import sys
from collections import Counter

# Read the samples file
input_file = sys.argv[1] if len(sys.argv) > 1 else '/tmp/40_samples_v4_output.txt'
with open(input_file, 'r') as f:
    content = f.read()

# Split into individual samples - handle nested parens in template names
sample_blocks = re.split(r'--- SAMPLE \d+ \(.+?\) ---', content)[1:]
samples = []
for block in sample_blocks:
    # Extract body content (handle line breaks in SSH output)
    body_match = re.search(r'BODY:\n(.+?)(?:----|\Z)', block, re.DOTALL)
    if body_match:
        body = body_match.group(1).strip()
        samples.append(body)

num_samples = len(samples) if samples else 1  # Avoid division by zero
print(f"DEBUG: Found {num_samples} samples")

print("=" * 80)
print("EMAIL QUALITY ANALYSIS V2 - Meaningful Metrics")
print("=" * 80)
print(f"\nTotal samples analyzed: {num_samples}")

# 1. UNSUBSTANTIATED CLAIMS CHECK (Critical - instant fail)
print("\n" + "=" * 80)
print("1. UNSUBSTANTIATED CLAIMS (must be 0)")
print("=" * 80)

bad_patterns = [
    (r'\d+%\s*(increase|boost|higher|improvement|growth|better|more)', 'Fake percentage claim'),
    (r'\$\d+[kKmM]\s*(per|/|in|of)', 'Fake dollar amount'),
    (r'guaranteed|guarantee|proven to|will definitely', 'Guarantee language'),
    (r'best in (class|market|industry)', 'Superlative claim'),
    (r'outperform(s|ed|ing)?', 'Outperformance claim'),
    (r'(highest|lowest|fastest|#1)\b', 'Comparative superlative'),  # Removed 'best' - common sign-off
    (r'best\s+(solution|platform|tool|service|option|choice|result)', 'Superlative product claim'),
]

bad_found = []
for pattern, name in bad_patterns:
    matches = re.findall(pattern, content, re.IGNORECASE)
    if matches:
        bad_found.append(f"{name}: {len(matches)} found")

claims_score = 25 if not bad_found else max(0, 25 - len(bad_found) * 10)
if bad_found:
    for issue in bad_found:
        print(f"  ❌ {issue}")
else:
    print("  ✅ No unsubstantiated claims found")
print(f"  Score: {claims_score}/25")

# 2. VALUE PROPOSITION CLARITY
print("\n" + "=" * 80)
print("2. VALUE PROPOSITION CLARITY")
print("=" * 80)

value_indicators = [
    r'transparen',
    r'visibility|clarity|see exactly',
    r'single (sdk|integration|codebase)',
    r'unified|one (dashboard|integration)',
    r'consent|privacy|gdpr|tcf',
    r'ssai|server-side',
    r'ml|machine learning|automated',
    r'migration|parallel|shadow',
    r'reward|vra',
]

emails_with_value = 0
for sample in samples:
    has_value = any(re.search(p, sample, re.IGNORECASE) for p in value_indicators)
    if has_value:
        emails_with_value += 1

value_pct = (emails_with_value / max(num_samples, 1)) * 100
value_score = min(20, int(value_pct / 5))
print(f"  Emails with clear value prop: {emails_with_value}/{num_samples} ({value_pct:.0f}%)")
print(f"  Score: {value_score}/20")

# 3. CALL-TO-ACTION PRESENCE
print("\n" + "=" * 80)
print("3. CALL-TO-ACTION QUALITY")
print("=" * 80)

soft_cta_patterns = [
    r'curious|wondering',
    r'happy to|would be glad',
    r'interested|worth',
    r'want to (see|know|learn)',
    r'feel free|no pressure|totally (fine|understand|free)',
    r'\?$',  # ends with question
]

hard_cta_patterns = [
    r'book a (call|demo|meeting)',
    r'schedule (now|today)',
    r'click here|sign up',
    r'limited (time|offer)',
]

soft_ctas = sum(1 for sample in samples if any(re.search(p, sample, re.IGNORECASE) for p in soft_cta_patterns))
hard_ctas = sum(1 for sample in samples if any(re.search(p, sample, re.IGNORECASE) for p in hard_cta_patterns))

# Soft CTAs are good for cold outreach, hard CTAs are pushy
cta_score = min(15, soft_ctas * 15 // max(num_samples, 1))
print(f"  Soft/conversational CTAs: {soft_ctas}/{num_samples}")
print(f"  Hard/pushy CTAs: {hard_ctas}/{num_samples} (lower is better)")
if hard_ctas > 0:
    cta_score = max(0, cta_score - hard_ctas * 2)
print(f"  Score: {cta_score}/15")

# 4. EMAIL LENGTH (50-150 words ideal for cold email)
print("\n" + "=" * 80)
print("4. EMAIL LENGTH (50-150 words ideal)")
print("=" * 80)

good_length = 0
too_short = 0
too_long = 0

for sample in samples:
    word_count = len(sample.split())
    if 50 <= word_count <= 150:
        good_length += 1
    elif word_count < 50:
        too_short += 1
    else:
        too_long += 1

length_pct = (good_length / max(num_samples, 1)) * 100
length_score = min(10, int(length_pct / 10))
print(f"  Optimal length (50-150 words): {good_length}/{num_samples}")
print(f"  Too short (<50): {too_short}, Too long (>150): {too_long}")
print(f"  Score: {length_score}/10")

# 5. PERSONALIZATION DEPTH
print("\n" + "=" * 80)
print("5. PERSONALIZATION DEPTH")
print("=" * 80)

# Check for meaningful personalization beyond just name
personalization_patterns = [
    r'{{FIRST_NAME}}|{{APP_NAME}}|{{APP_GENRE}}',  # Should be substituted
    r'\b(your|you)\b',  # Reader-focused language
]

# Check for unsubstituted variables (bad)
unsubstituted = re.findall(r'\{\{[A-Z_]+\}\}', content)

# Check for reader-focused language (good)
you_count = len(re.findall(r'\b(your|you)\b', content, re.IGNORECASE))
you_per_email = you_count / max(num_samples, 1)

personalization_score = 15 if not unsubstituted else 0
if you_per_email < 2:
    personalization_score = max(0, personalization_score - 5)

print(f"  Unsubstituted variables: {len(unsubstituted)} (should be 0)")
print(f"  'You/Your' usage: {you_per_email:.1f} per email (2+ is good)")
print(f"  Score: {personalization_score}/15")

# 6. PROFESSIONAL TONE (no sleazy tactics)
print("\n" + "=" * 80)
print("6. PROFESSIONAL TONE")
print("=" * 80)

sleazy_patterns = [
    r'act now|urgent|limited time|last chance',
    r'don\'t miss|you\'re missing',
    r'everyone is|competitors are',
    r'secret|hack|trick',
    r'unbelievable|incredible|amazing',
]

sleazy_found = []
for pattern in sleazy_patterns:
    matches = re.findall(pattern, content, re.IGNORECASE)
    if matches:
        sleazy_found.extend(matches[:2])

tone_score = 10 if not sleazy_found else max(0, 10 - len(sleazy_found) * 3)
if sleazy_found:
    print(f"  ⚠️  Sleazy language found: {sleazy_found[:5]}")
else:
    print("  ✅ Professional, non-pushy tone throughout")
print(f"  Score: {tone_score}/10")

# 7. GRAMMAR/FLOW
print("\n" + "=" * 80)
print("7. GRAMMAR & FLOW")
print("=" * 80)

grammar_issues = []

# Check for common grammar problems
repeated_words = re.findall(r'\b(\w+)\s+\1\b', content, re.IGNORECASE)
if repeated_words:
    grammar_issues.append(f"Repeated words: {repeated_words[:3]}")

# Check for incomplete sentences (lines ending without punctuation)
odd_endings = re.findall(r'[a-z]{3,}\n[A-Z]', content)
if len(odd_endings) > 5:
    grammar_issues.append(f"Potential incomplete sentences: {len(odd_endings)}")

# Check for unprocessed spintax
leftover_spintax = re.findall(r'\{[^}]+\|[^}]+\}', content)
if leftover_spintax:
    grammar_issues.append(f"Unprocessed spintax: {len(leftover_spintax)}")

grammar_score = 5 if not grammar_issues else max(0, 5 - len(grammar_issues))
if grammar_issues:
    for issue in grammar_issues:
        print(f"  ⚠️  {issue}")
else:
    print("  ✅ Good grammar and flow")
print(f"  Score: {grammar_score}/5")

# FINAL SCORE
print("\n" + "=" * 80)
print("OVERALL QUALITY SCORE")
print("=" * 80)

total_score = claims_score + value_score + cta_score + length_score + personalization_score + tone_score + grammar_score

print(f"""
  Breakdown:
    No false claims:     {claims_score}/25
    Value proposition:   {value_score}/20
    CTA quality:         {cta_score}/15
    Email length:        {length_score}/10
    Personalization:     {personalization_score}/15
    Professional tone:   {tone_score}/10
    Grammar/flow:        {grammar_score}/5
    ─────────────────────────
    TOTAL:               {total_score}/100
""")

if total_score >= 90:
    print("  🏆 EXCELLENT - High quality, professional cold emails")
elif total_score >= 75:
    print("  ✅ GOOD - Well-written with minor improvements possible")
elif total_score >= 60:
    print("  ⚠️  FAIR - Some areas need attention")
else:
    print("  ❌ NEEDS WORK - Significant issues to address")

print("\n" + "=" * 80)
