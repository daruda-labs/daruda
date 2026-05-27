# MISU-05: `bell_flash_until` + `prompt_jump_flash_until` → `FlashOverlay` enum

## 현황

**파일**: `crates/daruda_terminal/src/view/state.rs` (벨 플래시 / 프롬프트 점프 플래시 필드)

```rust
pub(crate) bell_flash_until: Option<Instant>,
pub(crate) prompt_jump_flash_until: Option<Instant>,
```

두 필드가 동일 타입(`Option<Instant>`)을 쓰면서 개념적으로 다른 시각 효과를 표현한다.

## 증거

| 위치 | 내용 |
|------|------|
| `state.rs:357–369` | `flash_deadlines_are_independent` 테스트 — "A regression that aliased them would either show both flashes at once or neither" |

방어 테스트가 필요한 이유: 두 필드를 같은 `Option<Instant>`로 타입하므로
실수로 하나를 다른 하나에 대입하거나 같은 변수로 alias할 경우 컴파일러가 잡지 못한다.

## 현재 상태

두 플래시가 동시에 활성화될 수 있는지 여부가 타입으로 표현되지 않는다.
코드베이스를 보면 두 플래시는 독립적으로 트리거되므로 동시 활성화가 의도된 경우처럼 보이나,
렌더러가 둘 다 `Some`일 때 어떻게 처리하는지 명시되어 있지 않다.

## 결정: additive `struct FlashOverlay`

`prepaint.rs:629–634`에서 두 플래시를 `flash_overlay_if_active`로 각각 독립 처리한다 — 동시 활성화가 의도된 설계다. exclusive enum 대신 additive struct로 통합한다.

## 제안 타입

```rust
#[derive(Default, Clone, Copy, PartialEq)]
pub(crate) struct FlashOverlay {
    pub bell: Option<Instant>,
    pub prompt_jump: Option<Instant>,
}
```

## 변경 예시

**`state.rs` — 필드 통합**
```rust
// before
pub(crate) bell_flash_until: Option<Instant>,
pub(crate) prompt_jump_flash_until: Option<Instant>,

// after
pub(crate) flash: FlashOverlay,
```

**`render.rs:95` — 벨 트리거**
```rust
// before
self.state.bell_flash_until = Some(Instant::now() + bell);

// after
self.state.flash.bell = Some(Instant::now() + bell);
```

**`jump.rs:159` — 프롬프트 점프 트리거**
```rust
// before
self.state.prompt_jump_flash_until = Some(Instant::now() + flash);

// after
self.state.flash.prompt_jump = Some(Instant::now() + flash);
```

**`prepaint.rs:629–634` — 렌더러**
```rust
// before
let bell_flash = flash_overlay_if_active(
    self.view.read(cx).state.bell_flash_until, || { ... }
);
let prompt_jump_flash = flash_overlay_if_active(
    self.view.read(cx).state.prompt_jump_flash_until, || { ... }
);

// after — 로직 무변경, 접근 경로만 수정
let flash = &self.view.read(cx).state.flash;
let bell_flash = flash_overlay_if_active(flash.bell, || { ... });
let prompt_jump_flash = flash_overlay_if_active(flash.prompt_jump, || { ... });
```

**`state.rs` — 방어 테스트 제거**
```rust
// 제거: flash_deadlines_are_independent 테스트 (12라인)
// FlashOverlay 구조체가 두 필드를 별개 타입으로 관리 → 앨리어싱 불가
```

## 변경 범위

1. `state.rs` — 필드 2개 → `flash: FlashOverlay`; struct 정의; 방어 테스트 제거
2. `render.rs:95` — `bell_flash_until` → `flash.bell`
3. `jump.rs:159` — `prompt_jump_flash_until` → `flash.prompt_jump`
4. `prepaint.rs:629–634` — 접근 경로 수정, 렌더 로직 무변경

## 비용

| 항목 | 내용 |
|------|------|
| 영향 파일 | 4개 (`state.rs`, `jump.rs`, `render.rs`, `element/prepaint.rs`) |
| 변이 사이트 | 쓰기 2곳(`jump.rs:159`, `render.rs:95`), 읽기 2곳(`prepaint.rs:629,634`) |
| 예상 공수 | **30분–1시간** |
| 선결 과제 | `prepaint.rs` 렌더 로직 확인 — 두 플래시가 동시에 활성화될 수 있는지 여부에 따라 exclusive/additive 선택 |
| 위험 요소 | 없음 (가장 단순한 변경) |

## iTerm2에서 배운 개선 방향

`VT100ScreenDelegate.h:226–234`:
```objc
- (void)screenActivateBellAudibly:(BOOL)audibleBell
                          visibly:(BOOL)flashBell
                    showIndicator:(BOOL)showBellIndicator
                            quell:(BOOL)quell;
```
iTerm2는 플래시를 일회성 이벤트로 delegate에 전달하고 렌더러가 타이머를 독립 관리한다.
"두 플래시가 exclusive인가 additive인가" 질문이 없다 — 각 효과가 완전히 독립된 신호이기 때문이다.

daruda에 적용:
`prepaint.rs:629–634`가 두 필드를 `flash_overlay_if_active`로 이미 각각 독립 처리한다.
→ additive 구조가 의도된 설계다. `struct FlashOverlay`로 묶어 의미 단위를 명확히 한다:
```rust
pub(crate) struct FlashOverlay {
    pub bell: Option<Instant>,
    pub prompt_jump: Option<Instant>,
}
```
이후 `prepaint.rs`는 필드 접근 경로만 바뀌고, 렌더 로직은 변경이 없다.

## 우선순위

**Low** — 방어 테스트로 커버, 즉각적 버그 경로 없음. `struct FlashOverlay` 래핑이 가장 낮은 비용으로 의미 단위를 명확히 한다.
