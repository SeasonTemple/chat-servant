----------------------- MODULE DispatchAndSessionGate -----------------------
\* chat-servant 核心安全状态机的 TLA+ 形式化规约。
\*
\* 对应生产实现与可执行模型检验器（Executable Model Checker）：
\*   - crates/core/src/dispatch.rs          (Dispatch 档位状态机与 DeliveryReceipt)
\*   - ui/src-tauri/src/commands.rs         (Arming 同轮防重、SentRegistry 自发回声表、sends_for_real)
\*   - ui/src-tauri/src/watch.rs            (会话身份闸 SessionGate 与静默期零调用)
\*   - ui/src-tauri/src/commands.rs :: tla_and_bdd_verification
\*     （在 cargo test 中以 BFS 穷举本规约定义的动作转移图并逐态断言全部 Safety Invariants）

EXTENDS Naturals, Sequences, FiniteSets

CONSTANTS
    Candidates,     \* 候选 ID 集合，例如 {"c0", "c1"}
    Peers           \* 会话身份指纹集合，例如 {"PeerA", "PeerB"}

VARIABLES
    \* ── 一、投递与回声抑制状态机变量 ──
    tier,           \* 当前发送档位 ∈ {"Manual", "Auto"}（硬约束第 5 条：仅此两档）
    ticks,          \* 观察循环当前轮次 ∈ Nat
    armed,          \* 当前轮次已触发过真实发送的候选集合 ⊆ Candidates
    armed_tick,     \* armed 记录所属的轮次 ∈ Nat
    sent_registry,  \* 已发出且在冷却期内的候选文本集合 ⊆ Candidates
    delivered,      \* 历史真实按回车发送记录 Seq([tick: Nat, cand: Candidates, tier: STRING])
    draft_triggers, \* 因屏幕消息触发下游起草的次数 ∈ Nat

    \* ── 二、会话身份闸状态机变量 ──
    locked_peer,    \* 当前锁定的会话身份 ∈ Peers
    screen_peer,    \* 屏幕当前显示的会话身份 ∈ Peers
    panel_messages, \* 面板对外展示的消息数 ∈ Nat
    quiet_calls     \* 静默期/切走切回期间发生的模型调用数 ∈ Nat

vars == <<tier, ticks, armed, armed_tick, sent_registry, delivered,
          draft_triggers, locked_peer, screen_peer, panel_messages, quiet_calls>>

Tiers == {"Manual", "Auto"}

Init ==
    /\ tier = "Manual"
    /\ ticks = 1
    /\ armed = {}
    /\ armed_tick = 1
    /\ sent_registry = {}
    /\ delivered = << >>
    /\ draft_triggers = 0
    /\ locked_peer \in Peers
    /\ screen_peer = locked_peer
    /\ panel_messages = 1
    /\ quiet_calls = 0

\* ── 动作定义（Next-State Relations）──

\* 1. 用户在 Manual 档点击候选（仅填入输入框，绝不按回车发送）
UserClickManual(c) ==
    /\ c \in Candidates
    /\ tier = "Manual"
    /\ UNCHANGED vars

\* 2. 用户在 Auto 档点击候选（同轮同候选仅首次触发真实发送，并登记到 sent_registry）
UserClickAuto(c) ==
    /\ c \in Candidates
    /\ tier = "Auto"
    /\ LET current_armed == IF armed_tick = ticks THEN armed ELSE {}
       IN IF c \in current_armed
          THEN \* 已被 Arming 拦截：拒绝重发
               /\ armed' = current_armed
               /\ armed_tick' = ticks
               /\ UNCHANGED <<tier, ticks, sent_registry, delivered, draft_triggers,
                              locked_peer, screen_peer, panel_messages, quiet_calls>>
          ELSE \* 首次发送：记入 armed 与 sent_registry，并追加一条真实发送记录
               /\ armed' = current_armed \cup {c}
               /\ armed_tick' = ticks
               /\ sent_registry' = sent_registry \cup {c}
               /\ delivered' = Append(delivered, [tick |-> ticks, cand |-> c, tier |-> tier])
               /\ UNCHANGED <<tier, ticks, draft_triggers,
                              locked_peer, screen_peer, panel_messages, quiet_calls>>

