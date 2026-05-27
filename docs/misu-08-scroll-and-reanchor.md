# MISU-08: scroll + sync + 조건부 lock/unlock 3중 중복 → `scroll_viewport_and_sync` + `reanchor_viewport_lock`

## 현황

**파일**: `crates/daruda_terminal/src/view/mouse.rs`

scroll → sync → 조건부 lock/unlock 4단계 블록이 3곳에 중복되어 있다.

```rust
// on_mouse_down 스크롤바 트랙 클릭 (151-162)
let _ = self.session.scroll_viewport(delta);
self.sync_viewport_scroll_tracking();
let new_offset = self.session.viewport_row_offset();
if new_offset + rows >= total {
    self.state.viewport_lock.unlock();
} else {
    self.state.viewport_lock.lock(self.session.viewport_top_abs_y());
}
self.refresh_viewport();
cx.notify();

// on_mouse_move 썸 드래그 (487-499) — 위와 문자 그대로 동일
let _ = self.session.scroll_viewport(delta);
self.sync_viewport_scroll_tracking();
let new_offset = self.session.viewport_row_offset();
if new_offset + rows >= total {
    self.state.viewport_lock.unlock();
} else {
    self.state.viewport_lock.lock(self.session.viewport_top_abs_y());
}
self.refresh_viewport();
cx.notify();

// autoscroll_poll_with_pos (642-657) — refresh 없이 동일 4단계
let _ = self.session.scroll_viewport(vel);
self.sync_viewport_scroll_tracking();
let vp_offset = self.session.viewport_row_offset();
let rows = self.session.rows() as u32;
let total = self.session.total_rows();
if vp_offset + rows >= total {
    self.state.viewport_lock.unlock();
} else {
    self.state.viewport_lock.lock(self.session.viewport_top_abs_y());
}
```

## 버그 경로

트랙 클릭과 썸 드래그가 현재 동기화돼 있더라도, 한쪽의 lock 분기 조건을 수정하면
다른 쪽이 조용히 틀어진다. `lock` / `unlock` 로직이 변경될 때(예: 새 ViewportLock 상태 추가)
세 곳을 모두 찾아 고쳐야 하는 구조다.

## iTerm2 참고 — 설계 방향 수정

원래 제안은 scroll + sync + 조건부 lock/unlock을 `scroll_and_reanchor` 하나로 합치는 것이었다.
iTerm2 분석 결과 이 방향을 재검토한다.

iTerm2 `PTYTextView`는 스크롤과 잠금을 독립 연산으로 유지한다:
- `lockScroll` — 항상 무조건 `setUserScroll:YES`. 위치 조건 없음.
- unlock — `mouseUp` 시 `isScrolledToBottom` 체크 후에만 `setUserScroll:NO`.
- scroll 메서드들은 lock 결정에 관여하지 않고 scroll 후 `lockScroll`만 호출.

그 결과 "scroll 후 at-bottom이면 unlock" 조건 분기가 scroll 경로에 없다.

```objc
// 스크롤 후 항상 lockScroll — 조건부 unlock 없음
[self scrollRectToVisible:aFrame];
[self cancelMomentumScroll];
[self lockScroll];  // 무조건

// unlock은 mouseUp 한 곳에서만
if ([self.mouseDelegate mouseHandlerIsScrolledToBottom:self]) {
    [self.mouseDelegate mouseHandlerUnlockScrolling:self];
}
```

**주목할 점**: iTerm2에도 `cancelMomentumScroll + lockScroll` 쌍이 4곳에서 반복 추출되지 않은 채
남아 있다. 같은 패턴을 공유하면서도 두 연산의 독립성을 유지하는 것이 iTerm2의 선택이다.

## 분리 원칙

두 연산은 **변경 이유**가 다르다.

