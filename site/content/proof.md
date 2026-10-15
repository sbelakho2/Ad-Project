+++
title = "Apex proof | Trace the data, decision and fee"
description = "Inspect the platform inputs, versioned calculation and signed records behind an Apex result. Download the rerun kit to check compatible exports."
template = "page.html"

[extra]
h1 = "Follow the result to its source."
lede = "You should be able to see what changed, which data was used and how a fee was calculated. Apex keeps those steps connected."
note = "Platform data. Registered rules. A signed record. Enough detail to check the fee yourself."
og_image = "og/proof.png"
+++

<ol class="steps evidence">
  <li>
    <p class="step-title">Source data</p>
    <p>Final platform report</p>
    <span class="step-meta">Inspect</span>
  </li>
  <li>
    <p class="step-title">Decision</p>
    <p>Versioned calculation</p>
    <span class="step-meta">Inspect</span>
  </li>
  <li>
    <p class="step-title">Fee</p>
    <p>Reproducible inputs</p>
    <span class="step-meta">Inspect</span>
  </li>
</ol>

## Start with the platform

Experiments and reporting live in the platform account. The evidence identifies the experiment, scope, reporting window and control/test data used. Open the experiment in the platform to compare it with your own account.

## See the calculation

Apex applies a registered measurement and decision rule to the source data. The result distinguishes the observed estimate from the conservative bound used for billing. The engine version and relevant rules belong in the export, so the same inputs can be checked again.

## Inspect the history

The signed change log records relevant changes, experiment events, decisions and billing proofs. Records are chained and verified before the history is shown. A verification failure is visible and stops the unverified history from being served.

## Keep your evidence

Download the available audit report, signed ledger, experiment references and fee inputs. The rerun kit helps reproduce the measurement from a compatible data export. It does not need access to your platform credentials.

<a class="button" href="/rerun-kit/README.md">Download the rerun kit</a>

## Know what verification means

Signatures and hashes make altered records detectable. They establish integrity of the recorded history. They do not independently certify the completeness or accuracy of the platform data. Those inputs, dates and limitations are shown alongside the calculation.

## Start with a finding you can inspect.

<a class="button" href="/free-audit/">Request the free audit</a>
