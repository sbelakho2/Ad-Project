#!/usr/bin/env bash
# Renders the nine Open Graph cards (1200x630 PNGs) into site/static/og/.
#
# Input:  site/static/og/template.html plus the route titles listed below.
# Output: site/static/og/<route>.png for the full marketing route set.
#
# The template is captured with headless Chromium through the Playwright CLI.
# Each card is rendered from a temporary copy in which the @font-face URLs point
# at the committed woff2 files under site/static/fonts/ through file:// URLs and
# the h1 line carries the route title, so the capture always uses the real
# self-hosted Manrope and IBM Plex Mono subsets. The script is idempotent: rerun
# it after any template, font or title change; it overwrites the PNGs in place.
#
# Requirements: python3, node + npx. The Playwright package and its Chromium
# build are fetched into the local npx / browser caches on first use. Set
# PLAYWRIGHT_VERSION to pin a different Playwright release (the Chromium build
# revision must be installed in the Playwright browser cache).
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
TEMPLATE="$ROOT/site/static/og/template.html"
OUT_DIR="$ROOT/site/static/og"
FONT_URL="file://$ROOT/site/static/fonts/"
PLAYWRIGHT_VERSION="${PLAYWRIGHT_VERSION:-1.63.0}"

TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/apex-og.XXXXXX")
trap 'rm -rf "$TMP_DIR"' EXIT

render_card() {
  local slug=$1 title=$2
  python3 - "$TEMPLATE" "$TMP_DIR/$slug.html" "$title" "$FONT_URL" <<'PY'
import sys

template, out, title, font_url = sys.argv[1:5]
html = open(template, encoding="utf-8").read()
html = html.replace("../fonts/", font_url)
start = html.index('<h1 class="og-title">')
end = html.index("</h1>", start) + len("</h1>")
html = html[:start] + '<h1 class="og-title">' + title + "</h1>" + html[end:]
open(out, "w", encoding="utf-8").write(html)
PY
  npx -y "playwright@$PLAYWRIGHT_VERSION" screenshot \
    --viewport-size=1200,630 --wait-for-timeout=400 \
    "file://$TMP_DIR/$slug.html" "$OUT_DIR/$slug.png"
}

render_card home "Improve the mediation you already run."
render_card how-it-works "A defined change. A recorded result."
render_card pricing "A share of qualifying gains."
render_card proof "Follow the result to its source."
render_card security "Clear access. Defined boundaries."
render_card for-publishers "For studios where ad ops is one job among many."
render_card free-audit "Request the free audit."
render_card guides "Guides."
render_card support "Support."

python3 - "$OUT_DIR" <<'PY'
import glob
import os
import sys

from PIL import Image

out_dir = sys.argv[1]
failures = []
for path in sorted(glob.glob(os.path.join(out_dir, "*.png"))):
    with Image.open(path) as image:
        if image.size != (1200, 630):
            failures.append(f"{path}: {image.size}")
        else:
            print(f"ok {os.path.basename(path)} {image.size[0]}x{image.size[1]}")
if failures:
    raise SystemExit("unexpected dimensions: " + "; ".join(failures))
PY
