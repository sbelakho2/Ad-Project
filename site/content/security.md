+++
title = "Apex security | Access and data boundaries"
description = "Understand read-only audits, separate write permission, encrypted platform credentials and verified change records."
template = "page.html"

[extra]
h1 = "Clear access. Defined boundaries."
eyebrow = "Access and data"
lede = "Apex uses platform APIs to inspect configuration and reports. It does not add code to your app or become part of the ad-serving path."
note = "The audit reads your account. Write access is a separate decision. Ad requests keep their existing path."
og_image = "og/security.png"

[[extra.panels]]
title = "App and ad traffic"
lines = ["Stay on the existing serving path"]

[[extra.panels]]
title = "Configuration and reports"
lines = ["Read through platform APIs"]

[[extra.panels]]
title = "Platform credentials"
lines = ["Encrypted in the account vault"]

[[extra.panels]]
title = "Signed records"
lines = ["Verified before display"]
+++

## Read access comes first

The audit makes no platform changes. We confirm the keys and account scope needed for your platform. Do not send platform credentials through the audit-request form or by email.

## Write access is separate

Applying changes requires the relevant platform permission and a verified operation in the deployment. Approval settings and critical findings can block a change. You can review the proposed scope and settings before deciding whether to proceed.

## Credentials and records

Stored platform credentials are encrypted with AES-256-GCM. Signed change-log records are checked before display. Access to one publisher's records does not grant permission to read another publisher's history.

## Data Apex handles

Account and ad-unit identifiers, configuration, platform reporting, experiment data, audit findings and contact details used to arrange the service. Relevant records may be retained for verification and billing. Ad requests continue through the publisher's existing serving setup.

## Stopping access

You can revoke platform credentials in the platform account. Contact us to arrange service closure and the available evidence export. Revoking access may prevent further checks or restoration, so we will explain the status of any running work.

## Ask before you share access.

Contact [hello@apexmediation.ee](mailto:hello@apexmediation.ee) with your platform and security question. Leave credentials out of the message.
