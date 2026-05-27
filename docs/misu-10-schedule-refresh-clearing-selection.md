# MISU-10: "selection 클리어 refresh 예약" 고립 패턴 → `schedule_viewport_refresh_clearing_selection`

## 현황

**파일**: `crates/daruda_terminal/src/view/output.rs:152-158`

alt-screen 종료 분기에서 `pending_refresh = true`와 `pending_refresh_keep_selection = false`를 직접 설정한다.

```rust
// apply_screen_change 내 alt-screen 종료 분기
Some(false) => {
    let _ = self.session.feed(crate::ansi::ERASE_SCROLLBACK);
    let _ = self.session.feed(crate::ansi::ERASE_DISPLAY_AND_HOME);
    self.state.pending_refresh = true;
    // Explicitly clear selection: any ScreenPos anchors held from
    // the primary screen are now invalid after the buffer switch.
    // pending_refresh_keep_selection must be false so the deferred
    // refresh calls refresh_viewport() (clears selection) rather
    // than refresh_viewport_preserving_selection().
    self.state.pending_refresh_keep_selection = false;
}
```

`schedule_viewport_refresh`(`viewport.rs:398-406`)는 항상 두 플래그를 모두 `true`로 설정하므로
이 경로에서 호출 불가. 결과적으로 "selection을 초기화하는 refresh 예약"에 해당하는 단일 진입점이 없고,
4줄짜리 주석이 의도를 대신하고 있다.

## 버그 경로

현재 이 경로는 1곳뿐이지만 유사한 "전체 상태 리셋" 경로가 추가될 때 위험하다.

- RIS(Reset to Initial State) 처리
- 세션 재접속(reconnect) 후 뷰 리셋
- 탭 이동 시 alt-screen 잔여 상태 정리

각 경로가 `pending_refresh_keep_selection = false`를 개별적으로 설정하거나,
실수로 `schedule_viewport_refresh`를 그대로 호출하면 selection이 유지된다.

## 관련 이슈

MISU-01이 두 bool을 `PendingRefresh` enum으로 교체하면 이 경로는
`self.state.pending_refresh = PendingRefresh::Clear`로 단순화된다.
MISU-01 완료 전에는 아래 메서드로 의도를 명시하는 것이 현실적인 개선이다.

## 제안 메서드

```rust
// viewport.rs — schedule_viewport_refresh 인근에 추가
pub(super) fn schedule_viewport_refresh_clearing_selection(
    &mut self,
    cx: &mut Context<Self>,
) {
    self.state.focused_prompt_row = None;
    self.state.focused_command_row = None;
    self.state.pending_refresh = true;
    self.state.pending_refresh_keep_selection = false;
    cx.notify();
}
```

```rust
// output.rs apply_screen_change 호출 측
Some(false) => {
    let _ = self.session.feed(crate::ansi::ERASE_SCROLLBACK);
    let _ = self.session.feed(crate::ansi::ERASE_DISPLAY_AND_HOME);
    self.schedule_viewport_refresh_clearing_selection(cx);
}
```

## 변경 범위

1. `viewport.rs` — 메서드 추가
2. `output.rs` — `apply_screen_change` 내 2줄(필드 직접 설정) + 4줄 주석 교체
   → 주석은 메서드 이름이 의도를 표현하므로 삭제

**동작 변화 주의**: 제안 메서드는 `focused_prompt_row`와 `focused_command_row`도 `None`으로
초기화한다. 현재 `apply_screen_change`는 이 초기화를 하지 않는다. alt-screen 종료 후 점프
포커스 행이 무효화된 스크린 좌표를 가리킬 수 있으므로 초기화가 올바른 동작이지만,
단순 추출이 아닌 **동작 추가**임을 구현 시 인지해야 한다.

MISU-01 완료 시 이 메서드는 제거하고 `PendingRefresh::Clear` 할당으로 흡수.

## iTerm2 참고

iTerm2에는 `pending_refresh` 플래그 패턴 자체가 없어 직접 대응 구조가 없다.
렌더를 `requestDelegateRedraw`(즉각 요청)로 처리하고 selection은 `[_selection clearSelection]`으로
직접 초기화하기 때문에 "deferred flag + keep/clear 분기" 설계가 필요하지 않다.

이 MISU는 daruda 고유의 구조적 문제이며, iTerm2 참고에서 도출할 대안은 없다.
MISU-01(PendingRefresh enum)로 흡수하는 기존 방향을 유지한다.

## 우선순위

**Low** — 현재 호출 경로는 1곳이며 즉각적인 버그 위험은 낮다.
MISU-01 착수 전에 작업하면 효과적이고, 착수 이후라면 MISU-01의 `PendingRefresh::Clear` 전환에서 흡수.
