<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Security Policy

## Supported versions

Before Luminate's first stable release, security fixes are made against the
latest development version and may not be backported to older snapshots. Once
the project publishes supported release lines, this section will identify them
explicitly.

## Reporting a vulnerability

Please do not disclose a suspected vulnerability in a public issue, discussion,
pull request, log, or packet capture.

Use the repository host's private **Report a vulnerability** form when it is
available. If no private reporting form is available, open a public issue that
asks the maintainer to establish private contact, but include no vulnerability
details, affected versions, reproduction steps, logs, or exploit information in
that issue.

A useful private report includes:

- the affected component and version or commit;
- the expected and observed behaviour;
- the security impact and realistic threat model;
- minimal reproduction steps or a proof of concept;
- relevant configuration, platform, hardware, and firmware details; and
- any suggested mitigation, disclosure constraints, or credit preference.

Remove credentials, personal information, network identifiers, and unrelated
device data. Encrypt especially sensitive supporting material if requested by
the maintainer after private contact is established.

## What happens after a report

The maintainer will work privately with the reporter to validate the issue,
identify affected versions, prepare a fix and tests, and coordinate disclosure.
Please keep the details private while the issue is investigated and downstream
users have time to receive a fix. Any published advisory will follow the
reporter's credit preference.

The project does not currently operate a bug-bounty programme and cannot
promise a particular response or remediation deadline.

## Responsible research

Test only systems, accounts, networks, and hardware you own or are explicitly
authorized to assess. Minimize access to data and disruption to devices or
services. Do not retain, alter, or publish data beyond what is necessary to
demonstrate the issue, and stop testing if it risks physical damage, persistent
device changes, or impact to another person.

Dependency-only reports that contain no Luminate-specific exploitability may be
filed publicly as routine maintenance. If an affected dependency is reachable
through Luminate in a way that creates a credible security impact, use the
private process above.
