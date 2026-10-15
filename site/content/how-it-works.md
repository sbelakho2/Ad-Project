+++
title = "How Apex works | Audit, test and verify"
description = "See how Apex reads your mediation setup, reviews supported changes, evaluates platform data and records the result."
template = "page.html"

[extra]
h1 = "A defined change. A recorded result."
eyebrow = "The process"
lede = "Apex works through your mediation account. Your app keeps its existing SDKs and ad-serving path."
cta = "Request the free audit"
og_image = "og/how-it-works.png"
+++

<ol class="steps">
  <li>
    <p class="step-title">Audit</p>
    <p>Read the account and inspect findings.</p>
  </li>
  <li>
    <p class="step-title">Review</p>
    <p>Agree the scope, settings and test rules.</p>
  </li>
  <li>
    <p class="step-title">Test</p>
    <p>Use a supported platform experiment.</p>
  </li>
  <li>
    <p class="step-title">Decide</p>
    <p>Keep, stop or wait for enough evidence.</p>
  </li>
</ol>

## 01 / Audit

The audit reads the available configuration and reports. It checks five groups of issues and returns a short list of findings with their evidence. Nothing is changed during the audit. A missing input is identified rather than filled in with an assumption.

## 02 / Review

A proposed change names the setting, affected app or ad unit, previous value and proposed value. You can inspect the reason, test allocation, analysis window and stop conditions before approving it. New accounts begin with approval required.

## 03 / Test

For a supported action, Apex creates a platform-native experiment with a control and a test configuration. The control provides the comparison. The registered scope and settings remain fixed during the test. Write access is enabled only for verified operations in the deployment.

## 04 / Decide

Apex evaluates the finalized platform data under the registered decision rule. A qualifying result can be kept. A failed test is stopped without a lift fee. Incomplete data means waiting; some conditions invalidate the test. The result and relevant actions are recorded in the signed change log.

## If a restore fails

The product reports the failed operation and requests intervention. It does not label the settings restored until the platform confirms the outcome. The history retains the failure and the action taken to resolve it.

## Availability

Read access is supported for MAX, LevelPlay and AdMob. The current production runner is MAX-specific. Available tests and changes depend on the platform, credentials and verified operations. We confirm that scope before enabling writes.

## See what your account needs first.

<a class="button" href="/free-audit/">Request the free audit</a>
