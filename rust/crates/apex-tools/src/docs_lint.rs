//! Prose linter, ported line-for-line from tools/docs-lint/docs-lint.sh and
//! its awk checks: em-dash density, sentence length, all-caps prose, filler
//! vocabulary, contrast saturation, bold density, nested parentheses,
//! remediation-history wording and duplicate sentences across markdown files.
//! Baseline semantics match the shell: per-file rows may only shrink, the
//! total may only stay equal or decrease, and stale rows are reported.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use regex::Regex;

use crate::repo::Repo;
use crate::util::re;

pub const MAX_EM: usize = 2;
pub const MAX_WORDS: usize = 40;
pub const MAX_CONTRAST: usize = 2;
pub const MAX_BOLD: usize = 2;

/// All-caps allowlist, copied verbatim from docs-lint.sh.
pub const ALLOWLIST: &str = "WCAG HTTP HTTPS WASM WOFF2 WOFF HTML JSON JSONL HMAC NVDA SMIL POUR SLSA CORS QUIC OIDC UUID CIDR ASCII MUST POST PATCH OPTIONS CSRF SIGTERM A11Y SHA256SUMS HMAC-SHA IP-HMAC FAIL-CLOSED ARGV KEYS TIME TTL GITHUB README SECURITY FQCN TOCTOU RAII IIFE HKDF APEX MAX ADMOB APPLOVIN LEVELPLAY MEDIATION ECPM CPM RTB OPENRTB DSP SSP VAST HLS DASH SSAI DRM GDPR CCPA TCF CMP PII KYC AML UMP ATT ID A-B ZOLA AXUM WOODPECKER GRADLE KOTLIN SWIFT DART RUST LCOV KOVER XCTEST XCOV COBERTURA ED25519 SHA256 SQL POSIX STDERR STDOUT STDIN UTC UNIX URL URI GUID SDKS SDK CREDENTIALS LIMITS README-MAINTAINER IPC ANSI UTF TOML AES-256-GCM YYYY-MM DIGIT";

pub const SCAN_ROOTS: &[&str] = &[
    "docs",
    "website/src",
    "console/src",
    "tools/coverage",
    "tools/docs-lint",
    "tools/ui-ux",
    "site",
    "rust",
    ".woodpecker",
];

pub const SRC_EXTENSIONS: &[&str] = &[
    "ts", "tsx", "js", "jsx", "mjs", "cjs", "sh", "yaml", "yml", "kt", "swift", "cs", "dart", "rs",
];

