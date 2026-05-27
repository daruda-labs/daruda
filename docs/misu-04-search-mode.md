# MISU-04: `is_regex` + `regex_error` → `SearchMode` enum

## 현황

**파일**: `crates/daruda_terminal/src/view/search.rs:40–45`

```rust
pub(super) is_regex: bool,
/// True when `is_regex` but the pattern failed to compile.
pub(super) regex_error: bool,
pub(super) matches: Vec<MatchRange>,
pub(super) focused: Option<usize>,
```

주석이 직접 의존성을 명시한다: "True when `is_regex` but the pattern failed to compile."
`is_regex = false`인데 `regex_error = true`인 조합은 의미가 없지만 타입이 허용한다.

## 증거

| 위치 | 내용 |
|------|------|
| `search.rs:40` 주석 | `regex_error`의 의미가 `is_regex`에 종속됨을 명시 |
| `scan_search_matches` | `is_regex = false` 경로에서 `regex_error = false` 하드코딩 — 방어 코드 |

## 버그 경로

`scan_search_matches`가 리터럴 검색 경로에서 `regex_error = false`를 명시적으로 세팅하지 않으면
이전 정규식 오류 상태가 잔류한다.
검색 모드를 전환(`is_regex` 토글)할 때 `regex_error`를 클리어하는 경로가 빠지면
UI에서 리터럴 검색 중에도 오류 표시가 남는다.

## 제안 타입

```rust
#[derive(Default, Clone, PartialEq)]
pub(super) enum SearchMode {
    #[default]
    Literal,
    Regex {
        /// None: 컴파일 전 또는 성공. 오류 메시지가 있으면 Some.
        compile_error: Option<String>,
    },
}
```

`SearchState`에서:
```rust
// before
pub(super) is_regex: bool,
pub(super) regex_error: bool,

// after
pub(super) mode: SearchMode,
```

## 변경 예시

**`search.rs` — `ScanResult` 구조체**
```rust
// before
pub(super) struct ScanResult {
    pub matches: Vec<MatchRange>,
    pub regex_error: bool,
}

// after
pub(super) struct ScanResult {
    pub matches: Vec<MatchRange>,
    pub compile_error: Option<String>,  // None = 성공 또는 비정규식 모드
}
```

**`search.rs` — `SearchState` 필드**
```rust
// before
pub(super) is_regex: bool,
pub(super) regex_error: bool,

// after
pub(super) mode: SearchMode,
```

**`search.rs` — `scan_search_matches` 정규식 검증 경로**
```rust
// before
let viewport_regex = if is_regex {
    match compile_regex(query, case) {
        Some(re) => Some(re),
        None => {
            return ScanResult { matches: Vec::new(), regex_error: true };
        }
    }
} else {
    None
};
// ... 마지막에:
ScanResult { matches, regex_error: false }

// after — 함수 시그니처: is_regex: bool 유지 (actions.rs 무변경)
let viewport_regex = if is_regex {
    match compile_regex(query, case) {
        Some(re) => Some(re),
        None => {
            return ScanResult {
                matches: Vec::new(),
                compile_error: Some(format!("invalid regex: {query}")),
            };
        }
    }
} else {
    None
};
// ... 마지막에:
ScanResult { matches, compile_error: None }
```

**`search_bar.rs:64` — UI 렌더링**
```rust
// before
let (counter, counter_color) = if self.state.search.regex_error {
    (ux_strings::SEARCH_REGEX_ERROR.to_string(), ux_theme::SEARCH_LABEL_ERROR)
} else if ...

// after
let (counter, counter_color) = if let SearchMode::Regex { compile_error: Some(_) }
    = &self.state.search.mode
{
    (ux_strings::SEARCH_REGEX_ERROR.to_string(), ux_theme::SEARCH_LABEL_ERROR)
} else if ...
```

**`search_bar.rs:284` — 쿼리 클리어 경로**
```rust
// before
self.state.search.regex_error = false;

// after — mode를 건드릴 필요 없음 (compile_error는 scan 결과가 덮어씀)
// 삭제
```

**`search_bar.rs:307` — scan 결과 반영**
```rust
// before
self.state.search.regex_error = result.regex_error;

// after
if let SearchMode::Regex { compile_error } = &mut self.state.search.mode {
    *compile_error = result.compile_error;
}
```

**`actions.rs` — 토글 핸들러**
```rust
// before
let new_is_regex = !self.state.search.is_regex;
self.set_search_query(&q, case, new_is_regex, cx);

// after
let new_is_regex = !matches!(self.state.search.mode, SearchMode::Regex { .. });
// set_search_query 시그니처 is_regex: bool 유지 — 내부에서 mode 전환
```

## 변경 범위

1. `search.rs` — `ScanResult.regex_error` → `compile_error`; `SearchState` 필드 교체; `SearchMode` 정의
2. `search_bar.rs` — 렌더링 분기, 스캔 결과 반영, 방어적 `regex_error = false` 제거
3. `actions.rs` — `!self.state.search.is_regex` → `!matches!(mode, Regex { .. })`
4. `mod.rs` — `set_search_query` 내부에서 `mode` 전환 처리

## 비용

| 항목 | 내용 |
|------|------|
| 영향 파일 | 4개 (`search.rs`, `search_bar.rs`, `mod.rs`, `actions.rs`) |
| 변이 사이트 | `search.rs` 4곳, `search_bar.rs` 3곳 |
| `actions.rs` | `is_regex: bool`을 읽어 `set_search_query`에 전달 — 함수 시그니처 유지 시 무변경 |
| 예상 공수 | **1–2시간** |
| 위험 요소 | `ScanResult` 구조체도 변경 필요 (`regex_error: bool` → `compile_error: Option<String>`) |

## iTerm2에서 배운 개선 방향

`iTermFindMode` enum (`sources/SearchingFiltering/iTermFindViewController.h`):
```objc
typedef NS_ENUM(NSUInteger, iTermFindMode) {
    iTermFindModeSmartCaseSensitivity = 0,
    iTermFindModeCaseSensitiveSubstring = 1,
    iTermFindModeCaseInsensitiveSubstring = 2,
    iTermFindModeCaseSensitiveRegex = 3,
    iTermFindModeCaseInsensitiveRegex = 4,
};
NS_INLINE BOOL iTermFilterModeIsRegularExpression(iTermFindMode mode) {
    switch (mode) { ... }  // 컴파일러 exhaustiveness 검사
}
```
`isRegex` + `caseSensitive` bool 쌍 대신 단일 enum 하나로 모든 유효 조합을 표현한다.
`isRegularExpression()` 파생 함수가 bool 재조합 없이 동작하는 것이 핵심이다.

daruda의 `SearchMode` enum도 같은 원리다.
iTerm2 대비 daruda가 한 단계 더 나아가는 부분: **컴파일 오류 상태를 타입 안에 포함**한다.
iTerm2는 오류를 검색 실행 시점에만 처리해 UI가 stale 오류 표시를 할 수 있지만,
`Regex { compile_error: Option<String> }` 구조에서는 패턴이 변경되는 순간 오류 상태가 즉시 갱신된다.

## 우선순위

**Medium** — 현재 `scan_search_matches`가 매 호출마다 상태를 덮어쓰므로 즉각적 버그 경로는 없음. 토글 경로에서 누락 시 stale 오류 표시 위험.
