# MISU-02: `is_dragging` + `drag_row` + `scrollbar_drag_start` → `MouseDragState` enum

## 현황

**파일**: `crates/daruda_terminal/src/view/state.rs:100–114`

```rust
pub(crate) is_dragging: bool,
pub(crate) drag_row: Option<usize>,
pub(crate) scrollbar_drag_start: Option<f32>,
```

두 종류의 드래그(텍스트 선택 / 스크롤바)가 같은 구조체에 혼재한다.

- 텍스트 선택 드래그: `is_dragging = true` + `drag_row = Some(_)`
- 스크롤바 드래그: `scrollbar_drag_start = Some(_)`, `is_dragging`은 무관
- 유휴 상태: 셋 모두 `false`/`None`

`is_dragging`이 스크롤바 드래그 중에는 어떤 값이어야 하는지 타입이 명시하지 않는다.

## 증거

| 위치 | 내용 |
|------|------|
| `mouse.rs:342–346` | 명시적 `on_mouse_up` — `is_dragging`, `drag_row`, `autoscroll_task` 리셋 |
| `mouse.rs:516–519` | 암묵적 마우스업(창 밖) — `is_dragging` + `drag_row` 리셋, `selection`은 **조건부** 클리어 |
| `mouse.rs:345` | 스크롤바 드래그 시작: `scrollbar_drag_start = Some(...)`, `is_dragging` 미설정 |

## 버그 경로

`mouse.rs:516–519` (암묵적 마우스업)에서 `selection`을 `is_empty()`일 때만 클리어한다.
명시적 `on_mouse_up`은 조건 없이 클리어 — 두 경로가 비대칭.
`is_dragging = false`이지만 `selection`이 남은 상태에서 다음 클릭이 들어오면
stale selection을 앵커로 사용할 위험이 있다.

두 타입의 드래그가 동시에 `Some`이 되는 조합(`is_dragging=true` + `scrollbar_drag_start=Some`)도
타입으로 차단되지 않는다.

## 제안 타입

```rust
#[derive(Default, Clone, Copy, PartialEq)]
pub(crate) enum MouseDragState {
    #[default]
    None,
    TextSelection { row: usize },
    ScrollbarDrag { offset: f32 },
}
```

`ViewState`에서:
```rust
// before
pub(crate) is_dragging: bool,
pub(crate) drag_row: Option<usize>,
pub(crate) scrollbar_drag_start: Option<f32>,

// after
pub(crate) mouse_drag: MouseDragState,
```

## 변경 예시

**`state.rs` — 타입 정의**
```rust
// before
pub(crate) is_dragging: bool,
pub(crate) drag_row: Option<usize>,
pub(crate) scrollbar_drag_start: Option<f32>,

// after
pub(crate) mouse_drag: MouseDragState,
```

**`mouse.rs` — 명시적 마우스업 (`on_mouse_up`)**
```rust
// before
self.state.drag_row = None;
self.state.is_dragging = false;
self.autoscroll_task = None;
if event.button == MouseButton::Left && self.state.scrollbar_drag_start.is_some() {
    self.state.scrollbar_drag_start = None;
    cx.notify();
    return;
}

// after
self.autoscroll_task = None;
let was_scrollbar = matches!(self.state.mouse_drag, MouseDragState::ScrollbarDrag { .. });
self.state.mouse_drag = MouseDragState::None;
if event.button == MouseButton::Left && was_scrollbar {
    cx.notify();
    return;
}
```

