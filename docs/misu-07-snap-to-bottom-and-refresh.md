# MISU-07: `snap_to_bottom` + pending_refresh 직접 설정 5곳 → `snap_to_bottom_and_refresh`

## 현황

**파일**: `crates/daruda_terminal/src/view/input.rs`, `view/actions.rs`

`snap_to_bottom()` 호출 직후 두 pending_refresh 플래그를 직접 설정하는 경로가 5곳이다.

```rust
// input.rs:212-216  (shift+end)
self.snap_to_bottom();
self.state.pending_refresh_keep_selection = true;
self.state.pending_refresh = true;
cx.notify();

// input.rs:238-241  (shift+pagedown, at_bottom 분기)
self.snap_to_bottom();
self.state.pending_refresh_keep_selection = true;
self.state.pending_refresh = true;
cx.notify();

// input.rs:313-317  (end)
self.snap_to_bottom();
self.state.pending_refresh_keep_selection = true;
self.state.pending_refresh = true;
cx.notify();

// input.rs:339-342  (pagedown, at_bottom 분기)
self.snap_to_bottom();
self.state.pending_refresh_keep_selection = true;
self.state.pending_refresh = true;
cx.notify();

// actions.rs:221-225  (on_scroll_to_bottom)
self.snap_to_bottom();
self.state.pending_refresh_keep_selection = true;
self.state.pending_refresh = true;
cx.notify();
```

## 버그 경로

`schedule_viewport_refresh`(`viewport.rs:398-406`)는 두 플래그 외에 **`focused_prompt_row = None`과
`focused_command_row = None`도 초기화**한다.  
5곳의 직접 설정 경로는 이 초기화를 건너뛰기 때문에 End/PageDown/ScrollToBottom 동작 후에도
jump 포커스 행이 유령처럼 남아 있을 수 있다.

| 위치 | 누락된 단계 |
|------|------------|
| `input.rs:212-216` | `focused_prompt_row = None`, `focused_command_row = None` |
| `input.rs:238-241` | 동일 |
| `input.rs:313-317` | 동일 |
| `input.rs:339-342` | 동일 |
| `actions.rs:221-225` | 동일 |

## 제안 메서드

```rust
// viewport.rs — snap_to_bottom 인근에 추가
pub(super) fn snap_to_bottom_and_refresh(&mut self, cx: &mut Context<Self>) {
    self.snap_to_bottom();
    self.schedule_viewport_refresh(cx);
}
```

`schedule_viewport_refresh`가 두 플래그 설정 + jump 포커스 초기화 + `cx.notify()`를 모두 처리하므로
호출자에 별도 `cx.notify()` 불필요.

## 변경 범위

1. `viewport.rs` — 메서드 추가
2. `input.rs` — 4곳의 3-라인 블록을 `self.snap_to_bottom_and_refresh(cx)` 한 줄로 교체
   (shift+end:212, shift+pagedown at_bottom:238, end:313, pagedown at_bottom:339)
3. `actions.rs` — 1곳 동일 교체 (on_scroll_to_bottom:221)

## iTerm2 참고

`PTYTextView.mouseHandlerUnlockScrolling:` (PTYTextView.m:7431) 가 `setUserScroll:NO`
(≈ `viewport_lock.unlock`) 의 **유일한** 호출 경로다. 잠금 해제 진입점을 한 곳으로 유지하는
설계가 daruda의 방향과 일치하며, 이 MISU가 올바른 방향임을 뒷받침한다.

## 우선순위

**High** — 새 "End 계열" 키 바인딩 추가 시 focused_row 초기화 누락이 즉시 버그로 이어지는 구조.
