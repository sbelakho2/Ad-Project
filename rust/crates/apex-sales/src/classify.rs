//! Reading replies: bounces, auto-replies, opt-outs and human answers.
//!
//! Classification runs in three layers, most certain first:
//!
//! 1. **Transport signals** (RFC 3464 delivery reports, RFC 3834
//!    `Auto-Submitted`, `Precedence`, mailer-daemon senders) decide bounces
//!    and automatic replies with certainty.
//! 2. **Opt-out phrases** ("unsubscribe", "remove me", ...) always win: a
//!    missed opt-out is the costliest error the engine can make.
//! 3. **Multinomial naive Bayes** over unigrams and bigrams, with Laplace
//!    smoothing, separates positive, referral, question, not-now and
//!    not-interested replies. It is trained on a seed corpus and on every
//!    reply an operator labels. When the posterior of the best class is
//!    under the confidence threshold the reply is `Unclear` and goes to a
//!    person.
//!
//! Quoted history (`>` lines and everything after an "On … wrote:" line)
//! is removed first, so our own words never vote.

use crate::model::{Outcome, ReplyClass};

/// Classes the naive-Bayes layer separates.
pub const NB_CLASSES: [ReplyClass; 5] = [
    ReplyClass::Positive,
    ReplyClass::Referral,
    ReplyClass::Question,
    ReplyClass::NotNow,
    ReplyClass::NotInterested,
];

/// Seed training corpus.
pub const SEED: [(ReplyClass, &str); 33] = [
    (ReplyClass::Positive, "yes please send the audit"),
    (ReplyClass::Positive, "sounds interesting let's talk"),
    (ReplyClass::Positive, "sure happy to try the free audit"),
    (ReplyClass::Positive, "interested how do we start"),
    (ReplyClass::Positive, "please run the audit on our app"),
    (ReplyClass::Positive, "let's set up a call this week"),
    (ReplyClass::Positive, "go ahead we would like to try it"),
    (ReplyClass::Positive, "book a time with me"),
    (
        ReplyClass::Referral,
        "please contact our monetization lead instead",
    ),
    (
        ReplyClass::Referral,
        "i'm not the right person reach out to our ad ops team",
    ),
    (ReplyClass::Referral, "cc'ing my colleague who handles ads"),
    (
        ReplyClass::Referral,
        "talk to our cto he decides on mediation",
    ),
    (
        ReplyClass::Referral,
        "forwarding this to the person in charge of ads",
    ),
    (
        ReplyClass::Referral,
        "you should email our publishing manager",
    ),
    (ReplyClass::Question, "how much does it cost"),
    (
        ReplyClass::Question,
        "what access do you need to our account",
    ),
    (ReplyClass::Question, "which platforms do you support"),
    (
        ReplyClass::Question,
        "is it safe for our revenue what if it drops",
    ),
    (ReplyClass::Question, "how does the fee work exactly"),
    (
        ReplyClass::Question,
        "what is the minimum size you work with",
    ),
    (
        ReplyClass::Question,
        "do you need our sdk keys or admin access",
    ),
    (ReplyClass::NotNow, "not right now maybe next quarter"),
    (ReplyClass::NotNow, "busy at the moment check back later"),
    (ReplyClass::NotNow, "ping me again in a few months"),
    (
        ReplyClass::NotNow,
        "we are in the middle of a launch later please",
    ),
    (
        ReplyClass::NotNow,
        "not a priority this year reach out next year",
    ),
    (ReplyClass::NotNow, "timing is bad for us right now"),
    (ReplyClass::NotInterested, "not interested thanks"),
    (ReplyClass::NotInterested, "no thank you"),
    (
        ReplyClass::NotInterested,
        "we already have a solution for this",
    ),
    (
        ReplyClass::NotInterested,
        "we handle monetization in house no need",
    ),
    (
        ReplyClass::NotInterested,
        "we are happy with our current setup",
    ),
    (ReplyClass::NotInterested, "no we don't need this"),
];