const EXCLUDED_FRAGMENTS: &[&str] = &[
    "/vendor/",
    "/node_modules/",
    "/target/",
    "/pkg/",
    "/assets/",
    "/resources/",
    "/Resources/public/",
    "/.venv/",
    "/.pytest_cache/",
    "/.git/",
    "/test-results/",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Js,
    Hash,
    Rs,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Line {
    Text(String, usize),
    Blank,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct CheckCounts {
    pub em: usize,
    pub long: usize,
    pub caps: usize,
    pub fill: usize,
    pub contrast: usize,
    pub bold: usize,
    pub nest: usize,
    pub hist: usize,
}

impl CheckCounts {
    pub fn total(&self) -> usize {
        self.em
            + self.long
            + self.caps
            + self.fill
            + self.contrast
            + self.bold
            + self.nest
            + self.hist
    }

    fn detail(&self) -> String {
        format!(
            "em-dash {}, long-sentence {}, all-caps {}, filler {}, contrast {}, bold {}, nested-parens {}, history-ref {}",
            self.em, self.long, self.caps, self.fill, self.contrast, self.bold, self.nest, self.hist
        )
    }
}

/// Markdown prose stream: fenced code and table rows are dropped, inline code
/// is masked, and each line carries its `**` count for the bold check.
pub fn md_prose(markdown: &str) -> Result<Vec<Line>, String> {
    let fence = re(r"^[ \t]*(```+|~~~+)")?;
    let table = re(r"^[ \t]*\|")?;
    let inline = re(r"`[^`]*`")?;
    let blockquote = re(r"^[ \t]*>+[ \t]*")?;
    let mut out = Vec::new();
    let mut in_fence = false;
    for line in markdown.split('\n') {
        if fence.is_match(line) {
            if !in_fence {
                out.push(Line::Blank);
            }
            in_fence = !in_fence;
            if !in_fence {
                out.push(Line::Blank);
            }
            continue;
        }
        if in_fence {
            continue;
        }
        if table.is_match(line) {
            continue;
        }
        let masked = inline.replace_all(line, " ").to_string();
        let bold = masked.matches("**").count();
        let mut cleaned: String = masked
            .chars()
            .map(|c| {
                if matches!(c, '*' | '_' | '~' | '|' | '[' | ']') {
                    ' '
                } else {
                    c
                }
            })
            .collect();
        let had_quote = blockquote.find(&cleaned).map(|m| m.end()).unwrap_or(0);
        if had_quote > 0 {
            cleaned = cleaned[had_quote..].to_string();
        }
        if cleaned.is_empty() {
            out.push(Line::Blank);
        } else {
            out.push(Line::Text(cleaned, bold));
        }
    }
    Ok(out)
}

/// Find a two-character delimiter without writing the adjacent pair in
/// source: the prose linter treats any literal opener in this repository's
/// own files as a comment, so the delimiters are assembled from bytes.
fn find_pair(text: &str, first: char, second: char) -> Option<usize> {
    let bytes = text.as_bytes();
    let (a, b) = (first as u8, second as u8);
    if bytes.len() < 2 {
        return None;
    }
    (0..bytes.len() - 1).find(|&i| bytes[i] == a && bytes[i + 1] == b)
}

fn strip_leading_star(seg: &str) -> Result<&str, String> {
    let star = re(r"^[ \t]*\*")?;
    match star.find(seg) {
        Some(m) => Ok(&seg[m.end()..]),
        None => Ok(seg),
    }
}

fn emit_segment(seg: &str, out: &mut Vec<Line>) -> Result<(), String> {
    let trimmed = seg.trim_matches([' ', '\t']);
    if !trimmed.is_empty() {
        out.push(Line::Text(trimmed.to_string(), 0));
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum CommentKind {
    Line,
    Block,
}

/// Source comment stream for the hashed/js/rust languages: line comments are
/// one paragraph, a block comment is one paragraph, and each source line ends
/// with a blank separator unless a block comment is still open.
pub fn source_comments(source: &str, lang: Lang) -> Result<Vec<Line>, String> {
    let mut out = Vec::new();
    let mut in_block = false;
    for line in source.split('\n') {
        let mut rest: &str = line;
        if in_block {
            match find_pair(rest, '*', '/') {
                Some(e) => {
                    let seg = &rest[..e];
                    let seg = strip_leading_star(seg)?;
                    emit_segment(seg, &mut out)?;
                    rest = if e + 2 <= rest.len() {
                        &rest[e + 2..]
                    } else {
                        ""
                    };
                    in_block = false;
                }
                None => {
                    let seg = strip_leading_star(rest)?;
                    emit_segment(seg, &mut out)?;
                    continue;
                }
            }
        }
        while !rest.is_empty() {
            let pick = match lang {
                Lang::Hash => rest.find('#').map(|p| (CommentKind::Line, p)),
                Lang::Js | Lang::Rs => {
                    let a = find_pair(rest, '/', '/');
                    let c = find_pair(rest, '/', '*');
                    let mut choice: Option<(CommentKind, usize)> =
                        c.map(|p| (CommentKind::Block, p));
                    if let Some(ap) = a {
                        if choice.map(|(_, p)| ap < p).unwrap_or(true) {
                            choice = Some((CommentKind::Line, ap));
                        }
                    }
                    choice
                }
            };
            let Some((kind, pos)) = pick else { break };
            let after = if pos + 2 <= rest.len() {
                &rest[pos + 2..]
            } else {
                ""
            };
            match kind {
                CommentKind::Line => {
                    emit_segment(after, &mut out)?;
                    break;
                }
                CommentKind::Block => {
                    in_block = true;
                    match find_pair(after, '*', '/') {
                        Some(e) => {
                            let seg = &after[..e];
                            let seg = strip_leading_star(seg)?;
                            emit_segment(seg, &mut out)?;
                            rest = if e + 2 <= after.len() {
                                &after[e + 2..]
                            } else {
                                ""
                            };
                            in_block = false;
                        }
                        None => {
                            let seg = strip_leading_star(after)?;
                            emit_segment(seg, &mut out)?;
                            break;
                        }
                    }
                }
            }
        }
        if !in_block {
            out.push(Line::Blank);
        }
    }
    Ok(out)
}

/// awk split with separator /[ \t]+/: leading and trailing separators create
/// empty fields.
pub fn awk_split_count(s: &str) -> usize {
    if s.is_empty() {
        return 0;
    }
    let is_sep = |c: char| c == ' ' || c == '\t';
    let chars: Vec<char> = s.chars().collect();
    let mut count = 0usize;
    if is_sep(chars[0]) {
        count += 1;
    }
    let mut i = 0usize;
    while i < chars.len() {
        if is_sep(chars[i]) {
            i += 1;
            continue;
        }
        count += 1;
        while i < chars.len() && !is_sep(chars[i]) {
            i += 1;
        }
    }
    if is_sep(chars[chars.len() - 1]) {
        count += 1;
    }
    count
}

fn nest_check(line: &str) -> bool {
    let mut depth = 0i32;
    for c in line.chars() {
        if c == '(' {
            depth += 1;
            if depth > 1 {
                return true;
            }
        } else if c == ')' && depth > 0 {
            depth -= 1;
        }
    }
    false
}

fn allowed_caps(core: &str) -> bool {
    ALLOWLIST.split(' ').any(|a| a == core)
}

#[derive(Debug)]
pub struct FileScan {
    pub counts: CheckCounts,
    pub violations: Vec<String>,
}

struct Checker<'a> {
    file: &'a str,
    counts: CheckCounts,
    violations: Vec<String>,
    em_dash: Regex,
    sentence_break: Regex,
    url: Regex,
    paren_dot: Regex,
    paren_caps: Regex,
    caps_token: Regex,
    filler_word: Regex,
    hist_fix: Regex,
    hist_fixed: Regex,
    hist_audit: Regex,
    contrast: Regex,
    all_non_letters: Regex,
}

impl<'a> Checker<'a> {
    fn new(file: &'a str) -> Result<Self, String> {
        Ok(Self {
            file,
            counts: CheckCounts::default(),
            violations: Vec::new(),
            em_dash: re(r"—")?,
            sentence_break: re(r"[.!?] ")?,
            url: re(r"https?://[^ )\t]+")?,
            paren_dot: re(r"\([^()]*\.[A-Za-z0-9]+[^()]*\)")?,
            paren_caps: re(r"\([A-Z][A-Z0-9_.-]{2,}\)")?,
            caps_token: re(r"[A-Z][A-Z0-9-]{3,}")?,
            filler_word: re(
                r"(^|[^A-Za-z0-9_-])(premium|seamless|robust|comprehensive|battle-tested|enterprise-grade|production-ready|best-in-class|powerful|flexible)([^A-Za-z0-9_-]|$)",
            )?,
            hist_fix: re(r"(^|[^a-z0-9_])(fix|fixes|fixing)([^a-z0-9_]|$)")?,
            hist_fixed: re(
                r"(^|[^a-z0-9_])((was|were|gets|get|been) fixed|fixed (the|this|that|a|an) )",
            )?,
            hist_audit: re(
                r"(^|[^a-z0-9_])(audit's|pre-audit|audit[ -](finding|findings|round|rounds|item|items|fix|fixes|id|ids|number|no)|audit[- ]*[^a-z0-9_][0-9]+)([^a-z0-9_]|$)",
            )?,
            contrast: re(r"— never |, not |— and not | is not ")?,
            all_non_letters: re(r"^[^A-Za-z]+|[^A-Za-z]+$")?,
        })
    }

    fn flush(&mut self, joined: &str, line_no: usize, pbold: usize) {
        if joined.is_empty() {
            return;
        }
        let t = joined.replace('\n', " ");
        let file = self.file;
        // em-dash density
        let em = self.em_dash.find_iter(&t).count();
        if em > MAX_EM {
            self.violations.push(format!(
                "{file}:{line_no}: em-dash: {em} em dashes in one paragraph (limit {MAX_EM})"
            ));
            self.counts.em += 1;
        }
        // sentence length
        let with_breaks = self.sentence_break.replace_all(&t, "\n").to_string();
        for sentence in with_breaks.split('\n') {
            if sentence.is_empty() {
                continue;
            }
            let wc = awk_split_count(sentence);
            if wc > MAX_WORDS {
                let preview: String = sentence.chars().take(70).collect();
                self.violations.push(format!(
                    "{file}:{line_no}: long-sentence: {wc} words (limit {MAX_WORDS}): {preview}"
                ));
                self.counts.long += 1;
            }
        }
        // all-caps
        let mut t2 = self.url.replace_all(&t, "").to_string();
        t2 = self.paren_dot.replace_all(&t2, "").to_string();
        t2 = self.paren_caps.replace_all(&t2, "").to_string();
        let mut s: &str = &t2;
        while let Some(m) = self.caps_token.find(s) {
            let tok = m.as_str().to_string();
            let before = s[..m.start()].chars().next_back();
            let after = s[m.end()..].chars().next();
            if after.map(|c| c.is_ascii_lowercase()).unwrap_or(false) {
                s = &s[m.start() + 1..];
                continue;
            }
            let before_hits = matches!(
                before,
                Some(':') | Some('.') | Some('_') | Some('>') | Some('$') | Some('`')
            );
            if before_hits || after == Some('`') || before == Some('`') {
                s = &s[m.end()..];
                continue;
            }
            let core: String = tok
                .trim_end_matches(|c: char| c.is_ascii_digit() || c == '-')
                .to_string();
            if core.chars().count() >= 4 && !allowed_caps(&core) {
                self.violations
                    .push(format!("{file}:{line_no}: all-caps: '{tok}'"));
                self.counts.caps += 1;
            }
            s = &s[m.end()..];
        }
        // filler vocabulary
        let lw = t.to_lowercase();
        if lw.contains("not merely") {
            self.violations
                .push(format!("{file}:{line_no}: filler: 'not merely'"));
            self.counts.fill += 1;
        }
        if lw.contains("the property you can rely on") {
            self.violations.push(format!(
                "{file}:{line_no}: filler: 'the property you can rely on'"
            ));
            self.counts.fill += 1;
        }
        for m in self.filler_word.find_iter(&lw) {
            let word = self.all_non_letters.replace_all(m.as_str(), "").to_string();
            self.violations
                .push(format!("{file}:{line_no}: filler: '{word}'"));
            self.counts.fill += 1;
        }
        // remediation-history wording
        for m in self.hist_fix.find_iter(&lw) {
            let word = self.all_non_letters.replace_all(m.as_str(), "").to_string();
            self.violations
                .push(format!("{file}:{line_no}: hist-ref: '{word}'"));
            self.counts.hist += 1;
        }
        for _ in self.hist_fixed.find_iter(&lw) {
            self.violations
                .push(format!("{file}:{line_no}: hist-ref: 'fixed'"));
            self.counts.hist += 1;
        }
        for m in self.hist_audit.find_iter(&lw) {
            let word = self.all_non_letters.replace_all(m.as_str(), "").to_string();
            self.violations
                .push(format!("{file}:{line_no}: hist-ref: '{word}'"));
            self.counts.hist += 1;
        }
        // contrast saturation
        let n = self.contrast.find_iter(&t).count();
        if n > MAX_CONTRAST {
            self.violations.push(format!(
                "{file}:{line_no}: contrast: {n} never/not constructions in one paragraph (limit {MAX_CONTRAST})"
            ));
            self.counts.contrast += 1;
        }
        // bold density
        if pbold / 2 > MAX_BOLD {
            self.violations.push(format!(
                "{file}:{line_no}: bold: {} bold spans in one paragraph (limit {MAX_BOLD})",
                pbold / 2
            ));
            self.counts.bold += 1;
        }
    }
}

/// Run the paragraph checks over one processed stream.
pub fn run_checks(lines: &[Line], file: &str) -> Result<FileScan, String> {
    let mut checker = Checker::new(file)?;
    let mut pbuf = String::new();
    let mut pline = 0usize;
    let mut pbold = 0usize;
    for (i, line) in lines.iter().enumerate() {
        let nr = i + 1;
        match line {
            Line::Blank => {
                let joined = std::mem::take(&mut pbuf);
                checker.flush(&joined, pline, pbold);
                pline = 0;
                pbold = 0;
            }
            Line::Text(content, nb) => {
                if pline == 0 {
                    pline = nr;
                }
                pbold += nb;
                if !pbuf.is_empty() {
                    pbuf.push('\n');
                }
                pbuf.push_str(content);
                if nest_check(content) {
                    checker.violations.push(format!(
                        "{file}:{nr}: nested-parens: parentheses nested more than one level"
                    ));
                    checker.counts.nest += 1;
                }
            }
        }
    }
    checker.flush(&pbuf, pline, pbold);
    Ok(FileScan {
        counts: checker.counts,
        violations: checker.violations,
    })
}

/// Normalized sentences for the duplicate pass.
pub fn dup_sentences(lines: &[Line]) -> Result<Vec<String>, String> {
    let sentence_break = re(r"[.!?] ")?;
    let edge = re(r"^[^A-Za-z0-9]+|[^A-Za-z0-9]+$")?;
    let non_alnum = re(r"[^A-Za-z0-9]+")?;
    let multi_space = re(r"  +")?;
    let digits = re(r"^[0-9]+$")?;
    let mut out = Vec::new();
    let mut pbuf = String::new();
    let mut flush = |pbuf: &mut String| -> Result<(), String> {
        if pbuf.is_empty() {
            return Ok(());
        }
        let t = pbuf.replace('\n', " ");
        let with_breaks = sentence_break.replace_all(&t, "\n").to_string();
        for sentence in with_breaks.split('\n') {
            let s = edge.replace_all(sentence, "").to_string();
            if s.is_empty() {
                continue;
            }
            let lower = s.to_lowercase();
            let spaced = non_alnum.replace_all(&lower, " ").to_string();
            let trimmed = spaced.trim_matches(' ').to_string();
            let n = multi_space.replace_all(&trimmed, " ").to_string();
            if n.is_empty() || digits.is_match(&n) {
                continue;
            }
            out.push(n);
        }
        pbuf.clear();
        Ok(())
    };
    for line in lines {
        match line {
            Line::Blank => flush(&mut pbuf)?,
            Line::Text(content, _) => {
                if !pbuf.is_empty() {
                    pbuf.push('\n');
                }
                pbuf.push_str(content);
            }
        }
    }
    flush(&mut pbuf)?;
    Ok(out)
}

#[derive(Debug, Default)]
pub struct Baseline {
    pub rows: BTreeMap<String, usize>,
    pub total: usize,
}

pub fn parse_baseline(text: &str) -> Baseline {
    let mut baseline = Baseline::default();
    let mut current_total: Option<usize> = None;
    for line in text.lines() {
        let trimmed = line.trim_end();
        if let Some(rest) = trimmed.strip_prefix("TOTAL") {
            if rest.starts_with(' ') || rest.starts_with('\t') {
                current_total = rest.trim().parse::<usize>().ok();
                continue;
            }
        }
        let mut parts = trimmed.splitn(2, ' ');
        if let (Some(count), Some(path)) = (parts.next(), parts.next()) {
            if let Ok(n) = count.parse::<usize>() {
                let path = path.trim_start_matches(' ').trim_end();
                if !path.is_empty() {
                    baseline.rows.insert(path.to_string(), n);
                }
            }
        }
    }
    baseline.total = current_total.unwrap_or(0);
    baseline
}

/// Baseline ratchet failures: overages per row and total, stale rows noted.
pub fn ratchet_failures(
    scanned: &BTreeMap<String, usize>,
    baseline: &Baseline,
) -> (Vec<String>, Vec<String>) {
    let mut failures = Vec::new();
    for (path, count) in scanned {
        let allowed = baseline.rows.get(path).copied().unwrap_or(0);
        if *count > allowed {
            failures.push(format!("{path}: {count} (overage {})", count - allowed));
        }
    }
    let mut stale = Vec::new();
    for (path, allowed) in &baseline.rows {
        if !scanned.contains_key(path) {
            stale.push(format!("{path} (row {allowed})"));
        }
    }
    (failures, stale)
}

#[derive(Debug)]
pub struct DocsLintArgs {
    pub source: bool,
    pub baseline: Option<String>,
    pub update_baseline: Option<String>,
}

pub fn parse_args(args: &[String]) -> Result<DocsLintArgs, String> {
    let mut out = DocsLintArgs {
        source: false,
        baseline: None,
        update_baseline: None,
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--source" => out.source = true,
            "--baseline" => {
                i += 1;
                out.baseline = args.get(i).cloned();
                if out.baseline.is_none() {
                    return Err("docs-lint: --baseline requires a file".into());
                }
            }
            "--update-baseline" => {
                i += 1;
                out.update_baseline = args.get(i).cloned();
                if out.update_baseline.is_none() {
                    return Err("docs-lint: --update-baseline requires a file".into());
                }
            }
            _ => {}
        }
        i += 1;
    }
    Ok(out)
}

fn product_root_readme(repo: &Repo) -> bool {
    match repo.read("README.md") {
        Ok(text) => text.lines().next() == Some("# ApexMediation Platform"),
        Err(_) => false,
    }
}

fn scan_files(repo: &Repo, extensions: &[&str]) -> Vec<PathBuf> {
    let mut out: BTreeSet<PathBuf> = BTreeSet::new();
    let woodpecker_ok = product_root_readme(repo);
    for root in SCAN_ROOTS {
        if *root == ".woodpecker" && !woodpecker_ok {
            continue;
        }
        let dir = repo.path(root);
        if !dir.is_dir() {
            continue;
        }
        for file in crate::util::walk_files(&dir, &[]) {
            let name = file
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let ext_ok = file
                .extension()
                .map(|e| extensions.iter().any(|x| x == &e.to_string_lossy()))
                .unwrap_or(false);
            let md_ok = extensions.contains(&"md") && name.ends_with(".md");
            if !ext_ok && !md_ok {
                continue;
            }
            let path_str = crate::util::slash(&file);
            if EXCLUDED_FRAGMENTS.iter().any(|f| path_str.contains(f)) {
                continue;
            }
            out.insert(file);
        }
    }
    if woodpecker_ok && extensions.contains(&"md") {
        let readme = repo.path("README.md");
        if readme.is_file() {
            out.insert(readme);
        }
        let security = repo.path("SECURITY.md");
        if security.is_file() {
            out.insert(security);
        }
    }
    out.into_iter().collect()
}

/// One shell word of the scanned file list. The retired shell iterates
/// `for f in $md_files`, so a path containing whitespace is split into
/// fragments; a fragment that is not a real file still yields a zero-count
/// summary row, exactly like awk reading an empty stream. Reproducing this
/// keeps the committed baseline (which was generated under the same rule)
/// valid.
#[derive(Debug, Clone)]
struct ScanEntry {
    display: String,
    real: Option<PathBuf>,
}

fn word_split_entries(files: &[PathBuf]) -> Vec<ScanEntry> {
    let mut out = Vec::new();
    for file in files {
        let text = crate::util::slash(file);
        for word in text.split_whitespace() {
            let real = if Path::new(word).is_file() {
                Some(PathBuf::from(word))
            } else {
                None
            };
            out.push(ScanEntry {
                display: word.to_string(),
                real,
            });
        }
    }
    out
}

/// `rel()` from the shell: strip the repo root prefix when present.
fn rel_display(root: &Path, display: &str) -> String {
    let root_str = crate::util::slash(root);
    match display.strip_prefix(&format!("{root_str}/")) {
        Some(rest) => rest.to_string(),
        None => display.to_string(),
    }
}

fn lang_for(path: &Path) -> Lang {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" => Lang::Js,
        "yaml" | "yml" | "sh" => Lang::Hash,
        _ => Lang::Rs,
    }
}

pub fn docs_lint(repo: &Repo, args: &DocsLintArgs) -> Result<(), String> {
    let md_entries = word_split_entries(&scan_files(repo, &["md"]));
    let src_entries = if args.source {
        word_split_entries(&scan_files(repo, SRC_EXTENSIONS))
    } else {
        Vec::new()
    };

    println!("== docs-lint (prose lint) ==");
    println!(
        "scan root: {} ({} markdown files, {} source files)",
        crate::util::slash(&repo.root),
        md_entries.len(),
        src_entries.len()
    );

    let mut file_counts: BTreeMap<String, CheckCounts> = BTreeMap::new();
    let mut report_lines: Vec<String> = Vec::new();
    for entry in &md_entries {
        let scan = match &entry.real {
            Some(path) => {
                let text =
                    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", entry.display))?;
                run_checks(&md_prose(&text)?, &entry.display)?
            }
            None => FileScan {
                counts: CheckCounts::default(),
                violations: Vec::new(),
            },
        };
        report_lines.extend(scan.violations);
        file_counts.insert(entry.display.clone(), scan.counts);
    }
    for entry in &src_entries {
        let scan = match &entry.real {
            Some(path) => {
                let text =
                    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", entry.display))?;
                run_checks(&source_comments(&text, lang_for(path))?, &entry.display)?
            }
            None => FileScan {
                counts: CheckCounts::default(),
                violations: Vec::new(),
            },
        };
        report_lines.extend(scan.violations);
        file_counts.insert(entry.display.clone(), scan.counts);
    }

    // Duplicate pass over markdown only.
    let mut dup: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    for entry in &md_entries {
        let Some(path) = &entry.real else { continue };
        if entry.display.ends_with("Resources/ACCESSIBILITY.md") {
            continue;
        }
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", entry.display))?;
        for sentence in dup_sentences(&md_prose(&text)?)? {
            *dup.entry(sentence)
                .or_default()
                .entry(entry.display.clone())
                .or_insert(0) += 1;
        }
    }
    let mut dup_per_file: BTreeMap<String, usize> = BTreeMap::new();
    let mut dup_messages: Vec<(String, usize)> = Vec::new();
    for (sentence, files) in &dup {
        let occurrences: usize = files.values().sum();
        if occurrences <= 1 {
            continue;
        }
        let preview: String = sentence.chars().take(70).collect();
        dup_messages.push((preview, occurrences));
        let first = files.keys().next().cloned().unwrap_or_default();
        for (file, count) in files {
            let extra = if *file == first { count - 1 } else { *count };
            *dup_per_file.entry(file.clone()).or_insert(0) += extra;
        }
    }

    println!();
    for v in &report_lines {
        println!("{v}");
    }

    println!();
    println!("== duplicate sentences (markdown, normalized) ==");
    println!();
    for (preview, occurrences) in &dup_messages {
        println!("  duplicate sentence ({occurrences} occurrences): {preview}");
    }

    println!();
    let mut total_violations = 0usize;
    let mut scanned_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut summary_lines: Vec<String> = Vec::new();
    for (path, counts) in &file_counts {
        let dup_count = dup_per_file.get(path).copied().unwrap_or(0);
        summary_lines.push(format!(
            "  {}\t{}, duplicate {dup_count}",
            path,
            counts.detail()
        ));
        let c = counts.total() + dup_count;
        scanned_counts.insert(rel_display(&repo.root, path), c);
        total_violations += c;
    }
    summary_lines.sort();
    for line in &summary_lines {
        println!("{line}");
    }
    println!(
        "  TOTAL: {total_violations} violations across {} files",
        file_counts.len()
    );

    if let Some(target) = &args.update_baseline {
        let mut rows: Vec<(String, usize)> = scanned_counts
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        let mut content = String::new();
        for (path, count) in &rows {
            content.push_str(&format!("{count} {path}\n"));
        }
        content.push_str(&format!("TOTAL {total_violations}\n"));
        std::fs::write(target, content)
            .map_err(|e| format!("docs-lint.sh: cannot write baseline: {target}: {e}"))?;
        println!("docs-lint: baseline written: {target} (TOTAL {total_violations})");
        return Ok(());
    }

    if let Some(target) = &args.baseline {
        let text = std::fs::read_to_string(target).map_err(|_| {
            format!(
                "docs-lint: baseline file not found: {target} (generate with --update-baseline)"
            )
        })?;
        let baseline = parse_baseline(&text);
        let (failures, stale) = ratchet_failures(&scanned_counts, &baseline);
        if !stale.is_empty() {
            eprintln!(
                "docs-lint: note: {} stale baseline row(s) (file no longer scanned):",
                stale.len()
            );
            for s in &stale {
                eprintln!("  {s}");
            }
        }
        let mut ok = true;
        if !failures.is_empty() {
            eprintln!("docs-lint: FAIL: per-file baseline rows exceeded:");
            for f in &failures {
                eprintln!("  {f}");
            }
            ok = false;
        }
        if total_violations > baseline.total {
            eprintln!(
                "docs-lint: FAIL: total {total_violations} exceeds baseline {} ({target}); regenerate with --update-baseline only after deliberate rewrites",
                baseline.total
            );
            ok = false;
        }
        if !ok {
            return Err("docs-lint: baseline enforcement failed".into());
        }
        println!(
            "docs-lint: OK: total {total_violations} at or below baseline {}; every scanned file at or below its per-file row",
            baseline.total
        );
        return Ok(());
    }

    println!("docs-lint: advisory: total {total_violations}, no baseline given (exit 0)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan_md(md: &str) -> FileScan {
        run_checks(&md_prose(md).unwrap(), "doc.md").unwrap()
    }

    #[test]
    fn em_dash_density_is_capped_per_paragraph() {
        let ok = scan_md("a—b c—d\n\nplain paragraph\n");
        assert_eq!(ok.counts.em, 0);
        let bad = scan_md("a—b c—d e—f\n");
        assert_eq!(bad.counts.em, 1);
        assert!(bad.violations[0].contains("em-dash: 3 em dashes"));
    }

    #[test]
    fn long_sentences_report_word_count() {
        let sentence = (0..45)
            .map(|i| format!("w{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let scan = scan_md(&format!("{sentence}.\n"));
        assert_eq!(scan.counts.long, 1);
        assert!(scan
            .violations
            .iter()
            .any(|v| v.contains("long-sentence: 45 words")));
    }

    #[test]
    fn all_caps_respect_the_allowlist_and_code_glue() {
        let allowed = scan_md("Use HTTP and WCAG here.\n");
        assert_eq!(allowed.counts.caps, 0);
        let flagged = scan_md("This FOOBAR thing is CAPS.\n");
        assert_eq!(flagged.counts.caps, 2);
        assert!(flagged
            .violations
            .iter()
            .any(|v| v.contains("all-caps: 'FOOBAR'")));
        let glued = scan_md("path::FOOBAR and obj.FOOBAR here.\n");
        assert_eq!(
            glued.counts.caps, 0,
            "glued identifiers are not prose: {:?}",
            glued.violations
        );
        let short = scan_md("ABCD word.\n");
        assert_eq!(short.counts.caps, 1);
    }

    #[test]
    fn filler_vocabulary_trips_including_not_merely() {
        let scan = scan_md("A premium and robust plan is not merely flexible.\n");
        assert_eq!(scan.counts.fill, 4, "{:?}", scan.violations);
        let clean = scan_md("A plain plan works.\n");
        assert_eq!(clean.counts.fill, 0);
        let hyphen = scan_md("The battle-tested engine runs.\n");
        assert_eq!(hyphen.counts.fill, 1);
    }

    #[test]
    fn remediation_history_wording_is_flagged() {
        let scan = scan_md("This was fixed after the audit finding 12 landed.\n");
        assert!(scan.counts.hist >= 2, "{:?}", scan.violations);
        let clean = scan_md("The prefix aligns and suffixes stay.\n");
        assert_eq!(clean.counts.hist, 0, "{:?}", clean.violations);
    }

    #[test]
    fn bold_and_contrast_density_limits() {
        let bold = scan_md("**a** **b** **c** all bold here\n");
        assert_eq!(bold.counts.bold, 1);
        let contrast = scan_md("x is not y, not z, not w\n");
        assert_eq!(contrast.counts.contrast, 1);
    }

    #[test]
    fn nested_parentheses_are_flagged_per_line() {
        let scan = scan_md("The map (with a (nested) aside) lands.\n");
        assert_eq!(scan.counts.nest, 1);
        let flat = scan_md("The map (with an aside) lands.\n");
        assert_eq!(flat.counts.nest, 0);
    }

    #[test]
    fn duplicate_sentences_count_across_files() {
        let a = dup_sentences(&md_prose("Alpha beta gamma delta epsilon.\n").unwrap()).unwrap();
        let b = dup_sentences(&md_prose("Alpha beta gamma delta epsilon!\n").unwrap()).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), 1);
        let unique = dup_sentences(&md_prose("Zeta eta theta iota kappa.\n").unwrap()).unwrap();
        assert_ne!(a, unique);
    }

    #[test]
    fn baseline_enforcement_rows_only_shrink_and_stale_rows_are_reported() {
        let baseline = parse_baseline("2 docs/a.md\n1 docs/fixed.md\nTOTAL 3\n");
        assert_eq!(baseline.rows.get("docs/a.md"), Some(&2));
        assert_eq!(baseline.total, 3);
        let mut scanned: BTreeMap<String, usize> = BTreeMap::new();
        scanned.insert("docs/a.md".to_string(), 3);
        scanned.insert("docs/new.md".to_string(), 1);
        let (failures, stale) = ratchet_failures(&scanned, &baseline);
        assert!(failures
            .iter()
            .any(|f| f.contains("docs/a.md: 3 (overage 1)")));
        assert!(failures
            .iter()
            .any(|f| f.contains("docs/new.md: 1 (overage 1)")));
        assert_eq!(stale.len(), 1);
        assert!(stale[0].contains("docs/fixed.md"));
        let mut ok_scan: BTreeMap<String, usize> = BTreeMap::new();
        ok_scan.insert("docs/a.md".to_string(), 2);
        let (failures, stale) = ratchet_failures(&ok_scan, &baseline);
        assert!(failures.is_empty());
        assert_eq!(stale.len(), 1);
    }

    #[test]
    fn md_prose_masks_code_and_drops_fences_and_tables() {
        let lines =
            md_prose("```\nFOOBAR inside fence\n```\n| table FOOBAR |\nplain `FOOBAR` text\n")
                .unwrap();
        let texts: Vec<String> = lines
            .iter()
            .filter_map(|l| match l {
                Line::Text(t, _) => Some(t.clone()),
                Line::Blank => None,
            })
            .collect();
        assert_eq!(texts, vec!["plain   text".to_string()]);
    }

    #[test]
    fn source_comments_extract_line_and_block_comments() {
        let src = "let x = 1; // note here\n/* block start\n * continued\nend */\ncode();\n";
        let lines = source_comments(src, Lang::Rs).unwrap();
        let texts: Vec<String> = lines
            .iter()
            .filter_map(|l| match l {
                Line::Text(t, _) => Some(t.clone()),
                Line::Blank => None,
            })
            .collect();
        assert!(texts.contains(&"note here".to_string()));
        assert!(texts.contains(&"block start".to_string()));
        assert!(texts.contains(&"continued".to_string()));
        assert!(texts.contains(&"end".to_string()));
    }

    #[test]
    fn awk_split_count_matches_gawk_boundaries() {
        assert_eq!(awk_split_count(""), 0);
        assert_eq!(awk_split_count(" a b"), 3);
        assert_eq!(awk_split_count("a b "), 3);
        assert_eq!(awk_split_count("  "), 2);
        assert_eq!(awk_split_count("one two"), 2);
        assert_eq!(awk_split_count("a\t\tb"), 2);
    }

    #[test]
    fn parse_args_reads_source_and_baselines() {
        let args = vec![
            "--source".to_string(),
            "--baseline".to_string(),
            "b.txt".to_string(),
        ];
        let parsed = parse_args(&args).unwrap();
        assert!(parsed.source);
        assert_eq!(parsed.baseline.as_deref(), Some("b.txt"));
        assert!(parsed.update_baseline.is_none());
    }
}
