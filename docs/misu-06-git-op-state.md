# MISU-06: `git_op_in_flight` + `git_stage_in_flight` → `GitOpState` enum

## 현황

**파일**: `crates/app/src/workspace/mod.rs:387,391`

```rust
pub(in crate::workspace) git_op_in_flight: bool,
/// running. Separate from `git_op_in_flight` so a stage click doesn't
/// block the UI while a larger operation is running.
pub(in crate::workspace) git_stage_in_flight: bool,
```

두 플래그가 의도적으로 분리되어 있다는 주석이 존재한다.
그러나 두 플래그가 동시에 `true`일 수 있는지, 그 경우 UI가 어떻게 동작해야 하는지
타입으로 표현되지 않는다.

## 분석

주석의 의도: "큰 git 연산이 진행 중이어도 stage 클릭은 UI를 블로킹하지 않아야 한다."
→ `git_op_in_flight = true` + `git_stage_in_flight = true`가 **동시에 가능한** 의도된 설계로 보인다.

그렇다면 유효 조합:
- `(false, false)` — 유휴
- `(true, false)` — 대형 연산 진행 중
- `(false, true)` — stage만 진행 중
- `(true, true)` — 대형 연산 + stage 동시

4개 조합이 모두 유효하다면 MISU 문제가 아니라 네이밍/문서화 문제.

## 확인 필요 사항

변경 전 아래를 확인해야 한다:

1. `git_stage_in_flight`를 소비하는 UI 코드가 `git_op_in_flight`와 독립적으로 분기하는지
2. `(true, true)` 조합에서 UI가 의도대로 동작하는지 (stage 버튼만 비활성화, 나머지 UI는 정상)
3. 두 플래그 리셋 경로가 각각 독립적인지 (stage 완료가 대형 연산 플래그를 건드리지 않는지)

## 제안 타입 (확인 후 선택)

두 플래그가 독립적 의미를 갖는다면 (현재 주석과 일치):
```rust
// 구조체로 묶어 "git 연산 상태"라는 의미 단위 명확화
pub(in crate::workspace) git_ops: GitOpsState,

struct GitOpsState {
    pub major_in_flight: bool,
    pub stage_in_flight: bool,
}
impl GitOpsState {
    pub fn any_in_flight(&self) -> bool { self.major_in_flight || self.stage_in_flight }
}
```

`(false, true)` 조합이 실제로 발생하지 않는다면 (stage는 항상 major와 함께):
```rust
pub(in crate::workspace) git_op: GitOpState,

enum GitOpState { None, MajorOp, MajorOpWithStage, StageOnly }
```

## 비용 분석 결과: WONTFIX 권고

사용 횟수 집계 결과:

| 파일 | `git_op_in_flight` | `git_stage_in_flight` |
|------|--------------------|-----------------------|
| `history.rs` | 10곳 | 0 |
| `index.rs` | 0 | 20곳 |
| `init.rs` | 3곳 | 0 |
| `git_changes/mod.rs` | 2곳 | 3곳 |
| `layout/snap.rs` | 필드 정의 | 필드 정의 |
| `render/snapshots.rs` | 복사 | 복사 |
| **합계** | **~15곳** | **~23곳** |

두 플래그는 완전히 독립된 파일(`history.rs` vs `index.rs`)에서 관리된다.
실제로 `(true, true)` 조합이 정상 동작 경로에서 발생한다 — stage 작업 중에 fetch/push가 진행될 수 있다.
4개 조합이 모두 유효하므로 이것은 MISU 문제가 아니라 의도적인 독립 플래그 설계다.

**변경 시 비용**: 38곳 + `snap.rs` + `snapshots.rs` = **50곳 이상**, 7개 파일  
**이득**: 없음 (모든 조합이 유효, illegal state가 존재하지 않음)

## 권고 사항

타입 통합 대신 주석을 강화:
```rust
/// True while a Fetch / Push / Rebase / Reset is running.
/// Independent from `git_stage_in_flight` — both can be true simultaneously.
pub(in crate::workspace) git_op_in_flight: bool,

/// True while a stage / unstage operation is running.
/// Independent from `git_op_in_flight` — stage clicks are never blocked by major ops.
pub(in crate::workspace) git_stage_in_flight: bool,
```

## 참조 패턴

iTerm2는 git 연산을 처리하지 않는다.
유사 패턴: 두 개의 독립적인 비동기 작업 진행 상태를 추적하는 경우,
iTerm2는 각 작업에 별도 delegate callback을 사용하고 상태를 중앙에 저장하지 않는다.
daruda의 두 플래그도 같은 원칙 — 완전히 독립된 두 작업 타입이므로 결합 불필요.

## 우선순위

**WONTFIX** — 의도된 독립 설계. 변경 비용이 높고 얻는 것이 없다.
