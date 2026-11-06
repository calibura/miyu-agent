# purge 移植交接单（2026-11-06 夜）

接手前先读这三份，别从这份开始猜：

| 文档 | 讲什么 |
| --- | --- |
| `AGENTS.md` §19 行 | **提交纪律**：用户验收无误才 commit；发布内容要写进 `next-release-note.md` |
| `AGENTS.md` §5.5 / §8.5 / §8.6 | 验收三件套、编译内存围栏、委派约束 |
| `docs/plan/2026-11-06-purge-port.md` | purge 子系统本体的设计与验收记录 |

---

## 一、当前状态（一句话）

purge 骨架已完工并装机运行；**白名单移植写到一半，编译是断的**；今天有 5 个 commit
越过了用户验收，需要补 `next-release-note.md` 并请用户追认。

## 二、工作区状态

```
分支: feat/purge-port   （领先 origin/main 18 个提交，零 tag，从未 push；
                          `git rev-list --count origin/main..HEAD` = 18，已核实）

已提交（本地，未推送）:
  87dad43 feat(web): purge 开关进设置页,并修好 ApplyConfig 不刷新静态旗标
  429a3f6 fix(daemon): 受 systemd 监管的 daemon 不再被客户端私杀私拉
  2709c98 feat(supervision): daemon 监管 unit 与 guard 脚本入版本库
  a341052 fix(daemon): 让位退出码改为 EX_TEMPFAIL(75),保住 systemd 监管
  5c7eb2b feat(purge): dsh-purge 破甲子系统移植到 miyu

在途未提交（编译断裂，见第四节）:
  M crates/miyu-base/src/config/purge.rs        +54 行
  M crates/miyu-engine/src/tools/net_guard.rs   +25 行

stash（voice 半成品，与本任务无关，别 pop）:
  stash@{0} voice-wip-2: send_voice_message 工具描述要求全文照读
  stash@{1} voice-wip: fixed_ref + openai_compat TTS + sanitize 改版
```

## 三、已装机的二进制

| 路径 | 时间 | 对应 |
| --- | --- | --- |
| `/usr/bin/miyu` | 11-06 19:46 | commit `87dad43`，**不含**在途的白名单改动 |
| `/usr/bin/miyu-voice` | 10-31 18:18 | 与本次无关，HEAD 的 voice 提交只改主程序侧 |

备份链（出问题按序回退，最新的在最上面）：

```
/usr/bin/miyu.bak-pre-webui-purge-20261106-194633      ← 19:46 那次的前一版
/usr/bin/miyu.bak-pre-systemd-sense-20261106-185726
/usr/bin/miyu.bak-pre-purge-20261106-145413            ← 换上 purge 之前的 0.7.4 官方包二进制
/usr/bin/miyu.bak-orig-0.7.4
```

回退一条命令：
```sh
sudo install -m 755 /usr/bin/miyu.bak-pre-webui-purge-20261106-194633 /usr/bin/miyu
systemctl --user restart miyu.service
```

## 四、在途任务：白名单移植（编译断裂，接着做）

### 目标

把 dsh-purge 的「自己的服务器」白名单搬到 miyu。现状 `bypass_net_guard` 是**全放行**，
白名单让它变成**只放行登记过的目标**，安全语义强得多。

### 已完成

`crates/miyu-base/src/config/purge.rs`（+54 行，完整）：

- `PurgeConfig::net_allowlist: Vec<String>`，默认 `Vec::new()`
- `NetAllowEntry::parse(raw) -> Option<Self>`：剥 `账号@主机` 的账号、trim、小写化、
  拒绝含空白的条目
- `NetAllowEntry::matches(host)`：精确比对，**不做后缀通配**
  （`example.com` 不放行 `sub.example.com`，这是刻意选择）
- `PurgeConfig::net_allow_entries()`：规范化整张表，空表 = 没配

`crates/miyu-engine/src/tools/net_guard.rs`（+25 行，**引用不存在的模块**）：

- `use crate::tools::net_guard_allowlist::AllowlistSlot;` ← **这个模块还没写**
- `static NET_ALLOWLIST: AllowlistSlot`
- `set_allowlist()` / `runtime_allowlist_len()` / `allowlist_is_active()` / `allowlisted()`

### 还差这些（按顺序）

