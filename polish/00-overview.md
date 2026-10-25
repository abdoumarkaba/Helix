# Helix — "Pro Finish" Polish Backlog

Assessment of what remains for a certified, professional-grade release.
No code changes made for this document set.

many files need removal, cost guide, workflows, etc for final release
## Index

- `01-correctness.md` — latent bugs that make the tool look broken (#1–#6)
- `02-detection.md` — detection gaps blocking the "zero-config" promise (#7–#10)
- `03-ux.md` — user-facing polish (#11–#17)
- `04-quality.md` — engineering / CI / tests (#18–#23)
- `05-roadmap.md` — nice-to-have / future (#24–#28)

## Priority order

1. Fix silent bugs #2 (whitelist) and #1 (`--yes`) — make the tool not look broken.
2. Complete rollback manifest #3 and error propagation #4, #23.
3. Finish naming migration #11 — biggest "amateurish" signal.
4. Make Clippy green #6 so CI is meaningful.
5. Ship Steam integration #12 — the feature that makes it actually useful.
6. Add LICENSE + CONTRIBUTING + play-db scaffold #18–#20.
7. Detection enhancements #7–#10.
8. Quality/UX items #13–#17, #21–#22.
9. Roadmap #24–#28 as time permits.
