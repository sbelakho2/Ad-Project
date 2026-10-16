//! # apex-sales
//!
//! ApexMediation's outbound sales engine: it finds mobile app publishers,
//! decides who to email, when, with which message and when to stop, sends
//! through the local mail transport, reads the replies and learns from every
//! outcome. It is an internal tool owned and run by the operator: publishers
//! never see it, and it is never offered or sold.
//!
//! It follows the architecture of a mature outbound engine (one decision
//! gate for every send, a durable event history, evidence-grounded copy,
//! sender-health breakers, calibrated scores, champion/challenger testing)
//! and replaces hand-set heuristics with explicit statistical models:
//!
//! | Question | Model | Module |
//! |---|---|---|
//! | Who is likely to be interested? | Bayesian logistic regression, Laplace-approximated online updates (Chapelle & Li, 2011); probit-corrected predictions; exact log-odds reason codes | [`scoring`] |
//! | What is an interested reply worth? | Beta–Binomial funnel with empirical-Bayes pooling across size bands; value-to-go imputation of delayed rewards | [`funnel`] |
//! | Who goes first? | Expected value on a Thompson draw: `p̃ · s_k · V − cost − c_k · C` | [`engine`] |
//! | Which message? | Beta Thompson sampling on revenue value, top-two sampling (Russo, 2016), discounting, complaint-safety constraint | [`bandit`] |
//! | When to send, when to stop? | Pooled Beta posteriors per local hour; per-step hazards with an expected-value stopping rule | [`cadence`] |
//! | Is the domain safe? | Bayesian rate tests plus a Bernoulli cumulative-sum (CUSUM) test, Page 1954; additive-increase warm-up | [`health`] |
//! | Did the emails cause the customers? | Randomized holdout, Newcombe interval for the difference | [`holdout`] |
//! | Are the probabilities honest? | Brier, log loss, reliability bins with Jeffreys intervals, isotonic recalibration | [`stats`], [`ops::report`] |
//! | Same publisher twice? | Fellegi–Sunter record linkage with Jaro–Winkler | [`dedup`] |
//! | What did they answer? | RFC 3464/3834 signals, opt-out rules, multinomial naive Bayes | [`classify`] |
//! | Which mediation platform? | Naive Bayes over `app-ads.txt` `DIRECT` lines, learned from declared platforms | [`signals`] |
//!
//! Hard rules are never traded against expected value: suppression, the
//! legal table, account coordination, the holdout, the domain breaker, the
//! daily cap, grounding and the complaint-safety rule all live in
//! [`decision::decide`], which runs at planning time and again at send time.
//!
//! [`simulate`] runs the production engine against a synthetic market whose
//! true parameters differ from the priors. With a list larger than daily
//! capacity it produces several times the expected revenue of naive
//! outreach on the same sends; with a list small enough to email everyone,
//! the gain is smaller (better messages and stopping only).
//!
//! The crate has no dependencies beyond `std`: HTTP goes through the system
//! `curl`, mail through the system `sendmail`, state through JSON-lines
//! files.
#![forbid(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod bandit;
pub mod cadence;
pub mod calendar;
pub mod classify;
pub mod cli;
pub mod compose;
pub mod decision;
pub mod dedup;
pub mod engine;
pub mod funnel;
pub mod health;
pub mod holdout;
pub mod io;
pub mod json;
pub mod model;
pub mod ops;
pub mod policy;
pub mod rng;
pub mod scoring;
pub mod signals;
pub mod simulate;
pub mod stats;
pub mod store;