/// Phrases that always mean "stop emailing me".
pub const OPT_OUT_PHRASES: [&str; 10] = [
    "unsubscribe",
    "remove me",
    "take me off",
    "stop emailing",
    "stop sending",
    "do not contact",
    "don't contact",
    "no more emails",
    "opt out",
    "abmelden",
];

/// The result of classifying a message.
#[derive(Debug, Clone, PartialEq)]
pub struct Classification {
    /// Outcome to record.
    pub outcome: Outcome,
    /// Posterior probability of the chosen class (1.0 for rule decisions).
    pub confidence: f64,
    /// Which layer decided and why.
    pub rationale: String,
}

/// A trainable naive-Bayes reply classifier.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplyClassifier {
    /// Token counts per class, as `(token, count)` sorted by token.
    counts: Vec<Vec<(String, f64)>>,
    /// Total token count per class.
    totals: Vec<f64>,
    /// Documents per class.
    docs: Vec<f64>,
    /// Vocabulary size.
    vocab: usize,
    /// Minimum posterior for a confident answer.
    pub threshold: f64,
}

/// Lowercased unigrams and bigrams of letters, digits and apostrophes.
pub fn tokens(text: &str) -> Vec<String> {
    let words: Vec<String> = text
        .to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect();
    let mut out = words.clone();
    for pair in words.windows(2) {
        out.push(format!("{} {}", pair[0], pair[1]));
    }
    out
}

impl Default for ReplyClassifier {
    fn default() -> Self {
        let mut c = Self {
            counts: vec![Vec::new(); NB_CLASSES.len()],
            totals: vec![0.0; NB_CLASSES.len()],
            docs: vec![0.0; NB_CLASSES.len()],
            vocab: 0,
            threshold: 0.6,
        };
        for (class, text) in SEED {
            c.train(class, text);
        }
        c
    }
}

impl ReplyClassifier {
    fn class_index(class: ReplyClass) -> Option<usize> {
        NB_CLASSES.iter().position(|c| *c == class)
    }

    /// Learn from one labelled reply. Classes outside [`NB_CLASSES`] are
    /// ignored.
    pub fn train(&mut self, class: ReplyClass, text: &str) {
        let Some(ci) = Self::class_index(class) else {
            return;
        };
        let toks = tokens(text);
        for t in toks {
            let known_anywhere = self
                .counts
                .iter()
                .any(|c| c.binary_search_by(|(k, _)| k.as_str().cmp(&t)).is_ok());
            if !known_anywhere {
                self.vocab += 1;
            }
            let bucket = &mut self.counts[ci];
            match bucket.binary_search_by(|(k, _)| k.as_str().cmp(&t)) {
                Ok(i) => bucket[i].1 += 1.0,
                Err(i) => bucket.insert(i, (t, 1.0)),
            }
            self.totals[ci] += 1.0;
        }
        self.docs[ci] += 1.0;
    }

    /// Posterior over [`NB_CLASSES`] for a reply body.
    pub fn posterior(&self, text: &str) -> Vec<f64> {
        let toks = tokens(text);
        let total_docs: f64 = self.docs.iter().sum();
        let v = self.vocab.max(1) as f64;
        let mut logp: Vec<f64> = (0..NB_CLASSES.len())
            .map(|ci| {
                let prior = ((self.docs[ci] + 1.0) / (total_docs + NB_CLASSES.len() as f64)).ln();
                let denom = self.totals[ci] + v;
                prior
                    + toks
                        .iter()
                        .map(|t| {
                            let count = self.counts[ci]
                                .binary_search_by(|(k, _)| k.as_str().cmp(t))
                                .map_or(0.0, |i| self.counts[ci][i].1);
                            ((count + 1.0) / denom).ln()
                        })
                        .sum::<f64>()
            })
            .collect();
        let max = logp.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let mut total = 0.0;
        for lp in logp.iter_mut() {
            *lp = (*lp - max).exp();
            total += *lp;
        }
        logp.iter().map(|p| p / total).collect()
    }