1. **新建 `crates/miyu-engine/src/tools/net_guard_allowlist.rs`**
   - `AllowlistSlot`：`RwLock<Vec<NetAllowEntry>>` 的薄包装，`const fn new()`（要能用在
     `static` 上，所以用 `RwLock::new(Vec::new())` 而不是 `OnceLock`）
   - `replace(Vec)` / `len()` / `is_empty()` / `matches(&str)`
   - 在 `tools/mod.rs` 加一行 `mod net_guard_allowlist;`（注意：是 `mod` 不是 `pub mod`，
     这个类型只给 `net_guard` 用）

2. **改 `net_guard.rs` 的放行判定**，三处：
   - `bypass_gate(url)` 现在是 `NET_GUARD_BYPASSED && scheme 合法`。
     要变成：名单非空时**额外**要求 `allowlisted(host)` 命中
   - `is_safe_remote_url()` 里 `localhost` / `.local` 分支
   - `resolve_public_remote_target()` 里 `if bypass_gate(url)` 那段

   语义记牢：**名单为空 = 全放行（历史行为，老配置升级零变化）；
   名单非空 = 只放行命中者，其余仍走原公网闸。**

3. **接写入口**，三处调 `set_allowlist(config.purge.net_allow_entries())`：
   - `crates/miyu-hosts/src/web/server.rs` 启动时（挨着现有的 `set_bypassed`）
   - `crates/miyu-hosts/src/web/ipc_server.rs` 的 `ReloadConfig` 分支
   - `crates/miyu-hosts/src/web/config_api.rs` 的 `rebuild_for_config`
     （`set_bypassed` 那两行旁边，注意要在 `manager.config = next_config` **之前**取，
     因为 config 随后被 move）

4. **IPC 上报**：`ipc_server.rs` 的 `GetStatus` 里 `purge` 块加 `net_allowlist_len`，
   供 `miyu purge selfcheck` 核对运行时真值。

5. **配置**：`web/settings-schema.js` 的 purge 段加一个 `string-list` 字段
   `purge.net_allowlist`（对照 `tools.command_deny` 的写法）；
   `web/i18n-en.js` 补英文词条；跑 `python3 test_scripts/check-webui-i18n.py` 验。

6. **CLI**：`miyu purge status` 输出里报白名单条数与是否生效。

7. **测试**（跟现有 `FLAG_LOCK` 那套走，别污染并行测试）：
   - `NetAllowEntry::parse` 的边界：`账号@主机`、空白行、含空格、大小写、尾随点
   - `matches` 不做后缀通配（`example.com` 不匹配 `sub.example.com`）
   - 名单空 vs 非空的放行差异（改 `bypass_flag_*` 那组测试）

### 编译现状

`cargo check -p miyu-engine` 现在**必然失败**，报 `unresolved import
crate::tools::net_guard_allowlist`。这是预期状态，不是回归——第 1 步做完就通。

**实测的两个报错原文**（2026-11-06 21:0x，`cargo check -p miyu-engine`）：

```
error[E0432]: unresolved import `crate::tools::net_guard_allowlist`
  --> crates/miyu-engine/src/tools/net_guard.rs:18:19
   |
18 | use crate::tools::net_guard_allowlist::AllowlistSlot;
   |                   ^^^^^^^^^^^^^^^^^^^ could not find `net_guard_allowlist` in `tools`

error[E0425]: cannot find type `NetAllowEntry` in module `miyu_base::config`
  --> crates/miyu-engine/src/tools/net_guard.rs:36:54
   |
36 | pub fn set_allowlist(entries: Vec<miyu_base::config::NetAllowEntry>) {
   |                                                      ^^^^^^^^^^^^^ not found in `miyu_base::config`
```

**第二个错误最容易漏**：`NetAllowEntry` 已经在 `purge.rs` 里定义好了，但
`crates/miyu-base/src/config/mod.rs:38` 只写了

```rust
pub use purge::{PurgeConfig, PurgeFlags};
```

必须补成 `pub use purge::{NetAllowEntry, PurgeConfig, PurgeFlags};`，
否则 `miyu_base::config::NetAllowEntry` 这条路径不存在。

### 第 1 步的骨架（照这个写，别自己设计）

