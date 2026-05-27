# MISU-01: `pending_refresh` + `pending_refresh_keep_selection` → `PendingRefresh` enum

## 현황

**파일**: `crates/daruda_terminal/src/view/state.rs:159–164`

```rust
pub(crate) pending_refresh: bool,
pub(crate) pending_refresh_keep_selection: bool,
```

두 bool이 항상 쌍으로 쓰인다.
- `pending_refresh = false`이면 `pending_refresh_keep_selection`은 의미가 없다.
- 유효한 조합은 4개 중 3개뿐: `(false, false)`, `(true, false)`, `(true, true)`.
- `(false, true)` 조합은 illegal state이지만 컴파일러가 막지 못한다.

## 증거

| 위치 | 내용 |
|------|------|
| `state.rs:343–354` | `pending_refresh_flags_are_independent` 테스트 — 개발자가 직접 불변식을 검증 |
| `viewport.rs:140–158` | alt-screen 종료 시 `pending_refresh = true` + `pending_refresh_keep_selection = false`를 명시적으로 쌍 설정 |
| `viewport.rs:399–404` | `schedule_viewport_refresh` 내부에서 두 필드를 항상 같이 쓴다 |

## 버그 경로

`viewport.rs`에서 두 필드를 직접 설정하는 호출 경로가 3곳 이상이다.
새 경로를 추가할 때 한 쪽만 설정하면 `(false, true)` 또는 `(true, ?)` 불일치가 발생한다.
`schedule_viewport_refresh`를 우회해 `pending_refresh = true`만 세팅하면
`pending_refresh_keep_selection`의 이전 값이 의도치 않게 재사용된다.

## 제안 타입

```rust
#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PendingRefresh {
    #[default]
    No,
    Clear,    // refresh + selection 초기화
    Preserve, // refresh + selection 유지
}
```

`ViewState`에서:
```rust
// before
pub(crate) pending_refresh: bool,
pub(crate) pending_refresh_keep_selection: bool,

// after
pub(crate) pending_refresh: PendingRefresh,
```

## 변경 예시

**`state.rs` — 타입 정의**
```rust
// before
pub(crate) pending_refresh: bool,
pub(crate) pending_refresh_keep_selection: bool,

// after
pub(crate) pending_refresh: PendingRefresh,
```

**`render.rs:70–77` — 소비 측**
```rust
// before
if self.state.pending_refresh {
    if self.state.pending_refresh_keep_selection {
        self.refresh_viewport_preserving_selection();
    } else {
        self.refresh_viewport();
    }
    self.state.pending_refresh = false;
    self.state.pending_refresh_keep_selection = false;
}

// after
match self.state.pending_refresh {
    PendingRefresh::Preserve => self.refresh_viewport_preserving_selection(),
    PendingRefresh::Clear    => self.refresh_viewport(),
    PendingRefresh::No       => {}
}
self.state.pending_refresh = PendingRefresh::No;
```

**설정 경로 — 단독 설정 3곳을 명시적 variant로 교체**
```rust
// before (jump.rs:152, mod.rs:367, viewport.rs:212)
self.state.pending_refresh = true;
// pending_refresh_keep_selection 미설정 → 이전 값 재사용

// after — 각 사이트에서 의도를 명시
self.state.pending_refresh = PendingRefresh::Clear;    // 또는 Preserve
```

**`viewport.rs` — alt-screen 종료 경로**
```rust
// before
self.state.pending_refresh = true;
// pending_refresh_keep_selection must be false ...
self.state.pending_refresh_keep_selection = false;

// after
self.state.pending_refresh = PendingRefresh::Clear;
```

**`viewport.rs:399–404` — `schedule_viewport_refresh`**
```rust
// before
self.state.pending_refresh = true;
self.state.pending_refresh_keep_selection = true;

// after
self.state.pending_refresh = PendingRefresh::Preserve;
```

## 변경 범위

1. `state.rs` — 필드 2개 → `pending_refresh: PendingRefresh`; `PendingRefresh` enum 정의
2. `render.rs:70–77` — `if/if` → `match`; 리셋을 `PendingRefresh::No` 단일 할당으로
3. `viewport.rs` — 4개 설정 경로를 각각 올바른 variant 할당으로 교체
4. `jump.rs:152`, `mod.rs:367` — 단독 설정 경로: 각 사이트 의도 확인 후 `Clear` 또는 `Preserve`
5. `ime.rs:132–133`, `input.rs` — 쌍 설정 경로: variant 단일 할당으로 축약
6. `state.rs:343–354` 방어 테스트 제거 (타입이 불변식을 보장)

## 비용

| 항목 | 내용 |
|------|------|
| 영향 파일 | 7개 (`state.rs`, `viewport.rs`, `render.rs`, `jump.rs`, `mod.rs`, `ime.rs`, `input.rs`) |
| 변이 사이트 | 쌍 설정 8곳 + 단독 설정 3곳(`jump.rs:152`, `mod.rs:367`, `viewport.rs:212`) |
| 소비 사이트 | `render.rs:70–77` 1곳 (if/else → match 전환) |
| 제거 가능 | 방어 테스트 12라인 |
| 예상 공수 | **2–3시간** |
| 위험 요소 | 단독 설정 3곳이 `keep_selection`을 묵시적으로 이전 값에 의존 중 — enum 전환 시 각 사이트 의도를 확인해야 함 |

## iTerm2에서 배운 개선 방향

`iTermScrollWheelStateMachine`은 스크롤 상태를 단일 enum + 전환 함수로 모델링한다.
"다음 상태로 가려면 무엇이 필요한가"를 전환 함수 하나에 집중시키고,
호출 경로는 전환 함수를 부를 뿐 필드를 직접 건드리지 않는다.

daruda에서도 같은 원칙을 적용한다:
- 호출 경로는 `state.pending_refresh = PendingRefresh::Clear` 한 줄만 쓴다.
- "두 필드를 같이 써야 한다"는 계약이 타입으로 사라지고 전환 API로 옮겨진다.

## 우선순위

**High** — 단독 설정 경로 3곳에서 `keep_selection`이 이전 값을 그대로 사용하는 잠재적 버그 현존.
