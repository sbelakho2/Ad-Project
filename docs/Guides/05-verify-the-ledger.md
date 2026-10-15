# Verify the ledger

The ledger is the record of what was done, when and why. It is signed and
hash chained, and you can verify it without our servers.

## The shape

Each record carries its sequence number, the hash of the previous record, and
a signature over the canonical bytes. Changing any earlier record breaks every
later hash, so a tampered history cannot verify.

## Verify it yourself

The ledger ships with the engine source. Open the file with your publisher key
and the chain is checked line by line. A verification failure reports the
offending line and serves no records from it.

## Re-run the fee

The re-run kit turns your own dashboard export into the same lift and lower
bound the engine computed, so you can check the fee directly. It runs in
python, with no toolchain to install. See the kit shipped next to the ledger.

## Where it lives

One ledger file per publisher, appended only. The console shows it over http
with the publisher token. Access is yours, at no cost, at any time.