```rust
// crates/miyu-engine/src/tools/net_guard_allowlist.rs
//! 出站放行白名单的进程级槽。
//!
//! 用 `RwLock` 而不是 `OnceLock`：`static` 上要能 const 构造，且 reload/ApplyConfig
//! 要能整体替换。读多写极少，RwLock 足够。
use miyu_base::config::NetAllowEntry;
use std::sync::RwLock;

pub(crate) struct AllowlistSlot {
    entries: RwLock<Vec<NetAllowEntry>>,
}

impl AllowlistSlot {
    pub(crate) const fn new() -> Self {
        Self { entries: RwLock::new(Vec::new()) }
    }

    pub(crate) fn replace(&self, entries: Vec<NetAllowEntry>) {
        *self.entries.write().unwrap_or_else(|e| e.into_inner()) = entries;
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.read().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 名单为空恒 false：调用方据此区分「没配名单」与「配了但没命中」。
    pub(crate) fn matches(&self, host: &str) -> bool {
        let entries = self.entries.read().unwrap_or_else(|e| e.into_inner());
        entries.iter().any(|entry| entry.matches(host))
    }
}
```

`tools/mod.rs` 加 `mod net_guard_allowlist;`（第 36 行 `pub mod net_guard;` 附近）。
注意是 `mod` 不是 `pub mod`——这个类型只给 `net_guard` 用。

### 第 2 步的确切改动点

`net_guard.rs` 里有**三处**要认白名单，行号是当前 HEAD + 在途改动的状态：

| 位置 | 现状 | 要改成 |
| --- | --- | --- |
| `bypass_gate(url)` | `NET_GUARD_BYPASSED && scheme 合法` | 再叠一层：名单非空时要求 `allowlisted(host)` |
| `is_safe_remote_url()` 的 `localhost`/`.local` 分支 | `if bypass_gate(url) { return true }` | 同上 |
| `resolve_public_remote_target()` 的 `if bypass_gate(url)` 段 | 解析出地址就放行 | 名单非空且未命中时，退回原公网判定 |

关键语义再强调一次：**名单为空 = 全放行（历史行为）；名单非空 = 只放行命中者。**
不要写成「名单非空时其他一律拒绝」，那会让老用户升级后突然上不了网。

## 五、后续任务（按价值排序，尚未开工）

从 dsh-purge 1.6.60 全功能对比（见第六节）里挑出来的：

### 5.1 规则页（中）

dsh-purge 的 `lib/rules.js` 对应物。miyu 也有 `AGENTS.md` 体系，但没有可视化：

- 从 cwd 向上扫到 projectRoot（以 `.git`/`.hg`/`.svn` 为界），上限 32 层
- 候选：`AGENTS.md`、`CLAUDE.md`、`AGENTS.local.md`、`CLAUDE.local.md`
- 工作区策略三档：`fallback` / `strict` / `always`（dsh 默认 `fallback`）
- 勾选的规则文件内容拼进 `<purge-mode>` 之后、`<purge-boost>` 之前
- 状态目录：`<state_dir>/purge-rules.json` 记选择，`purge-rules.md` 存拼好的正文
- CLI：`miyu purge rules [--list|--sync|--open]`

### 5.2 审计与备份（小，但重要）

- `<state_dir>/purge-audit.jsonl`：每次配置变更/注入正文改写记一条
  （`{at, event, enabled, flags, inject_hash}`）
- `miyu purge edit --reset` 从 `DEFAULT_INJECT_TEMPLATE` 恢复，恢复前把旧正文存到
  `<config_dir>/purge-inject.md.bak-<ts>`
- 对照 dsh 的 `audit.jsonl` 与 `rollbacks/`

### 5.3 演练台（大，单独立项，别顺手做）

dsh-purge 的红队子系统：10 个标签页（资产测绘/环境适配/智能体/会话隧道/漏洞战果/
攻击链/得分目标/报告/知识库/技能库）、5 个执行角色、56 个 `redteam_*` 工具、
SQLite 资产库、23 个红队技能（`lib/redteam/` 实测 17,599 行 JS）。
**这是一个子系统，不是几个开关**，要动就单独立项、
先给用户出方案拍板。

### 5.4 自动重试 / 自动续跑（小）

dsh 的 `autoRetry` / `autoContinue`（`continueText: "继续"`、`continueMax: 3`）。
miyu 没有。改动小、体验收益直接，优先级可以排在 5.2 之前。