    /// Classify a full reply (headers and body, as raw RFC 5322 text, or a
    /// bare body).
    pub fn classify(&self, raw: &str) -> Classification {
        let (headers, body) = split_message(raw);
        let header = |name: &str| -> Option<String> {
            headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.to_ascii_lowercase())
        };
        let content_type = header("content-type").unwrap_or_default();
        let from = header("from").unwrap_or_default();
        if content_type.contains("report-type=delivery-status")
            || from.contains("mailer-daemon")
            || from.contains("postmaster@")
        {
            let lower = body.to_ascii_lowercase();
            let permanent = lower
                .lines()
                .filter_map(|l| l.trim().strip_prefix("status:"))
                .any(|s| s.trim().starts_with('5'))
                || (!lower.contains("status:") && !lower.contains("delayed"));
            return Classification {
                outcome: if permanent {
                    Outcome::HardBounce
                } else {
                    Outcome::SoftBounce
                },
                confidence: 1.0,
                rationale: "delivery status report".into(),
            };
        }
        let auto = header("auto-submitted").is_some_and(|v| !v.trim().starts_with("no"))
            || header("x-autoreply").is_some()
            || header("x-autorespond").is_some()
            || header("precedence").is_some_and(|v| {
                let v = v.trim();
                v == "auto_reply" || v == "bulk" || v == "junk"
            })
            || header("subject").is_some_and(|s| {
                s.contains("out of office")
                    || s.contains("automatic reply")
                    || s.contains("abwesenheit")
            });
        if auto {
            return Classification {
                outcome: Outcome::AutoReply,
                confidence: 1.0,
                rationale: "automatic reply headers".into(),
            };
        }
        let fresh = strip_quoted(&body);
        let lower = fresh.to_lowercase();
        if let Some(p) = OPT_OUT_PHRASES.iter().find(|p| lower.contains(*p)) {
            return Classification {
                outcome: Outcome::Reply(ReplyClass::OptOut),
                confidence: 1.0,
                rationale: format!("opt-out phrase \"{p}\""),
            };
        }
        let post = self.posterior(&fresh);
        let (best, p) = post
            .iter()
            .copied()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap_or((0, 0.0));
        if p < self.threshold || fresh.trim().is_empty() {
            return Classification {
                outcome: Outcome::Reply(ReplyClass::Unclear),
                confidence: p,
                rationale: format!(
                    "best class {} at {:.0}% is under the threshold",
                    NB_CLASSES[best].as_str(),
                    p * 100.0
                ),
            };
        }
        Classification {
            outcome: Outcome::Reply(NB_CLASSES[best]),
            confidence: p,
            rationale: format!("naive Bayes {:.0}%", p * 100.0),
        }
    }
}

/// Split raw message text into unfolded headers and body. Text with no
/// header block is all body.
pub fn split_message(raw: &str) -> (Vec<(String, String)>, String) {
    let text = raw.replace("\r\n", "\n");
    let Some((head, body)) = text.split_once("\n\n") else {
        return (Vec::new(), text);
    };
    let looks_like_headers = head.lines().next().is_some_and(|l| {
        l.split_once(':')
            .is_some_and(|(k, _)| !k.is_empty() && !k.contains(' '))
    });
    if !looks_like_headers {
        return (Vec::new(), text);
    }
    let mut headers: Vec<(String, String)> = Vec::new();
    for line in head.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some(last) = headers.last_mut() {
                last.1.push(' ');
                last.1.push_str(line.trim());
            }
        } else if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    (headers, body.to_string())
}

