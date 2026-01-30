import re
import sys
from collections import Counter

# Read the samples file (from argument or default)
input_file = sys.argv[1] if len(sys.argv) > 1 else '/tmp/40_samples_output.txt'
with open(input_file, 'r') as f:
    content = f.read()

# Split into individual samples
samples = re.split(r'--- SAMPLE \d+ \([^)]+\) ---', content)[1:]
num_samples = len(samples) if samples else 40  # Fallback to 40 if regex fails

# Alternative count from template types
template_matches = re.findall(r'--- SAMPLE \d+ \(', content)
if template_matches:
    num_samples = len(template_matches)

print("=" * 80)
print("QUALITY ANALYSIS OF 40 EMAIL SAMPLES")
print("=" * 80)
print(f"\nTotal samples analyzed: {num_samples}")

# 1. Check for unsubstantiated claims (BAD)
bad_patterns = [
    (r'\d+%\s*(increase|boost|higher|improvement|growth|better)', 'Percentage claim'),
    (r'\$\d+[kK]\s*(per|/)', 'Dollar amount claim'),
    (r'guaranteed|guarantee|proven to|will definitely', 'Guarantee language'),
    (r'best in (class|market|industry)', 'Superlative claim'),
    (r'outperform(s|ed|ing)?', 'Outperformance claim'),
    (r'(highest|lowest|best|fastest) (eCPM|fill|revenue|rates)', 'Comparative claim'),
]

# 2. Check for good honest language (GOOD)
good_patterns = [
    (r'no (commitment|promises|pressure|strings attached|risk)', 'No pressure language'),
    (r'depends on|based on your', 'Conditional language'),
    (r'curious|wondering|question', 'Curious/questioning tone'),
    (r'if (this is|you|that|helpful|interested)', 'Conditional offer'),
    (r'would it help|want to see|happy to|worth exploring|want me to', 'Helpful tone'),
    (r'transparen|impression-level|data across|visibility|clarity', 'Transparency language'),
    (r'your (data|revenue|control|setup|current)', 'Ownership language'),
]

# 3. Check for product features mentioned (GOOD - shows value prop)
feature_patterns = [
    (r'transparency|signed.*log|Ed25519', 'Transparency Engine'),
    (r'migration studio|shadow|rollback', 'Migration Studio'),
    (r'VRA|verifiable revenue|revenue auditor|reconcil', 'VRA/Revenue Verification'),
    (r'SSAI|server-side.*ad|HLS|DASH', 'CTV/SSAI'),
    (r'circuit breaker|kill switch|OTA|reliability', 'Reliability Features'),
    (r'consent|GDPR|TCF|privacy', 'Privacy/Consent'),
    (r'ML.*waterfall|bid prediction|optimize', 'ML Optimization'),
]

print("\n" + "=" * 80)
print("1. PROBLEMATIC CLAIMS CHECK (should be 0 or minimal)")
print("=" * 80)

bad_counts = Counter()
bad_examples = {}
for pattern, name in bad_patterns:
    matches = re.findall(pattern, content, re.IGNORECASE)
    if matches:
        bad_counts[name] = len(matches)
        bad_examples[name] = matches[:3]

if bad_counts:
    for name, count in bad_counts.most_common():
        print(f"  WARNING  {name}: {count} occurrences")
        if name in bad_examples:
            print(f"      Examples: {bad_examples[name]}")
else:
    print("  OK No problematic claims found!")

print("\n" + "=" * 80)
print("2. HONEST LANGUAGE CHECK (higher is better)")
print("=" * 80)

good_counts = Counter()
for pattern, name in good_patterns:
    count = len(re.findall(pattern, content, re.IGNORECASE))
    if count > 0:
        good_counts[name] = count

for name, count in good_counts.most_common():
    print(f"  OK {name}: {count} occurrences")

print("\n" + "=" * 80)
print("3. PRODUCT FEATURE COVERAGE (shows value proposition)")
print("=" * 80)

feature_counts = Counter()
for pattern, name in feature_patterns:
    count = len(re.findall(pattern, content, re.IGNORECASE))
    if count > 0:
        feature_counts[name] = count

for name, count in feature_counts.most_common():
    print(f"  {name}: {count} mentions")

print("\n" + "=" * 80)
print("4. TEMPLATE VARIETY CHECK")
print("=" * 80)

template_types = re.findall(r'SAMPLE \d+ \(([^)]+)\)', content)
template_counts = Counter(template_types)
for template, count in template_counts.most_common():
    print(f"  - {template}: {count} samples")

print("\n" + "=" * 80)
print("5. PERSONALIZATION CHECK")
print("=" * 80)

# Check that app names are properly substituted
unsubstituted = re.findall(r'\{\{[A-Z_]+\}\}', content)
if unsubstituted:
    print(f"  WARNING  Found {len(unsubstituted)} unsubstituted variables: {set(unsubstituted)}")
else:
    print("  OK All personalization variables properly substituted!")

# Check for variety in app names
app_names = re.findall(r'TO: \w+@([a-z]+)\.com', content)
unique_apps = len(set(app_names))
print(f"  OK {unique_apps} unique app recipients (diversity: {unique_apps * 100 // max(num_samples, 1)}%)")

print("\n" + "=" * 80)
print("6. GRAMMAR/QUALITY ISSUES")
print("=" * 80)

# Check for awkward phrasing from spintax
issues = []
awkward = re.findall(r'\b(is is|the the|to to|for for|that that)\b', content, re.IGNORECASE)
if awkward:
    issues.append(f"Repeated words: {awkward[:5]}")

incomplete_spintax = re.findall(r'\{[^}]+\|[^}]+\}', content)
if incomplete_spintax:
    issues.append(f"Unprocessed spintax: {len(incomplete_spintax)} found")

# Check for grammar issues
grammar_issues = re.findall(r'(visibility into every ad network gets|how it be useful|it seem not worth)', content, re.IGNORECASE)
if grammar_issues:
    issues.append(f"Grammar issues: {len(grammar_issues)} found - {grammar_issues[:3]}")

# Check for awkward CTA phrasing
bad_ctas = re.findall(r'(want to see how it be|would a walkthrough works)', content, re.IGNORECASE)
if bad_ctas:
    issues.append(f"Awkward CTA phrasing: {len(bad_ctas)} found")

if issues:
    for issue in issues:
        print(f"  WARNING  {issue}")
else:
    print("  OK No major grammar issues detected")

print("\n" + "=" * 80)
print("OVERALL QUALITY SCORE")
print("=" * 80)

score = 0
max_score = 100

# Deduct for bad claims (0-20 points)
score += max(0, 20 - len(bad_counts) * 5)

# Add for good language (0-30 points) - adjusted for realistic per-email density
# Target: ~3 good language indicators per email (120 for 40 emails) = full score
good_total = sum(good_counts.values())
score += min(30, (good_total * 30) // 120)

# Add for feature coverage (0-25 points)
score += min(25, len(feature_counts) * 4)

# Add for personalization (0-15 points)
score += 15 if not unsubstituted else 0

# Add for template variety (0-10 points)
score += min(10, len(template_counts) * 2)

# Deduct for grammar issues
score -= len(issues) * 3

print(f"\n  Quality Score: {score}/100")
if score >= 90:
    print("  EXCELLENT - Templates are high quality and honest")
elif score >= 75:
    print("  GOOD - Templates are well-written with minor issues")
elif score >= 60:
    print("  FAIR - Some improvements needed")
else:
    print("  NEEDS WORK - Significant issues detected")

print("\n" + "=" * 80)
