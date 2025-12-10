# Public Repository Synchronization Guide

This guide explains how to synchronize the private `Ad-Project` monorepo with the public `apex-mediation-platform` repository.

## Overview

We maintain a strict separation between our internal monorepo (containing backend code, infrastructure, and internal docs) and the public repository (containing only SDKs, integration docs, and examples).

**Private Repo**: `https://github.com/sbelakho2/Ad-Project` (Origin)
**Public Repo**: `https://github.com/sbelakho2/apex-mediation-platform`

## Setup Requirements

To enable the synchronization workflow, you must configure a GitHub Secret in the **Private Repository**.

1.  **Generate a Personal Access Token (PAT)**:
    *   Go to GitHub Settings > Developer settings > Personal access tokens > Tokens (classic).
    *   Generate a new token with the `repo` scope (Full control of private repositories).
    *   Copy the token.

2.  **Add Secret to Private Repo**:
    *   Go to `Ad-Project` > Settings > Secrets and variables > Actions.
    *   Click **New repository secret**.
    *   **Name**: `PUBLIC_REPO_TOKEN`
    *   **Value**: [Paste your PAT here]
    *   Click **Add secret**.

## How to Sync

The synchronization is **manual** (`workflow_dispatch`) to prevent accidental leaks of internal code.

1.  Go to the **Actions** tab in the `Ad-Project` repository.
2.  Select the **Sync Public Repo** workflow from the sidebar.
3.  Click **Run workflow**.
4.  (Optional) Enter a custom commit message.
5.  Click **Run workflow**.

## What Gets Synced?

The workflow automatically filters and copies only the following artifacts:

| Source (Private) | Destination (Public) | Description |
| :--- | :--- | :--- |
| `sdk/` | `sdk/` | All SDK source code (Android, iOS, Unity, Web, TV). |
| `docs/Customer-Facing/` | `docs/` | Public integration guides. |
| `Test Apps/` | `examples/` | Sample applications. |
| `CHANGELOG.md` | `CHANGELOG.md` | Release history. |
| `docs/Public/README.md` | `README.md` | The public-facing README. |

**Note**: Build artifacts (`.build`, `build/`) are automatically stripped during the sync process.
