# 10: 推理接缝——三段式与它的降级路径

**What to build:** 一次可观察的完整行为：对方来一条新消息 → 程序读出意图 →
给出三条排序过的候选 → 推荐那条在最上面。

三段式：**判断 → 带着判断起草 → 排序**。

关键是**降级路径也要能用**：判断服务限流时，仍然拿到候选（退回盲起草 + 一次合问）；
排序挂了就按第一条推荐，判断摘要照常显示；起草返回空候选时报一个
说清楚的错误，而不是崩在取第一条上。

推理接缝只换模型传输（假实现返回预置 JSON），所以测试不联网、不花钱、可确定性重跑。

**Blocked by:** 04

**Status:** done（`8e94cfb` 接缝 + 三条降级路径 + 静默期零调用；
`ba66d3c` 真实 HTTP 传输 ureq，key 只从凭据管理器读、不进日志；
`cabc323` 起 `TurnContext` 另带 `images` 与分段计数。
验收项已在下方**逐条勾上并标注对应测试**）

- [x] `Message[]` + 配置 → `DraftSet`，候选按胜出概率排序，推荐条置顶
      —— `happy_path_runs_all_three_stages` + `DraftSet::sorted/recommended`
- [x] 判断调用失败时，仍然返回候选，且走的是盲起草路径
      —— `judge_failure_degrades_to_blind_but_still_yields_candidates`
- [x] 排序调用失败时，按第一条推荐，判断摘要照常显示
      —— `rank_failure_falls_back_to_probability_but_keeps_intent`
- [x] 起草返回空候选时抛出明确错误，不 panic、不返回空结果
      —— `empty_draft_is_a_clear_error_not_a_panic`
- [x] 意图结果在界面上是**可读的**（建议动作 / 可能意图 / 对方可能需要 / 紧张度），
      不是黑盒标签 —— `Intent::summary()` 直接生成那行字，`blind` 时说「盲拟」而不是装懂
- [x] 静默期（没有新消息）下**零模型调用**，有测试用计数断言锁住
      —— `quiet_period_makes_zero_model_calls`；
      另有一条**每轮重新调**的测试 `each_turn_calls_the_models_again_on_purpose`，
      它是故意的：缓存判断会在用户改窗口大小后变成过期的
- [x] 候选清洗生效：剥掉编号、方括号、引号、照抄的 "me:" 前缀，
      不加句尾句号 —— `candidates_are_cleaned_after_ranking_not_before`
      （顺序有讲究：排序阶段返回的新候选会绕过起草后的清洗）
- [x] 整个推理接缝的测试不联网、不需要 API key
      —— `the_seam_only_depends_on_the_trait_not_on_any_client`；
      真实传输另用 `FakeHttp` 断言请求形状，同样不联网