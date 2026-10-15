# Connecting platforms

Apex reads and writes through the platform management apis you already have.
There is no sdk and no change to your app.

## AppLovin MAX

You give a management key for reads and writes, and a report key for the
revenue reporting api. The report key is a separate credential from the
management key.

## LevelPlay

You give the secret key and the refresh token. Apex exchanges them for a short
lived bearer token, caches it, and refreshes it before it expires.

## AdMob

You give an oauth token limited to the mediation scope, and the publisher id.
Mediation group updates are read, modified and written back as a full list, so
a stale view can never drop a mapping.

## Keys and safety

Keys are stored encrypted with the vault key, one file per publisher. A ledger
read over http needs that publisher token, and a publisher without a token is
refused rather than served. Rotate any key that was ever committed.
