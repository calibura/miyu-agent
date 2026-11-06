# miyu 项目全景（2026-11-06 夜）

一份文档说清四件事：项目是什么、现在到哪了、每一块干什么、接下来按什么顺序做。
数据全部来自当天对工作区的实测（行数、提交、面板清单均可复核），不是凭印象写的。

> 配套文档：
> - 移植交接细节（在途代码怎么接）→ `docs/plan/2026-11-06-purge-handoff.md`
> - purge 子系统本体设计 → `docs/plan/2026-11-06-purge-port.md`
> - 提交纪律与验收硬约束 → `AGENTS.md`（尤其 §19、§5.5、§8.5）

---

## 一、项目是什么

**miyu** 是一个活在终端里的二次元少女 AI 助手（Rust workspace，~35 万行），
开箱即用，支持接入通讯平台。当前版本 `0.7.4`。

仓库是 [SHORiN-KiWATA/miyu-agent](https://github.com/SHORiN-KiWATA/miyu-agent)
的分叉（`calibura` 维护，分叉基点 `5df19e2`，2026-11-02）。上游不再自动跟随；
`main` 留作对照基线，所有本地定制在 `feat/purge-port` 分支。

### 1.1 分层架构（四 crate + CLI 壳）

| crate | 职责 | 规模 |
| --- | --- | --- |
| `miyu-base` | 基础/配置层：路径、AppConfig、i18n、沙盒、purge 配置、模型缓存 | 36,305 行 |
| `miyu-core` | 存储与协议/子系统/传输层：SQLite 会话库、IPC 协议、daemon 生命周期、技能 | 66,887 行 |
| `miyu-engine` | 工具与引擎层：Agent 循环、工具注册表、命令闸、net_guard、purge 注入 | 71,754 行 |
| `miyu-hosts` | 场所与展示层：daemon 宿主、WebUI 服务端、QQ/OneBot 平台、goals/todos | 109,714 行 |
| `src/`（主程序壳） | CLI（TUI/REPL）、config TUI、OOBE、persona、prompts | 73,449 行 |

依赖方向严格单向：`base ← core ← engine ← hosts ← src`。
`test_scripts/arch_dep_check.py` 门禁盯着，跨层引用会被拒。

### 1.2 前端（web/，31,646 行 JS）

WebUI 是 schema 驱动的：`settings-schema.js`（3,646 行）定义设置页全部字段，
`settings.js` 渲染；面板走 `dashboards.js` 注册制。

已上线的面板（11 个）：

| 面板 | 文件 | 干什么 |
| --- | --- | --- |
| affection | `dash-affection.js` | 好感度 |
| goals | `dash-goals.js` | 跨会话长任务目标（暂停/继续/完成，带审计日志） |
| groups | `dash-groups.js` | QQ 群管理 |
| kb | `dash-kb.js` | 知识库 |
| ledger | `dash-ledger.js` + `dash-ledger-manage.js` | 账本 |
| memes | `dash-memes.js` | 表情包管理 |
| memory | `dash-memory.js` | 记忆面板 |
| qq | `dash-qq.js` | QQ 运行状态 |
| scripts | `dash-scripts.js` | 脚本管理 |
| sponsors | `dash-sponsor.js` | 赞助 |
| **purge** | `settings-schema.js` purge 段 | 破甲开关（今天新加，5 个 toggle） |

### 1.3 工具面

模型可用的工具约 40+ 个（`ToolSpec::new` 全仓去重），核心的有：
`run_command`（带 rm 闸与 command_deny 子串名单）、`web_search` / `web_fetch`
（带 SSRF 三件套 net_guard）、`read/write/glob/grep` 文件面、`subagent` 委派、
`send_voice_message`（TTS）、QQ 系（`qq_mention_all`、`qq_withdraw_message`、
`qq_group_manage`…）、记忆系（`remember_fact`、`recall_memories`）、
知识库系（`search_knowledge_base`）、AUR/Arch 系（`aur`、`archwiki_query`…）、
账本（`ledger`）、闹钟（`alarm`）。

### 1.4 平台接入

QQ（OneBot 协议）是目前唯一实装的平台：`platforms/onebot/`（dispatch、identity），
配 `qqa2miyu.service` 桥接进程。语音链路：`miyu-voice` 独立进程（唤醒词/本地识别/
麦克风），TTS 走 `voice_tts.rs`（现支持 MiniMax/Mimo/GPT-SoVITS，openai_compat 在途）。

### 1.5 监管与运维（本机部署形态）

- `miyu.service`（systemd user unit）跑 daemon，`Restart=on-failure`，
  让位退出码 `EX_TEMPFAIL(75)`
- `miyu-guard.timer` 每 30 秒 reclaim：只杀 cgroup 不含 `miyu.service` 的野 daemon，
  过渡态（activating/deactivating）先等
- `systemd-oomd` 监控 `/user.slice/user-1000.slice`（内存压力 60% 持续 30s 就杀
  最大户），编译任务过大时会先被 oomd 收掉而不是拖垮整机
- 配置真相源 `~/.miyu/config/config.jsonc`；改完 `miyu reload` 生效

---

## 二、各子系统当前进度

### 2.1 已完成并装机（本机 `/usr/bin/miyu` = commit `87dad43`，11-06 19:46）

| 子系统 | 状态 | 说明 |
| --- | --- | --- |
| purge 破甲子系统 | ✅ 装机运行 | 注入/命令闸/出站闸/沙盒直通/boost 五层，selfcheck 全绿 |
| purge WebUI 面板 | ✅ 装机 | 设置页 5 个 toggle，ApplyConfig 旗标刷新已修 |
| daemon systemd 感知 | ✅ 装机 | 受监管的 daemon 不再被旧客户端私杀私拉 |
| supervision 入库 | ✅ | unit + guard 文件与已部署版逐字节一致 |
| stand-down 退出码 | ✅ | 实测 75，systemd 正常重拉 |
| GPT-SoVITS 播报 | ✅（11-03 提交） | 主程序侧已支持，`fixed_ref`/`openai_compat` 在途 |
| stt_only 纯转写 | ✅（11-02） | QQ 语音转文字不必开唤醒 |
| goals 面板 + 待办清单 | ✅（10-28） | `next-release-note.md` 里记录着，等发布 |
| OneBot mention_all / 群通知 | ✅（10-28） | |

### 2.2 在途（工作区有代码，未提交）

| 任务 | 状态 | 卡在哪 |
| --- | --- | --- |
| voice 半成品 | 40% | 冻在 `stash@{0}`、`stash@{1}`；pop 会与 purge 面板在
`web/i18n-en.js`、`web/settings-schema.js` 轻微冲突（行区间不相邻，大概率自动合并） |

### 2.2.1 已完成里程碑 M1：出站白名单（commit `f0bdf11`，已推送）

`bypass_net_guard` 从「全放行」收窄为「名单空=全放行 / 有名单=只放行命中者」。
档案：

| 产物 | 位置 |
| --- | --- |
| 白名单槽 | `crates/miyu-engine/src/tools/net_guard_allowlist.rs`（`AllowlistSlot`，RwLock 包 Vec） |
| 条目解析与匹配 | `crates/miyu-base/src/config/purge.rs`（`NetAllowEntry::parse/matches`） |
| 放行判定 | `net_guard.rs::bypass_gate`（三态） |
| 写入口 ×3 | `web/server.rs` 启动、`ipc_server.rs` ReloadConfig、`config_api.rs` rebuild_for_config |
| 运行时上报 | `GetStatus` 的 `purge.net_allowlist_len` |
| CLI | `miyu purge status` 逐条列名单，空名单明说「放行不设限」 |
| WebUI | `settings-schema.js` 的 `purge.net_allowlist`（string-list）+ 2 条 i18n |
| 测试 | 7 条全绿（slot 三态/整体替换/大小写尾点；bypass_gate 三态 + 子域名不通配 + 总闸关） |

**语义红线**：名单空必须维持全放行，否则老配置升级后静默断网。
`allowlist_narrows_the_bypass_rather_than_replacing_it` 钉住这条。

### 2.3 未开工（已在交接单排好价值序）

| 任务 | 量级 | 一句话 |
| --- | --- | --- |
| 规则页 | 中 | 扫描 cwd→projectRoot 的 AGENTS/CLAUDE.md，三档策略，拼进注入 |
| 审计与备份 | 小 | `purge-audit.jsonl` + `purge edit --reset` 带备份 |
| 自动重试/续跑 | 小 | dsh 的 `autoRetry`/`autoContinue`（continueText=“继续”，上限 3） |
| 演练台 | **大** | 红队子系统：10 标签页、5 角色、56 工具、SQLite 资产库、23 技能。
**单独立项，先出方案拍板，别顺手做** |

---

## 三、dsh-purge 插件移植专项

### 3.1 对比结论（全功能，已核到源码级）

对照物：`dsh-purge-community 1.6.60`（本机 `~/.dsh/dsh-purge-community/`，tarball 已解包）。

| 能力 | dsh-purge | miyu | 结论 |
| --- | --- | --- | --- |
| 注入正文 | 225 行每步重贴 | 18,625 字节会话期冻结 | ✅ 对等，miyu 更稳（冻结优于重贴） |
| 命令闸放行 | `hooksDenyBypass` | `bypass_command_guards` | ✅ 对等 |
| 出站闸放行 | 补丁拆 net-scope | `bypass_net_guard` | ✅ 对等 |
| 沙盒直通 | 补丁拆沙箱 | `sandbox_passthrough` | ✅ 对等 |
| boost 强化段 | 有 | `BOOST_SECTION` 常量 | ✅ 对等 |
| **自己的服务器白名单** | 有（UI 可编辑） | ❌ 在途 60% | 🔧 本轮正在补 |
| **规则页**（AGENTS/CLAUDE.md 管理） | 有 | ❌ 无 | 📋 已排（5.1） |
| **技能库管理** | 有 | ❌（miyu 有独立 skills 系统，不经 purge 管） | ➖ 架构不同，可不做 |
| **演练台**（红队子系统） | 有（10 标签） | ❌ 无 | 📋 已排（5.3，单独立项） |
| 状态页（补丁统计/版本/更新） | 有 | 文本版 `purge status` | ➖ miyu 零补丁架构不需要 |
| 审计/快照回滚 | 有 | ❌ 无 | 📋 已排（5.2） |
| 自动更新 | 有 | ❌（git 拉取即可） | ➖ 不需要 |
| 加密槽默认稿 | 有 | ❌ 明文模板 | ➖ 单机场景收益低 |
| autoRetry / autoContinue | 有 | ❌ 无 | 📋 已排（5.4） |

### 3.2 移植的架构差异（为什么 miyu 不需要 67 条补丁）

dsh 是插件宿主，purge 要拆审批/沙箱/过滤器就得 patch 宿主源码（67 条）；
miyu 宿主是自己的，同一批能力收敛成 5 个配置开关。**关着时与上游逐字节一致**，
这是移植时钉死的契约（`purge_off_reproduces_the_upstream_prompt_byte_for_byte` 测试）。

### 3.3 白名单（在途中）的设计决策

- `net_allowlist: Vec<String>`，条目支持 `IP`、`主机名`、`账号@主机`（账号剥掉不参与判定）
- 精确匹配，**不做后缀通配**（`example.com` 不放行 `sub.example.com`）
- 名单空 = 全放行（历史行为，老配置零变化）；名单非空 = 只放行命中者
- 接线三处：启动 / `ReloadConfig` / `rebuild_for_config`（与 `set_bypassed` 同点）
- selfcheck 经 IPC `GetStatus` 查运行时条数，不读配置自欺

详细接法（含 `AllowlistSlot` 骨架代码、报错原文、改动点行号）
→ `docs/plan/2026-11-06-purge-handoff.md` 第四节。

---

## 四、后续计划（执行顺序）

前置动作（接手者第一件事）：

1. `git push origin feat/purge-port` —— 18 个 commit、零 tag、从未推送，先保住历史
2. 向用户追认今天 4 个越过验收的 commit（`a341052`、`2709c98`、`429a3f6`、`87dad43`）
3. 补 `next-release-note.md` 草稿（缺 purge 移植、WebUI 面板、systemd 感知、
   supervision 入库、stand-down 75 五项）

然后按序执行：

### P1 白名单收尾（在途，半天）

按 handoff 第四节七步走完：建模块 → 接三处写入口 → IPC 上报 → schema/i18n →
CLI → 测试。验收命令在 handoff 第十节，全部带内存围栏。

### P2 voice 半成品完成（1-2 天）

pop 两个 stash → 修 `voice_tts` 3 条红测试 → 清 `goals_overview_json` warning →
config TUI 接线（`src/config_tui/voice.rs` 已改 170 行）→ 判断 `miyu-voice` 是否
需要重编（stash 动了 `voice/worker.rs`，要）。stash pop 后**必须重跑**
`check-webui-i18n.py`。

### P3 purge 规则页（2-3 天）

扫描器（32 层上限、`.git`/`.hg`/`.svn` 为界）→ 三档策略 → 状态文件
`purge-rules.json` + 拼装 `purge-rules.md` → 注入位置（`<purge-mode>` 之后、
`<purge-boost>` 之前）→ `miyu purge rules` CLI → WebUI 勾选面板。

### P4 审计与备份（1 天）

`purge-audit.jsonl`（配置变更/注入改写各记一条）+ `purge edit --reset` 带时间戳备份。

### P5 自动重试/续跑（1 天）

回合收尾处挂 autoRetry 计数；TUI 空闲时发 `continueText`（上限 `continueMax`）。
配置默认关。

### P6 发布 0.7.5（半天）

版本三处同步（Cargo.toml / Cargo.lock / 打包配方）→ release commit → tag →
按 `miyu-release-workflow.md` 走构建与六平台 smoke → 本机 pacman 轮换。
**注意**：tag 与发布要用户明确授权（AGENTS §7.2 的发版链）。

### P7 演练台（立项后单排，2-3 周量级）

出方案文档拍板后才动：SQLite 资产库 schema、5 角色 subagent 化、56 工具注册、
WebUI 右侧栏。dsh 的 `lib/redteam/`（17,599 行 JS）与 `skills/redteam/`（23 个技能）
是现成参照。

---

## 五、纪律红线（每天都要碰的）

1. **验收后才 commit**（AGENTS §19）：内容做完 → 给验收流程 → 用户点头 → commit →
   同步 `next-release-note.md`
2. **编译/测试必须带围栏**（§8.5）：`systemd-run --user --scope
   -p MemoryMax=20G -p MemorySwapMax=0`，同一时刻一个 cargo。
   本机 GPT-SoVITS 常驻 8.3G，裸跑 cargo 会把 31G 顶穿——今天为此硬复位过一次
3. **黑盒实测用 `MIYU_HOME` 沙箱**（§5.4），普通 CLI 未知子命令会把参数当对话
   发给生产 daemon
4. **`git add` 别用 `-A`**（§8.8）：会把别人/别的任务的在途半成品扫进提交
5. **文件规模**：目标 800 行 / 上限 1500 / 红线 2000（`refactor-size-report.py` 门禁）
6. **模型可见面无 CJK**（`check-model-english.sh`）；WebUI 文案中文写代码、
   英文补词典（`check-webui-i18n.py`）

## 六、一句话总结

骨架全部就位且在跑：purge 五层破甲、WebUI 面板、systemd 监管、oomd 护栏。
在途的白名单编译是断的（缺一个 80 行的模块，骨架已在交接单里）；
voice 半成品在 stash 里等收尾。按 P1→P6 推进，P7 演练台单独立项。
