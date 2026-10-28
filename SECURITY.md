# Security Policy

## Scope

This repository contains the **PayRogen Solana program** (Anchor) — the on-chain
component that enforces payment splitting, escrow, dynamic gateway fees, and wallet
freezing. Security reports concerning the on-chain program logic in this repository
are in scope.

Out of scope for this repository: the off-chain gateway API, SDKs, web/admin UIs, and
infrastructure (these live in separate systems).

## Reporting a Vulnerability

**Please do NOT open a public issue for security vulnerabilities.**

Report privately through one of:

- GitHub private vulnerability reporting: use the **"Report a vulnerability"** button
  under this repository's **Security** tab (Security → Advisories → Report a vulnerability).
- Email: **security@payrogen.com**

Please include:

- A clear description of the issue and the affected instruction(s)/account(s).
- A proof-of-concept or step-by-step reproduction (devnet preferred).
- The potential impact (e.g., fund loss, unauthorized state change, DoS).
- Any suggested remediation.

## Our Commitment

- We will acknowledge your report within **3 business days**.
- We will provide an assessment and expected remediation timeline within **10 business days**.
- We will keep you informed through resolution and credit you (if you wish) once a fix
  is deployed and disclosed.

## Safe Harbor

We will not pursue or support legal action against researchers who:

- Make a good-faith effort to avoid privacy violations, data loss, or service disruption.
- Only test against **devnet** or their own deployments — **never** against mainnet funds
  or other users' accounts.
- Do not exploit a finding beyond what is necessary to demonstrate it.
- Give us reasonable time to remediate before any public disclosure.

## Supported Versions

The program is under active development. Security fixes are applied to the current
`main` branch and the latest deployed program. Please reference the exact commit hash
when reporting.
