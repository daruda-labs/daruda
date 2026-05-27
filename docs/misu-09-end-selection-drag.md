# MISU-09: 드래그 종료 3-필드 2곳 → `end_selection_drag`

## 현황

**파일**: `crates/daruda_terminal/src/view/mouse.rs`

드래그 종료 시 초기화해야 하는 3개 필드가 두 곳에서 각각 다른 순서로 처리된다.

```rust
// on_mouse_up:342-344  (명시적 마우스업)
self.state.drag_row = None;
self.state.is_dragging = false;
self.autoscroll_task = None;

// on_mouse_move:517-519  (창 밖에서 버튼을 뗀 암묵적 마우스업)
self.state.is_dragging = false;
self.autoscroll_task = None;
self.state.drag_row = None;
```

두 경로 모두 `on_mouse_down` → `start_autoscroll`이 설정한 드래그 상태를 정리하는 "드래그 종료" 의미를 갖는다.

## 버그 경로

`start_autoscroll`(`mouse.rs:575-611`)이 현재 설정하는 드래그 상태:

```rust
self.state.is_dragging = true;
self.state.viewport_lock.lock(...);
self.autoscroll_task = Some(...);
```

드래그 시작 시 설정되는 필드가 추가될 때(예: 드래그 시작 위치 저장, 드래그 모드 플래그 등)
`on_mouse_up`은 눈에 잘 보이는 경로라 수정하기 쉽지만,
`on_mouse_move` 내부의 암묵적 마우스업 분기는 조용히 빠뜨리기 쉽다.

## 제안 메서드

```rust
// mouse.rs 내부 (또는 드래그 관련 메서드 인근)
fn end_selection_drag(&mut self) {
    self.state.is_dragging = false;
    self.state.drag_row = None;
    self.autoscroll_task = None;
}
```

호출 측은 이후 필요에 따라 selection 정리와 `cx.notify()`를 각각 처리.

```rust
// on_mouse_up
self.end_selection_drag();
if event.button == MouseButton::Left && self.state.scrollbar_drag_start.is_some() {
    self.state.scrollbar_drag_start = None;
    cx.notify();
    return;
}
// ...

// on_mouse_move (암묵적 마우스업)
if self.state.is_dragging && event.pressed_button != Some(MouseButton::Left) {
    self.end_selection_drag();
    if self.state.selection.map(|s| s.is_empty()).unwrap_or(false) {
        self.state.selection = None;
    }
    cx.notify();
    return;
}
```

## 변경 범위

1. `mouse.rs` — `end_selection_drag` 메서드 추가, 두 경로에서 호출

## iTerm2 참고 — 장기 방향

iTerm2는 이 패턴을 **별도 객체**로 완전히 분리했다. `iTermSelectionScrollHelper`가
드래그-스크롤 관련 상태를 캡슐화하고, `mouseUp` 하나로 일괄 정리한다.

```objc
// iTermSelectionScrollHelper.m
- (void)mouseUp {
    _selectionScrollDirection = 0;
    _disabled = NO;
}

// PTYMouseHandler.m — 드래그 종료
_mouseDown = NO;
[_selectionScrollHelper mouseUp];   // 드래그 스크롤 상태 정리 위임
_mouseDragged = NO;
```

daruda의 `is_dragging`, `drag_row`, `autoscroll_task`는 `TerminalView`에 평탄하게 산재되어 있어
iTerm2보다 분리가 덜 되어 있다.

**단계적 방향**:
1. **1단계 (이 MISU)** — `end_selection_drag` 메서드 추출. 두 경로의 정리 로직을 한 곳으로.
2. **2단계 (별도 MISU)** — 드래그 상태를 `SelectionDrag` struct로 분리. 아래 참고.

## 2단계 struct 설계

### 현재 필드 배치

```
TerminalViewState (GPUI-free)        TerminalView (GPUI entity)
────────────────────────────         ──────────────────────────
is_dragging: bool            ─┐
drag_row: Option<usize>      ─┤─ 드래그 상태 ─┬─ autoscroll_task: Option<Task<()>>
                               └──────────────┘
```

