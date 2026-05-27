# MISU-03: `char_selection` + `char_anchor` + `is_drag_selecting` → `SelectionDrag` enum

## 현황

**파일**: `crates/app/src/workspace/main_area/file_view_pane/mod.rs:176–181`

```rust
pub char_selection: Option<CharSelection>,
pub char_anchor: Option<CharPos>,
pub is_drag_selecting: bool,
```

파일 뷰어의 텍스트 선택 상태를 세 필드가 분산 표현한다.

- 드래그 중: `is_drag_selecting = true` + `char_anchor = Some(_)` + `char_selection = Some(_)`
- 드래그 완료: `is_drag_selecting = false` + `char_anchor = Some(_)` + `char_selection = Some(_)` (shift+클릭 확장용)
- 선택 없음: 셋 모두 `false`/`None`

의도한 상태가 3개임에도 2^3 = 8개 조합이 가능하다.

## 증거

| 위치 | 내용 |
|------|------|
| `handle_mouse_down` (line ~392) | `char_anchor` + `char_selection` 동시 설정 |
| `handle_mouse_drag` (line ~424) | `is_drag_selecting`만 리셋하는 경로 존재 |
| 드래그 핸들러 | `char_anchor.is_some()` 방어적 체크 수행 |

## 버그 경로

`handle_mouse_drag`에서 좌버튼 릴리스 시 `is_drag_selecting = false`만 세팅하고
`char_anchor` / `char_selection`은 그대로 두는 경우가 있다.
그 상태에서 새 클릭이 들어오기 전, 외부 이벤트로 `char_anchor`만 클리어되면
`is_drag_selecting = false` + `char_anchor = None` + `char_selection = Some(_)`라는
불일치 상태가 된다.

`shift+클릭` 확장 경로에서 `char_anchor.is_some()`을 전제하므로,
`char_anchor = None`인 상태에서 이 경로를 타면 silent no-op 또는 잘못된 범위 확장.

## 제안 타입

`InProgress`와 `Complete` 모두 `CharSelection { anchor, active }` 값을 그대로 담는다.
`CharSelection`이 이미 앵커+활성 위치를 함께 갖고 있으므로 중복 필드 없이 표현할 수 있다.

```rust
#[derive(Default, Clone, PartialEq)]
pub enum SelectionDrag {
    #[default]
    None,
    /// 버튼을 누른 채 드래그 진행 중. `sel.anchor` 고정, `sel.active` 갱신.
    InProgress(CharSelection),
    /// 버튼을 뗐지만 앵커 유지. shift+클릭으로 `sel.anchor` 기준 확장 가능.
    Complete(CharSelection),
}
```

```rust
impl SelectionDrag {
    pub fn char_selection(&self) -> Option<&CharSelection> {
        match self {
            Self::InProgress(sel) | Self::Complete(sel) => Some(sel),
            Self::None => None,
        }
    }

    pub fn anchor(&self) -> Option<CharPos> {
        self.char_selection().map(|s| s.anchor)
    }

    pub fn is_in_progress(&self) -> bool {
        matches!(self, Self::InProgress(_))
    }
}
```

`FileViewState`에서:
```rust
// before
pub char_selection: Option<CharSelection>,
pub char_anchor: Option<CharPos>,
pub is_drag_selecting: bool,

// after
pub selection_drag: SelectionDrag,
```

## 변경 예시

**`handle_mouse_down` — 클릭/shift+클릭**
```rust
// before
pub(in crate::workspace) fn handle_mouse_down(&mut self, hit: CharPos, shift: bool) {
    if shift {
        let anchor = self.char_anchor.unwrap_or(hit);
        self.char_selection = Some(CharSelection { anchor, active: hit });
    } else {
        self.char_anchor = Some(hit);
        self.char_selection = Some(CharSelection { anchor: hit, active: hit });
        self.is_drag_selecting = true;
    }
}

// after
pub(in crate::workspace) fn handle_mouse_down(&mut self, hit: CharPos, shift: bool) {
    let anchor = if shift { self.selection_drag.anchor().unwrap_or(hit) } else { hit };
    self.selection_drag = SelectionDrag::InProgress(CharSelection { anchor, active: hit });
}
```