/// Remove quoted history: `>` lines and everything from an "On … wrote:"
/// or "-----Original Message-----" line on.
pub fn strip_quoted(body: &str) -> String {
    let mut out = Vec::new();
    for line in body.lines() {
        let t = line.trim();
        let lower = t.to_ascii_lowercase();
        if (lower.starts_with("on ") && lower.ends_with("wrote:"))
            || lower.starts_with("-----original message")
            || lower.starts_with("from: ")
        {
            break;
        }
        if t.starts_with('>') {
            continue;
        }
        out.push(line);
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_signals_decide_bounces_and_autoreplies() {
        let c = ReplyClassifier::default();
        let hard = "From: Mail Delivery System <MAILER-DAEMON@mx.example>\r\nContent-Type: multipart/report; report-type=delivery-status\r\n\r\nStatus: 5.1.1\r\nuser unknown\r\n";
        assert_eq!(c.classify(hard).outcome, Outcome::HardBounce);
        let soft = "From: postmaster@mx.example\n\nStatus: 4.2.2\nmailbox full, delayed\n";
        assert_eq!(c.classify(soft).outcome, Outcome::SoftBounce);
        let ooo = "Auto-Submitted: auto-replied\nSubject: Out of Office\n\nI am away until Monday.";
        assert_eq!(c.classify(ooo).outcome, Outcome::AutoReply);
        let human = "Auto-Submitted: no\nSubject: Re: hi\n\nnot interested thanks";
        assert_ne!(c.classify(human).outcome, Outcome::AutoReply);
        let bulk = "Precedence: bulk\n\nhello";
        assert_eq!(c.classify(bulk).outcome, Outcome::AutoReply);
    }

    #[test]
    fn opt_outs_always_win() {
        let c = ReplyClassifier::default();
        let r = c.classify("Sounds interesting but please remove me from your list.");
        assert_eq!(r.outcome, Outcome::Reply(ReplyClass::OptOut));
        assert_eq!(r.confidence, 1.0);
    }

    #[test]
    fn naive_bayes_separates_the_classes() {
        let c = ReplyClassifier::default();
        let cases = [
            (
                "Yes, please run the free audit for us.",
                ReplyClass::Positive,
            ),
            (
                "How much does it cost and what access do you need?",
                ReplyClass::Question,
            ),
            (
                "Not right now, check back next quarter.",
                ReplyClass::NotNow,
            ),
            (
                "No thank you, we handle this in house.",
                ReplyClass::NotInterested,
            ),
            (
                "Please contact our monetization lead, she handles ads.",
                ReplyClass::Referral,
            ),
        ];
        for (text, want) in cases {
            assert_eq!(c.classify(text).outcome, Outcome::Reply(want), "{text}");
        }
        let unsure = c.classify("Hmm.");
        assert_eq!(unsure.outcome, Outcome::Reply(ReplyClass::Unclear));
        assert_eq!(c.classify("").outcome, Outcome::Reply(ReplyClass::Unclear));
    }

    #[test]
    fn quoted_history_does_not_vote_and_training_helps() {
        let c = ReplyClassifier::default();
        let reply = "No thank you.\n\nOn Fri, Oct 16, 2026 at 10:00, Aaron wrote:\n> Would a free audit be useful? Let's talk.\n";
        assert_eq!(
            c.classify(reply).outcome,
            Outcome::Reply(ReplyClass::NotInterested)
        );
        let mut c = ReplyClassifier::default();
        let text = "we use a different vendor called zorblax";
        for _ in 0..5 {
            c.train(ReplyClass::NotInterested, text);
        }
        c.train(ReplyClass::Unclear, "ignored");
        assert_eq!(
            c.classify(text).outcome,
            Outcome::Reply(ReplyClass::NotInterested)
        );
        let p = c.posterior("zorblax");
        assert!((p.iter().sum::<f64>() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn splits_headers_and_unfolds() {
        let (h, b) = split_message("Subject: a\r\n b\r\nFrom: x@y.z\r\n\r\nbody text");
        assert_eq!(h[0], ("Subject".to_string(), "a b".to_string()));
        assert_eq!(b, "body text");
        let (h, b) = split_message("just a body\n\nwith paragraphs");
        assert!(h.is_empty());
        assert!(b.starts_with("just a body"));
        let (h, _) = split_message("no blank line at all");
        assert!(h.is_empty());
        assert_eq!(strip_quoted("hi\n-----Original Message-----\nold"), "hi");
        assert_eq!(tokens("We're in!"), vec!["we're", "in", "we're in"]);
    }
}
