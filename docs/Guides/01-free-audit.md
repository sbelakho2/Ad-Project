# The free audit

The audit reads your mediation account and finds money left on the table. It is
read only, it needs no contract, and it runs before anything is billed.

## What you give us

One key set per platform, read only. For AppLovin MAX that is the management
key and the report key. For LevelPlay that is the secret key, the refresh
token and the app key. For AdMob that is an oauth token limited to the
mediation scope.

## What you get

A short list of findings, each with the exact numbers behind it. A stale floor,
a network missing from your app-ads.txt, a reporting gap between the platform
and the mediation report. Every finding names the ad unit and the country it
concerns.

## What it does not do

It does not change anything. It reads your account through the platform keys
you supplied and returns a report. No write call is made, and the keys can be
revoked the moment you are done.

## Next

Read [what each finding means](02-reading-findings.md), or connect the first
platform in [connecting platforms](03-connecting-platforms.md).