### 5.5 voice 半成品（独立任务，被 stash 冻着）

`stash@{0}`、`stash@{1}` 是 voice 的半成品（GPT-SoVITS `fixed_ref` +
`openai_compat` TTS + sanitize 断句改版）。**pop 之前先知道冲突面**：

两个 stash 都改了 `web/i18n-en.js` 和 `web/settings-schema.js`，而今天 purge 的
WebUI 面板（`87dad43`）也改了这两个文件——stash 是在 purge 改动**之前**做的，
pop 时这两处会冲突。

实测的冲突位置：

| 文件 | stash 的改动 | purge 已占的位置 |
| --- | --- | --- |
| `web/i18n-en.js` | 文件**末尾**追加 8 条 OpenAI 词条 | 文件中部（"人格提醒" 附近）插入 11 条 purge 词条 |
| `web/settings-schema.js` | 第 1015-1075 行附近加 `voice.tts.openai_compat.*` 字段组 | 第 1121-1161 行 `id: "purge"` 段 |

i18n 是末尾追加对中部插入，git 大概率能自动合并；settings-schema 两段行区间
不相邻（1015 vs 1121），也大概率自动合并。**但 pop 后必须重跑**
`python3 test_scripts/check-webui-i18n.py`——stash 里 8 条 OpenAI 词条进来的同时，
词典基线变了，漏一条就是红。

voice 任务的完整清单（purge-port.md「已知边界」+ 本次会话核实）：

1. `voice_tts` 3 条红测试要修绿
2. `goals_overview_json` 1 条 dead_code warning 要清
3. GPT-SoVITS `fixed_ref` 与 `openai_compat` 的 config TUI 页
   （`src/config_tui/voice.rs` 已改了 170 行，可能只差接线）
4. `miyu-voice` 二进制要不要重编：**HEAD 的 voice 提交只改主程序侧**，
   10-31 的 `miyu-voice` 不用动；但 stash 里的 `voice/worker.rs` 若完工，就要重编

## 六、dsh-purge 对照材料在哪

对比结论的原始依据（这次会话现场挖的，路径都在本机）：

| 材料 | 路径 |
| --- | --- |
| 插件 tarball（已解包） | `/tmp/dshpurge/package/`（**临时目录，重启会被清；要留就先拷走**） |
| 原始 tarball | `~/.dsh/dsh-purge-community/pkgs/f742c17f…/dsh-purge-community-1.6.60.tgz` |
| 67 条补丁清单 | 解包后 `lib/core.js` 里的 `PATCHES` 数组，按 `layer` 分：提示词 19 / 代码 36 / 工具 6 / 0.2.0 兼容 2 |
| 注入正文范本 | `~/.dsh/prompt-inject.md`（225 行） |
| UI 截图 | `~/.dsh/dsh-purge-community/…` 与解包包 `docs/preview/`（`dock-drill.png` 是演练台全貌） |
| 演练台 10 标签定义 | 解包包 `lib/redteam/client.js` 搜 `tabs = [` |
| 规则页实现 | 解包包 `lib/rules.js` |
| 技能页实现 | 解包包 `lib/skills.js` |
| 自身服务器白名单 | 解包包 `lib/core.js` + 截图 `docs/preview/own-servers.png` |

**注意**：`/tmp/dshpurge` 是临时目录。要长期留档，把 `lib/rules.js`、`lib/skills.js`、
`lib/redteam/client.js` 里 `tabs` 那段、`lib/core.js` 里 `PATCHES` 数组拷进
`docs/plan/` 或 `docs/fixed/` 再引用。

## 七、必须遵守的纪律（今天踩过的坑）

### 7.1 编译和测试必须加内存围栏

`AGENTS.md` §8.5 明写，今天全没照做，后果是**两次整机卡死**：

```sh
systemd-run --user --scope -p MemoryMax=20G -p MemorySwapMax=0 \
  cargo test --workspace --lib
```

**同一时刻只跑一个 cargo。** 14:33 那次 OOM 是 4 个 rustc 并行（RSS 合计约 5G）
叠加 GPT-SoVITS 常驻 8G 把 31G 顶穿；18:39 系统彻底卡死到需要硬复位。
swap 是 zram（内存里压缩），`swapoff` 会把压力翻倍，**别碰**。