| | `scroll_viewport_and_sync` | `reanchor_viewport_lock` |
|---|---|---|
| 책임 | 뷰포트 위치를 이동하고 세션의 delta 이벤트를 소비 | 새 위치가 라이브 엣지인지 판단해 lock/unlock 결정 |
| 변경 이유 | 세션 스크롤 API 변경, delta 소비 방식 변경 | ViewportLock 상태 머신 변경, at-bottom 정책 변경 |
| 호출자 관심사 | "얼마나 스크롤했나" | "이 위치에서 PTY 출력을 따라가야 하나" |

둘을 하나로 합치면 lock 정책을 바꿀 때 scroll 코드를 건드려야 하는 거짓 결합이 생긴다.
iTerm2가 `lockScroll`을 scroll과 분리해 독립 호출한 이유이기도 하다.

## 수정된 제안 — 두 개의 독립 메서드

```rust
// viewport.rs — scroll 전담: 위치 이동 + delta 소비
pub(super) fn scroll_viewport_and_sync(&mut self, delta: i32) {
    let _ = self.session.scroll_viewport(delta);
    self.sync_viewport_scroll_tracking();
}

// viewport.rs — lock 결정 전담: 현재 위치 기준으로 lock/unlock
pub(super) fn reanchor_viewport_lock(&mut self) {
    let new_offset = self.session.viewport_row_offset();
    let rows = self.session.rows() as u32;
    let total = self.session.total_rows();
    if new_offset + rows >= total {
        self.state.viewport_lock.unlock();
    } else {
        self.state.viewport_lock.lock(self.session.viewport_top_abs_y());
    }
}
```

```rust
// 트랙 클릭 / 썸 드래그 (6줄 → 4줄, 의도 명확)
self.scroll_viewport_and_sync(delta);
self.reanchor_viewport_lock();
self.refresh_viewport();
cx.notify();

// autoscroll (5줄 → 3줄 + 기존 schedule 유지)
self.scroll_viewport_and_sync(vel);
self.reanchor_viewport_lock();
// ... selection 업데이트 ...
self.schedule_viewport_refresh(cx);
```

`scroll_viewport_and_sync`는 `viewport.rs:restore_pinned_viewport`(467)와
`input.rs` PageUp/Down 키 경로에서도 같은 두 줄 패턴을 줄이는 데 재사용 가능하다.

**주의 — `on_scroll_wheel` 적용 불가**: `on_scroll_wheel`(mouse.rs:851)은 스크롤 후
위치가 실제로 이동했을 때만 sync한다(`if viewport_offset != offset_before`).
`scroll_viewport_and_sync`는 무조건 sync하므로 동작이 달라진다. 직접 교체하지 말 것.

**autoscroll에서 `rows` 재조회**: `reanchor_viewport_lock()` 내부에서 `rows`, `total`을
읽는다. 호출 직후 selection 업데이트에도 `rows`가 필요하므로 호출 측에서 별도로 재조회해야 한다.

```rust
// autoscroll 호출 측 — rows를 별도 재조회
self.scroll_viewport_and_sync(vel);
self.reanchor_viewport_lock();
let vp_offset = self.session.viewport_row_offset();
let rows = self.session.rows() as u32;   // ← reanchor 내부와 별개로 재조회
if let Some(sel) = self.state.selection.as_mut() {
    let target_row = if vel < 0 { vp_offset } else { vp_offset + rows.saturating_sub(1) };
    sel.active = ScreenPos::viewport(target_row, 0);
}
self.schedule_viewport_refresh(cx);
```

## 변경 범위

1. `viewport.rs` — `scroll_viewport_and_sync`, `reanchor_viewport_lock` 추가
2. `mouse.rs` — 세 중복 블록 교체 (`on_mouse_down` 트랙, `on_mouse_move` 썸 드래그, `autoscroll_poll_with_pos`)
3. `viewport.rs:restore_pinned_viewport` — 내부 `scroll_viewport` + `sync` 두 줄을 `scroll_viewport_and_sync`로 교체

## 우선순위

**High** — 트랙 클릭과 썸 드래그가 완전히 동일한 코드를 두 번 유지하고 있으며,
`ViewportLock` 상태 머신 변경 시 두 경로의 분기가 무조건 함께 변경되어야 한다.
