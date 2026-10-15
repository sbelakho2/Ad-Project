+++
title = "Connecting platforms | Apex guides"
description = "MAX uses a management key and a separate reporting key, saved secrets are encrypted, and a read connection does not authorize writes."
template = "guide.html"
weight = 3

[extra]
h1 = "Connecting platforms"
lede = "Connect the account you control."
og_image = "og/guides.png"

[extra.prev]
title = "Reading findings"
url = "/guides/reading-findings/"

[extra.next]
title = "Changes and rollbacks"
url = "/guides/changes-and-rollbacks/"
+++

MAX uses a management key and a separate reporting key. LevelPlay uses the relevant app key, secret and refresh token. AdMob uses the account identifier and authorized token supported by the deployment.

Use the secure connection form. Saved secrets are encrypted and are not shown again. A successful read connection does not authorize writes.

The connection page shows read access, reporting health and action availability separately. If credentials expire, reconnect before expecting new reporting. Revoke access in the platform account when it is no longer needed.

Next: [Review a change.](/guides/changes-and-rollbacks/)