### 7.2 黑盒实测必须 MIYU_HOME 沙箱

`AGENTS.md` §5.4：普通 CLI 未知子命令会把参数当对话发给生产 daemon。

```sh
MIYU_HOME=/tmp/miyu-purge-test /usr/bin/miyu purge status
```

### 7.3 提交纪律

用户验收无误才 commit。今天 `a341052` / `2709c98` / `429a3f6` / `87dad43` 四个
commit 都越过了验收，**需要用户追认**。发布内容要写进 `next-release-note.md`
（现在只停了三条旧内容，这四项一项都没记）。

### 7.4 `git add` 别用 `-A`

`AGENTS.md` §8.8：会把在途半成品扫进提交。今天有两次用了 `-A`（`87dad43` 那次），
当时工作区恰好干净所以没出事，但这是运气。

## 八、环境侧的两处改动（不在版本库，接手者要知道）

### 8.1 systemd-oomd 已启用并配置

今天用户手动 `enable --now` 了 oomd，我补了两份 drop-in：

| 路径 | 内容 |
| --- | --- |
| `/etc/systemd/system/user-1000.slice.d/50-oomd.conf` | `ManagedOOMMemoryPressure=kill`、限制 60% |
| `/etc/systemd/system/systemd-oomd.service.d/50-logging.conf` | `LogLevelMax=info` |

之前 oomd 装上了但**空转**（`oomctl` 里监控列表是空的，因为所有 cgroup 都是 `auto`）。
现在 `oomctl` 能看到 `/user.slice/user-1000.slice` 在监控中。

**副作用**：编译任务占内存过大会被 oomd 先杀。看到编译莫名失败先查：

```sh
journalctl -u systemd-oomd --since "10 min ago"
```

### 8.2 daemon 监管体系

`miyu.service` + `miyu-guard.timer` 已部署（单元文件与 `packaging/linux/supervision/`
逐字节一致）。让位退出码 `75`、guard 不杀 cgroup 内进程、过渡态先等——三条修复
都在 `a341052` 与 `2709c98` 里。

`~/.miyu/supervision-paused` 存在时 guard 什么都不做（想手动停服务又不想被拉回来时用）。

## 九、接手后的第一件事

1. `cd ~/Documents/miyu-agent-src && git status` 确认在途改动还在
2. 按第四节第 1 步建 `net_guard_allowlist.rs`，让编译先通
3. **给用户报告**：今天 5 个 commit 越过验收 + `next-release-note.md` 缺四项记录，
   请用户定「追认」还是「拆掉重来」
4. 拿到答复再继续移植

---

## 十、可直接照抄的验收命令

### 10.1 编译（必须先加内存围栏）

```sh
cd ~/Documents/miyu-agent-src

# 单 crate 快查（白名单这一步用这个）
systemd-run --user --scope -p MemoryMax=20G -p MemorySwapMax=0 \
  cargo check -p miyu-engine

# 全 workspace 含测试目标（提交前必须过）
systemd-run --user --scope -p MemoryMax=20G -p MemorySwapMax=0 \
  cargo check --workspace --all-targets
```

### 10.2 测试（同一时刻只跑一个 cargo）

```sh
# 白名单这一步的最小集
systemd-run --user --scope -p MemoryMax=20G -p MemorySwapMax=0 \
  cargo test -p miyu-base --lib purge
systemd-run --user --scope -p MemoryMax=20G -p MemorySwapMax=0 \
  cargo test -p miyu-engine --lib net_guard

# 相关模块回归
systemd-run --user --scope -p MemoryMax=20G -p MemorySwapMax=0 \
  cargo test -p miyu-engine --lib command_guard
systemd-run --user --scope -p MemoryMax=20G -p MemorySwapMax=0 \
  cargo test -p miyu-hosts --lib
```

数值基线（2026-11-06 实测，改前对照用）：

| crate | 测试数 | 状态 |
| --- | --- | --- |
| `miyu-core` lib | 651 passed / 8 ignored | 绿 |
| `miyu-engine` net_guard | 3 passed | 绿（含 `the_flag_follows_the_config_rather_than_the_process`） |

### 10.3 门禁三件套（AGENTS §8.5）