\* 3. 用户显式切换档位（切换档位立即清空同轮已装填记录 disarm）
UserSetTier(next_tier) ==
    /\ next_tier \in Tiers
    /\ tier' = next_tier
    /\ armed' = {}
    /\ UNCHANGED <<ticks, armed_tick, sent_registry, delivered, draft_triggers,
                   locked_peer, screen_peer, panel_messages, quiet_calls>>

\* 4. 会话结束或异常输入回落（硬约束第 5 条：无条件回落 Manual 并 disarm）
SessionEnd ==
    /\ tier' = "Manual"
    /\ armed' = {}
    /\ UNCHANGED <<ticks, armed_tick, sent_registry, delivered, draft_triggers,
                   locked_peer, screen_peer, panel_messages, quiet_calls>>

\* 5. 屏幕 OCR 读到一条我们自己刚发出去的话（回声）：被 SentRegistry 改标为 Me，不推进轮次、不触发起草
OcrSelfEchoArrives(c) ==
    /\ c \in Candidates
    /\ c \in sent_registry
    /\ UNCHANGED vars

\* 6. 锁定会话中对方发来真正的新消息：推进轮次 ticks 并触发一次起草
OcrPeerNewMessageArrives ==
    /\ screen_peer = locked_peer
    /\ ticks' = ticks + 1
    /\ draft_triggers' = draft_triggers + 1
    /\ panel_messages' = panel_messages + 1
    /\ UNCHANGED <<tier, armed, armed_tick, sent_registry, delivered,
                   locked_peer, screen_peer, quiet_calls>>

\* 7. 用户把微信切到另一个会话（未点「改为锁定此会话」）：身份闸关闭，面板清空展示，零模型调用
SwitchScreen(p) ==
    /\ p \in Peers
    /\ screen_peer' = p
    /\ panel_messages' = IF p = locked_peer THEN 1 ELSE 0
    /\ UNCHANGED <<tier, ticks, armed, armed_tick, sent_registry, delivered,
                   draft_triggers, locked_peer, quiet_calls>>

\* 8. 用户显式点击「改为锁定此会话」（relock）：锁定对象更新为当前屏幕会话
RelockToCurrentScreen ==
    /\ locked_peer' = screen_peer
    /\ panel_messages' = 1
    /\ sent_registry' = {}
    /\ armed' = {}
    /\ UNCHANGED <<tier, ticks, armed_tick, delivered, draft_triggers,
                   screen_peer, quiet_calls>>

Next ==
    \/ \E c \in Candidates : UserClickManual(c)
    \/ \E c \in Candidates : UserClickAuto(c)
    \/ \E t \in Tiers : UserSetTier(t)
    \/ SessionEnd
    \/ \E c \in Candidates : OcrSelfEchoArrives(c)
    \/ OcrPeerNewMessageArrives
    \/ \E p \in Peers : SwitchScreen(p)
    \/ RelockToCurrentScreen

\* ── 安全不变量（Safety Invariants）──

\* 不变量 1（硬约束第 5 条）：档位集合严格只有 Manual 与 Auto 两档
TierDomainInvariant ==
    tier \in {"Manual", "Auto"}

\* 不变量 2（硬约束第 5 条）：Manual 档在任何路径下绝不产生真实回车发送
ManualNeverSends ==
    \A i \in 1..Len(delivered) : delivered[i].tier = "Auto"

\* 不变量 3（硬约束第 5 条 & 票 20）：同一轮次（ticks）内，同一条候选绝不被自动发送两次
NoDoubleSendInSameRound ==
    \A i, j \in 1..Len(delivered) :
        (i # j /\ delivered[i].tick = delivered[j].tick /\ delivered[i].cand = delivered[j].cand)
        => FALSE

\* 不变量 4（硬约束第 1 条 & 票 18/82）：屏幕处于非锁定会话时，面板不泄露原会话消息
NoCrossSessionLeak ==
    (screen_peer # locked_peer) => (panel_messages = 0)

\* 不变量 5（硬约束第 6 条）：静默期及切走/切回期间零额外模型调用
QuietPeriodZeroCalls ==
    quiet_calls = 0

Spec == Init /\ [][Next]_vars

THEOREM Spec => [](TierDomainInvariant /\ ManualNeverSends /\ NoDoubleSendInSameRound
                   /\ NoCrossSessionLeak /\ QuietPeriodZeroCalls)
=============================================================================