**`handle_mouse_drag` — 드래그 진행/릴리스**
```rust
// before
pub(in crate::workspace) fn handle_mouse_drag(
    &mut self, active: CharPos, still_pressed: bool, hovered: bool,
) -> bool {
    if self.is_drag_selecting && !still_pressed {
        self.is_drag_selecting = false;
        return true;
    }
    if !self.is_drag_selecting || !hovered { return false; }
    let Some(anchor) = self.char_anchor else { return false; };
    let new_sel = CharSelection { anchor, active };
    if self.char_selection.as_ref() != Some(&new_sel) {
        self.char_selection = Some(new_sel);
        return true;
    }
    false
}

// after
pub(in crate::workspace) fn handle_mouse_drag(
    &mut self, active: CharPos, still_pressed: bool, hovered: bool,
) -> bool {
    let SelectionDrag::InProgress(ref sel) = self.selection_drag else {
        return false;
    };
    let anchor = sel.anchor;
    if !still_pressed {
        let sel = CharSelection { anchor, active };
        self.selection_drag = if sel.is_empty() {
            SelectionDrag::None
        } else {
            SelectionDrag::Complete(sel)
        };
        return true;
    }
    if !hovered { return false; }
    let new_sel = CharSelection { anchor, active };
    if self.selection_drag.char_selection() != Some(&new_sel) {
        self.selection_drag = SelectionDrag::InProgress(new_sel);
        return true;
    }
    false
}
```

**`file_view.rs` — 3곳의 트리플 리셋**
```rust
// before
fc.view.char_selection = None;
fc.view.char_anchor = None;
fc.view.is_drag_selecting = false;

// after
fc.view.selection_drag = SelectionDrag::None;
```

**렌더 파이프라인 — `body.rs:31`**
```rust
// before
let char_selection = fv.char_selection.clone();
// 이후: char_selection: Option<&CharSelection> 파라미터로 전달

// after — char_selection() 반환이 &CharSelection이므로 동일하게 사용
let char_selection = fv.selection_drag.char_selection().cloned();
// 렌더 함수 시그니처 char_selection: Option<&CharSelection> 무변경
```

## 변경 범위

1. `file_view_pane/mod.rs` — 필드 3개 → `selection_drag: SelectionDrag`; enum + impl 정의
2. `handle_mouse_down` / `handle_mouse_drag` — enum 전환으로 재작성
3. `file_view.rs` 3곳, `file_pane_ops.rs` 3곳 — 트리플 리셋 → `SelectionDrag::None`
4. `render/body.rs:31` — `fv.char_selection.clone()` → `fv.selection_drag.char_selection().cloned()`
5. `render/markdown.rs` 클로저 — `fv.char_anchor.unwrap_or(pos)` → `fv.selection_drag.anchor().unwrap_or(pos)`
6. `render/mod.rs` — `fv.is_drag_selecting` → `fv.selection_drag.is_in_progress()`

## 비용

| 항목 | 내용 |
|------|------|
| 영향 파일 | ~10개 (`file_view_pane/mod.rs`, `file_view.rs`, `render/mod.rs`, `render/markdown.rs`, `render/body.rs`, `render/row.rs`, `render/content_element.rs`, `file_pane_ops.rs` 등) |
| 변이 사이트 | ~25곳 (3× 트리플 리셋 in `file_view.rs`, 드래그 핸들러 다수) |
| 렌더 파이프라인 | `body.rs` / `row.rs` / `content_element.rs`는 `char_selection: Option<&CharSelection>` 파라미터로 전달 — `SelectionDrag::char_selection()` 접근자 추가로 대부분 무변경 |
| 예상 공수 | **4–5시간** |
| 위험 요소 | `render/markdown.rs`가 클로저 내부에서 `fv.char_anchor` / `fv.is_drag_selecting`을 직접 수정 (View purity 위반 가능성) — enum 전환 시 접근자 API 설계 필요 |

### 렌더 파이프라인 변경 최소화 전략

```rust
impl SelectionDrag {
    pub fn char_selection(&self) -> Option<&CharSelection> {
        match self {
            Self::InProgress { anchor, active } => Some(/* 임시 범위 */),
            Self::Complete { selection, .. } => Some(selection),
            Self::None => None,
        }
    }
    pub fn anchor(&self) -> Option<CharPos> { ... }
}
```

`char_selection()` 접근자를 추가하면 렌더 함수 시그니처를 그대로 유지할 수 있다.

## iTerm2에서 배운 개선 방향

`iTermSelection.h`는 공개 API를 readonly 파생 프로퍼티로만 노출한다:
```objc
@property(nonatomic, readonly) BOOL extending;  // live && extend
@property(nonatomic, readonly) BOOL live;
```
외부에서 invalid state를 직접 만들 수 없다는 점에서 올바른 방향이다.
단, 내부 구현은 여전히 두 개 bool이라 내부 코드에서 여전히 불일치가 가능하다.

daruda는 내부도 enum으로 통합해 이 한계를 제거한다.
`SelectionDrag` enum은 내부/외부 구분 없이 컴파일러가 모든 경로를 검사한다.
`char_selection()` 접근자는 iTerm2의 파생 프로퍼티와 동일한 역할을 하되,
구현이 여전히 bool 쌍에 의존하지 않는다.

## 우선순위

**High** — 마우스업 경로가 복수이고 각각 다른 필드를 리셋하는 비대칭 구조 현존. 단, 영향 범위가 가장 넓어 신중한 구현 필요.
