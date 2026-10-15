+++
title = "Analytics projections | Apex guides"
description = "What customer_daily, platform_daily, lifecycle_cohorts and operator_queues hold, and how as-of versions, freshness and dedup work."
template = "guide.html"
weight = 6

[extra]
h1 = "Analytics projections"
lede = "The read models built from source facts."
og_image = "og/guides.png"

[extra.prev]
title = "Verify the record"
url = "/guides/verify-the-record/"
+++

The analytics store keeps facts and projections apart. Facts are the stored
events and report days; the four projections are the read models the console
and the operator plane query. A projection is always rebuilt from source
facts, never edited in place.

## What each projection holds

| Projection | One row per | Holds |
|---|---|---|
| `customer_daily` | publisher, day, platform | Revenue in integer micros with its currency, impressions when every contributing source row supplied them, contributing source days, and the count of final and provisional days. |
| `platform_daily` | platform, day | Revenue in micros with its currency, impressions, and the distinct accounts with final or only provisional reporting that day. |
| `lifecycle_cohorts` | cohort month, publisher, stage | Accounts in the cohort at each lifecycle stage. |
| `operator_queues` | queue entry | Queue name (`incidents`, `due_decisions`, `stale_accounts`, `overdue_jobs`, `held_fees`), the object, its publisher, severity, age in seconds, owner when assigned, and the next action class. |

## As-of versioning

Every rebuild computes all rows in Rust from the stored source facts and
writes them under a new as-of version. The rebuild totals are compared with
the stored projections before readers switch; a mismatch refuses the switch
instead of publishing the new version. A backfill carries a batch ID and
changes the as-of version, so replayed or corrected history never
masquerades as new customer activity.

Readers see one version at a time. The store records the current as-of
version per projection and queries it, so a partially written rebuild is
never read.

## Freshness and finality

A report day is final once the contributing sources agree on it; until then
it counts as provisional. Projections keep both counts, so a page can say
how much of a number rests on provisional days. The operator queue marks
accounts as stale when no stage movement was seen within the review window.
That window is fourteen days by default and tunable per deployment. Queue
rows carry their age so an operator can see how long a condition has held.

When a source cannot be read, the readers report the missing or unreachable
state with its reason. A missing number is never rendered as zero.

## Deduplication and replay

Facts deduplicate on the stable source and event key `(source, event_id)`.
The store is a replacing table ordered by that key and readers apply
read-your-writes semantics, so a replayed line collapses into the original
row. This makes the access-log ingest safe to rerun: re-reading the whole
log file derives the same event identity per line.

## Money

Every amount is an integer in micros with an explicit currency, from the
first fact to the rendered page. A projection never infers a currency from
an unlabelled integer.
