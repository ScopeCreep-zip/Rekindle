# Architectural Decision Records (ADRs)

This directory holds **MADR 4.0** Architectural Decision Records for
Rekindle. ADRs are short, append-only records of architecturally
significant choices: what was decided, why, what alternatives were
considered, and what consequences follow.

## How to read

Pick an ADR by filename — they are numbered chronologically. The
filename is `NNNN-short-title.md`.

| ID | Title | Status |
|----|-------|--------|
| [0001](0001-veilid-as-transport.md) | Adopt Veilid as the sole transport substrate | Accepted |
| [0002](0002-signal-protocol-for-1to1.md) | Use the Signal Protocol for 1:1 friend messaging | Accepted |
| [0003](0003-flat-smpl-governance.md) | Flat SMPL governance replaces the v1.0 coordinator model | Accepted (supersedes the v1.0 design) |
| [0004](0004-tauri-2-frontend.md) | Use Tauri 2 + SolidJS as the desktop frontend | Accepted |
| [0005](0005-daemon-cli-track.md) | Add a daemon + CLI track alongside the Tauri desktop app | Accepted |

## How to write a new ADR

1. Copy [`0001-veilid-as-transport.md`](0001-veilid-as-transport.md) to
   `NNNN-short-title.md` where `NNNN` is the next number.
2. Fill in **Context**, **Decision drivers**, **Considered options**,
   **Decision outcome**, **Consequences**, and **More information**.
3. **Do not edit accepted ADRs.** If a decision changes, write a new
   ADR that supersedes it and add a `Superseded by [NNNN](...)` line
   to the old one.
4. Keep ADRs short — most should fit comfortably under 200 lines.

## Status values

- **Proposed** — under review
- **Accepted** — current architecture follows this ADR
- **Deprecated** — no longer recommended; legacy code may still rely on it
- **Superseded by NNNN** — replaced; see ADR NNNN

## Why ADRs

Documentation explains *what* the system does. ADRs explain *why* it
got that way. New contributors deserve to know the reasoning behind a
choice — particularly the alternatives that were rejected — without
having to read every PR thread.

## More information

- [MADR 4.0 template](https://github.com/adr/madr/tree/develop/template)
- [adr.github.io](https://adr.github.io/) — Architectural Decision Records
- [matklad on architecture-md](https://matklad.github.io/2021/02/06/ARCHITECTURE.md.html)
- [AWS — Master ADRs best practices](https://aws.amazon.com/blogs/architecture/master-architecture-decision-records-adrs-best-practices-for-effective-decision-making/)