**`mouse.rs` — 암묵적 마우스업 (창 밖 이탈)**
```rust
// before
if self.state.is_dragging && event.pressed_button != Some(MouseButton::Left) {
    self.state.is_dragging = false;
    self.autoscroll_task = None;
    self.state.drag_row = None;
    if self.state.selection.map(|s| s.is_empty()).unwrap_or(false) {
        self.state.selection = None;
    }
    cx.notify();
    return;
}

// after — on_mouse_up과 동일한 selection 클리어 로직
if matches!(self.state.mouse_drag, MouseDragState::TextSelection { .. })
    && event.pressed_button != Some(MouseButton::Left)
{
    self.autoscroll_task = None;
    self.state.mouse_drag = MouseDragState::None;
    if let Some(sel) = self.state.selection {
        if sel.is_empty() {
            self.state.selection = None;
        }
    }
    cx.notify();
    return;
}
```

**`mouse.rs` — 스크롤바 드래그 시작**
```rust
// before
self.state.scrollbar_drag_start = Some(click_offset);

// after
self.state.mouse_drag = MouseDragState::ScrollbarDrag { offset: click_offset };
```

**`mouse.rs` — 텍스트 선택 드래그 시작 (`start_autoscroll`)**
```rust
// before
self.state.is_dragging = true;

// after — drag_row는 on_mouse_drag에서 설정되므로 초기값 0 사용
self.state.mouse_drag = MouseDragState::TextSelection { row: 0 };
```

**`prepaint.rs:643–673` — scrollbar thumb 색상**
```rust
// before
let is_dragging = v.state.scrollbar_drag_start.is_some();

// after
let is_dragging = matches!(v.state.mouse_drag, MouseDragState::ScrollbarDrag { .. });
```

**`prepaint.rs:466–471` — autoscroll 범위**
```rust
// before
if let (Some(drag_row), Some(end_row)) = (view.state.drag_row, end_vp_row) { ... }

// after
if let (MouseDragState::TextSelection { row: drag_row }, Some(end_row)) =
    (view.state.mouse_drag, end_vp_row) { ... }
```

## 변경 범위

1. `state.rs` — 필드 3개 → `mouse_drag: MouseDragState`; enum 정의
2. `mouse.rs` — 드래그 시작/종료 8곳; 암묵적 마우스업 경로를 명시적 경로와 동일하게 통일
3. `element/prepaint.rs` — `scrollbar_drag_start.is_some()` → `matches!` 패턴; `drag_row` → 구조체 분해

## 비용

| 항목 | 내용 |
|------|------|
| 영향 파일 | 3개 (`state.rs`, `mouse.rs`, `element/prepaint.rs`) |
| 변이 사이트 | `mouse.rs` 8곳 (드래그 시작/종료/리셋) |
| 읽기 사이트 | `mouse.rs` 3곳, `prepaint.rs` 3곳 |
| 예상 공수 | **1–2시간** |
| 위험 요소 | `prepaint.rs`에서 `scrollbar_drag_start.is_some()`으로 thumb 색상 전환 — `MouseDragState::ScrollbarDrag` 매칭으로 교체 필요 |

## iTerm2에서 배운 개선 방향

`iTermScrollWheelStateMachine` (`sources/Swipe/iTermScrollWheelStateMachine.h:12–17`):
```objc
typedef NS_ENUM(NSUInteger, iTermScrollWheelStateMachineState) {
    iTermScrollWheelStateMachineStateGround,
    iTermScrollWheelStateMachineStateStartDrag,
    iTermScrollWheelStateMachineStateDrag,
    iTermScrollWheelStateMachineStateTouchAndHold,
};
```
- 드래그 타입별로 variant를 분리했다 — `StartDrag`와 `TouchAndHold`는 동시에 존재할 수 없다.
- 전환 함수가 유일한 상태 변경 경로이므로 리셋 비대칭 문제가 구조적으로 불가능하다.

daruda `MouseDragState` enum이 동일한 구조를 취한다.
`TextSelection { row }` ↔ `ScrollbarDrag { offset }` 전환은 항상 `None`을 경유하도록 강제하면
"두 드래그가 동시에 활성화" 조합이 타입 수준에서 차단된다.

## 우선순위

**High** — 암묵적/명시적 마우스업 간 비대칭 리셋이 현재도 존재하는 버그 구조.
