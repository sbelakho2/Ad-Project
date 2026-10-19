> Superseded. Describes a removed pre-pivot product. Current documentation: [docs index](../INDEX.md).

# Product Overview

**ApexMediation** is a **Bring-Your-Own (BYO)** ad mediation platform designed for mid-tier and advanced publishers who demand transparency, reliability, and full control over their revenue.

Unlike traditional mediation platforms that operate as "black boxes" with their own conflicting demand sources, ApexMediation is a neutral orchestration layer. We do not own an ad network. We do not compete with you. We simply ensure that your existing network accounts compete fairly for every impression.

## Core Features

*   **Hybrid Auction & Waterfall**: Combines real-time bidding (RTB) with traditional waterfall mediation to maximize yield from both modern and legacy networks.
*   **Full SDK Coverage**: Native support for **Android**, **iOS**, **Unity**, **Web**, and **TV** (tvOS & Android TV).
*   **Cryptographic Transparency (VRA)**: Every auction decision is cryptographically signed. You can independently verify that no bid was suppressed and no logic was tampered with.
*   **Migration Studio**: Seamlessly import your existing configurations from Unity LevelPlay, ironSource, or AppLovin MAX.
*   **BYO Model**: You keep your own network accounts. You get paid directly by the networks. We charge a transparent mediation fee based on reported revenue.

## High-Level Architecture

```mermaid
graph LR
    App[Mobile/Web App] -->|Request| SDK[Apex SDK]
    SDK -->|Auction Request| Apex[Apex Backend]
    Apex -->|Bid Request| NetworkA[AdMob (BYO)]
    Apex -->|Bid Request| NetworkB[AppLovin (BYO)]
    Apex -->|Bid Request| NetworkC[Meta (BYO)]
    NetworkA -->|Bid| Apex
    NetworkB -->|Bid| Apex
    NetworkC -->|Bid| Apex
    Apex -->|Winner + Token| SDK
    SDK -->|Render| App
```

## Who is this for?

ApexMediation is built for:
*   **CTOs and Lead Developers** who need a stable, crash-free SDK with transparent open-source components.
*   **Ad Ops Managers** who are tired of opaque "optimization" algorithms and want verifiable fairness.
*   **Publishers** migrating from Unity/ironSource/MAX who want to retain their direct network relationships.

## What this is NOT

*   **Not a DSP**: We do not buy your inventory.
*   **Not an Ad Network**: We do not sell ads.
*   **Not a Black Box**: We prove our decisions with cryptography.