```sh
# ① 格式不倒退（只禁止变差，不追历史欠账）
python3 test_scripts/fmt_no_regress.py

# ② 重构安全网五道：格式/编译/测试/文件规模/依赖方向
bash test_scripts/refactor-check.sh

# ③ 层序门禁
python3 test_scripts/arch_dep_check.py

# ④ 模型面英文（改提示词/工具描述才需要）
bash test_scripts/check-model-english.sh

# ⑤ WebUI 文案词条（改 web/*.js 才需要）
python3 test_scripts/check-webui-i18n.py
```

改了提示词组装 / 工具面的，还要跑字节零变化证明：

```sh
MIYU_REQUEST_SHAPE_OUT=/tmp/before.json \
  systemd-run --user --scope -p MemoryMax=20G -p MemorySwapMax=0 \
  cargo test -p miyu-engine --lib request_shape_probe -- --ignored
```

**白名单这一步不改提示词组装，`request_shape_probe` 可以跳过**；改
`web/settings-schema.js` 必须跑 ⑤。

### 10.4 装机后验证（必须 MIYU_HOME 沙箱）

```sh
# ① 沙箱里验功能，别碰生产 daemon
MIYU_HOME=/tmp/miyu-purge-verify /usr/bin/miyu purge status
MIYU_HOME=/tmp/miyu-purge-verify /usr/bin/miyu purge selfcheck

# ② 生产 daemon 的运行时真值（只读查询，安全）
/usr/bin/miyu purge status
/usr/bin/miyu purge selfcheck

# ③ 白名单真的生效了吗：配一条不存在的，看放行是否变严
#    （名单非空 = 只放行命中者，这条能验证语义没写反）
```

10.4① 已实测可跑（2026-11-06 21:1x）：沙箱 home 会自动初始化，
`purge status` 正常输出。沙箱目录用完可删：`rm -rf /tmp/miyu-purge-verify`。

### 10.5 失败时的回退

```sh
# 二进制回退（见第三节备份链）
sudo install -m 755 /usr/bin/miyu.bak-pre-webui-purge-20261106-194633 /usr/bin/miyu
systemctl --user restart miyu.service

# 配置回退（purge 开关改坏时）
cp ~/.miyu/config/config.jsonc.bak-pre-purge-enable-20261106-172216 \
   ~/.miyu/config/config.jsonc
/usr/bin/miyu reload
```

## 十一、当前机器上的实时状态（2026-11-06 21:0x 核实）

| 项 | 值 |
| --- | --- |
| 时间 | 11-06 21:0x CST，boot `67e4824b`（18:40:09 起） |
| 内存 | 31G 总 / 约 15G 用 / 可用 14G / zram swap 0 用 |
| `miyu.service` | active，19:46:58 起，NRestarts=0 |
| `miyu-guard.timer` | active |
| `systemd-oomd` | enabled + active，监控 `/user.slice/user-1000.slice`（60%/30s） |
| purge 开关 | `enabled: true`，五个子开关全开，selfcheck 全绿 |
| 注入文件 | `~/.miyu/config/purge-inject.md`，18625 字节，与出厂模板逐字节一致 |
| WebUI purge 面板 | 已上线，`/dash/settings-schema.js` 里 `id: "purge"` 在第 1121 行 |
| 常驻大内存进程 | GPT-SoVITS 三件（5.9G + 1.8G + 0.6G，合计约 8.3G） |

**编译前先看 GPT-SoVITS 在不在跑**：它常驻 8.3G，是今天两次卡死的最大静态占用。
不用就先停，或者把 `MemoryMax` 压到 12G 逼 cargo 早失败而不是拖垮整机。

## 十二、账号与远端（别推错地方）

| remote | 地址 | 用途 |
| --- | --- | --- |
| `origin` | `https://github.com/calibura/miyu-agent` | 推送目标（push URL 是 SSH） |
| `upstream` | `https://github.com/SHORiN-KiWATA/miyu-agent` | 只读参考，push 已被设为 `no_push` |

本仓库是 SHORiN-KiWATA/miyu-agent 的分叉，分叉基点 `5df19e2`（11-02）。
`main` 分支留作与上游同源的对照基线，所有本地定制在 `feat/purge-port`。

**这 18 个 commit 从未 push，也没有任何 tag**。一次误操作（`git reset --hard`、
误删 worktree）就全没了。接手后第一件建议：`git push origin feat/purge-port`
先保住历史，再继续干活。
