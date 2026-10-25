# 05 — Nice-to-have / roadmap

## 24. Frame-drop investigation
- Profile with MangoHud logs; identify shader-compile stalls vs. scheduler issues.

## 25. First-run shader-compile UX
- Detect missing DXVK cache; print estimated wait + progress hint.

## 26. Wayland "not responding" handling
- Increase compositor timeout guidance or auto-set `WAYLAND_DEBUG`.

## 27. Persistent play-db integration
- Re-enable `fetch_database()` in `orchestrator.rs`; wire `DbEntry` into `PlanBuilder`.

## 28. Metrics aggregation across sessions
- Extend `--review-mangohud` with trend view (avg FPS over last N runs per game).
