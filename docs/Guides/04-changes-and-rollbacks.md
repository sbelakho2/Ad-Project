# Changes and rollbacks

A change is a single edit to one setting, proposed from a finding, tested by
the platform experiment, and kept or rolled back by the ledger.

## Pre-registration

Before the first platform call, the change is written to the ledger as a
pre-registration. It pins the setting, the prior value, the split and the
analysis window. The test can only be judged against what it pre-registered.

## Approval mode

A new account starts in approval mode for thirty days. Every proposal is shown
but nothing is written until you approve it. After that you can keep approval
on, or let the autopilot run within the caps.

## The decision

The platform experiment decides. A kept change is promoted to all users. A
rollback, or a void from a guardrail, deprecates the experiment. Fill is a
guardrail: a test stops when fill drops, whatever the revenue says.

## Rollbacks cost nothing

A change that fails, or that a guardrail stops, is never billed. Only a kept
change with a positive lower bound enters the monthly invoice, and the amount
is the lower bound, not the point estimate.