`is_dragging: bool` + `drag_row: Option<usize>` 조합은 CLAUDE.md 안티패턴:
> ❌ `bool` flag + 그 flag가 `true`일 때만 의미 있는 `Option` 필드
> → `Option<Active { data }>` 로 invalid state를 unrepresentable하게

`drag_row`는 `is_dragging = true`일 때만 유의미하다. 별개 필드로 두면
`is_dragging = false`이면서 `drag_row = Some(_)`인 상태를 타입이 막지 못한다.

### 제안 struct

`autoscroll_task`(`gpui::Task<()>`)는 GPUI 타입이므로 `TerminalViewState`에 둘 수 없다.
드래그 상태 전체를 `TerminalView`에 `Option<SelectionDrag>`로 통합한다.

```rust
// mouse.rs (또는 mod.rs) — GPUI 의존
pub(super) struct SelectionDrag {
    /// 마지막 드래그 위치의 뷰포트 행 (0-based).
    /// paint가 빈 행까지 하이라이트를 연장할 때 사용.
    pub(super) drag_row: Option<usize>,
    /// 창 밖 커서 위치를 50ms 주기로 폴링하는 태스크.
    /// Drop 시 태스크가 자동 취소되므로 명시적 취소 불필요.
    autoscroll_task: gpui::Task<()>,
}
```

### 필드 이전 계획

```rust
// TerminalViewState — 제거
- pub(crate) is_dragging: bool,
- pub(crate) drag_row: Option<usize>,

// TerminalView — 교체
- pub(super) autoscroll_task: Option<gpui::Task<()>>,
+ pub(super) drag: Option<SelectionDrag>,
```

### 사용 변화

| 현재 | 변경 후 |
|------|---------|
| `self.state.is_dragging` | `self.drag.is_some()` |
| `self.state.drag_row` | `self.drag.as_ref().and_then(\|d\| d.drag_row)` |
| `self.state.drag_row = Some(r)` | `if let Some(d) = &mut self.drag { d.drag_row = Some(r) }` |
| 3-필드 초기화 (`end_selection_drag`) | `self.drag = None;` (Task drop → 자동 취소) |

### `on_mouse_down:110` 처리

`on_mouse_down` 첫 줄에서 `self.state.drag_row = None`을 리셋한다. 2단계 후 이 필드는
`TerminalViewState`에 없으므로 컴파일 오류가 된다. 해당 줄을 제거하면 된다 — 드래그 시작
(`start_autoscroll`)이 새 `SelectionDrag`를 생성하면서 `drag_row: None`으로 초기화하므로
별도 리셋이 불필요하다.

### paint 접근 경로 변화

`drag_row`는 paint(`element/`)에서 빈 행 하이라이트 연장에 사용된다. `TerminalViewState`
모듈 주석 규칙("paint와 event 양쪽이 접근하는 필드는 TerminalViewState에")과 충돌하지만,
`element/`는 이미 `view.state` 대신 view 엔티티 전체를 읽으므로 `view.drag.as_ref().and_then(...)`
형태로 접근 가능하다. `element/` 내 `drag_row` 참조를 모두 변경해야 한다.

`self.drag = None` 한 줄이 1단계의 `end_selection_drag` 전체를 대체한다.
`autoscroll_task`의 Drop semantics를 활용해 취소를 명시하지 않아도 된다.

### 분리 원칙

`SelectionDrag`는 **드래그가 활성인 동안만 존재하는 값들**의 묶음이다.
`Option<SelectionDrag>`의 `Some`/`None`이 "드래그 중"/"드래그 없음" 상태 전이를 대표한다.
`start_autoscroll`은 `Some(...)` 생성, `end_selection_drag`(또는 `drag = None`)은 `None`으로 전이.
새 드래그 전용 필드를 추가할 때 struct 안에 넣으면 컴파일러가 초기화와 해제를 동시에 강제한다.

## 우선순위

**Medium** — 현재 버그는 없으나 `start_autoscroll`이 드래그 상태 필드를 확장할 때
암묵적 경로에서 누락되기 쉬운 구조. 1단계는 비용이 낮고 2단계로 가는 발판이 된다.
