# purge 子系统移植验收说明

dsh-purge（DeepSeek Harness 的「破甲」插件）向 miyu 的移植。dsh 那边是
「80 条补丁写进宿主源码 + prompt-inject.md 每步重贴 + shim + 面板」；miyu 是
自己的宿主，不需要补丁层，移植的是它的**开关层**：配置驱动的注入、闸放行与
沙盒直通。配置里没有 `[purge]` 段时，每个挂接点字节与上游完全一致（零行为
变化）。

## 开了什么（对照 dsh-purge 的五层）

| dsh-purge | miyu 移植 |
| --- | --- |
| 宿主补丁 ×80（写 DSH 源码） | 不需要——宿主自有，改为 `config.jsonc` 的 `[purge]` 段 |
| `prompt-inject.md` 注入 | `<config>/purge-inject.md`，system 末尾 `<purge-mode>` 段，**会话期字节冻结** |
| boost 尾部强化段 | `BOOST_SECTION` 常量（`<purge-boost>`），追加在段后 |
| 审批/沙箱/fs 围栏放行 | `bypass_command_guards`（rm 闸+子串名单）/ `bypass_net_guard`（SSRF 三件套）/ `sandbox_passthrough`（Landlock 直通） |
| selfcheck / probe CLI | `miyu purge status / selfcheck / edit / probe` |

## 配置（config.jsonc）

```jsonc
"purge": {
  "enabled": true,                 // 总开关
  "bypass_command_guards": true,   // run_command 的 rm 闸 + command_deny 退位
  "bypass_net_guard": true,        // web_images 下载与链接预览的公网闸退位
  "sandbox_passthrough": true,     // 子进程不再装 Landlock 规则集
  "boost": true                    // 尾部强化段
}
```

默认值即「开子开关」：`enabled` 是唯一要手动打开的。总开关关着，子开关一律
无效（有测试钉住）。

## 文件

| 路径 | 内容 |
| --- | --- |
| `~/.miyu/config/purge-inject.md` | 注入正文。`miyu purge edit` 首次生成默认模板；**用户改正文永不覆盖** |
| `src/prompts/purge-default.md` | 出厂模板（编译进二进制；源自 dsh-purge sealed slot，按 Miyu 语境改写） |
| `crates/miyu-base/src/config/purge.rs` | PurgeConfig / PurgeFlags / 注入文件读写 |
| `crates/miyu-engine/src/agent/purge.rs` | PurgeInjection：构造期折段、冻结、拼装 |
| `crates/miyu-base/src/sandbox/mod.rs` | `PURGE_PASSTHROUGH` 静态旗标，confine 三入口直通 |
| `crates/miyu-engine/src/tools/net_guard.rs` | `NET_GUARD_BYPASSED` 静态旗标，公网闸退位（URL 形状层保留） |

## 生效路径

- **注入**：Agent 构造期读配置 × 正文 → 冻结段体 → `assemble_system_prompt`
  最末尾追加（MCP 指令段之后）。同一会话字节恒定，中途改文件只影响新会话。
- **命令闸**：`install_builtin_guards` 按 `command_guards_bypassed()` 决定挂不挂。
- **出站闸**：daemon 启动（`web/server.rs`）与 `ReloadConfig` IPC 都会刷新旗标；
  改配置后 `miyu reload` 或重启即生效，不用重编。
- **沙盒直通**：同上静态旗标。策略照算照报（`<sandbox>` 尾巴、`/sandbox` 查看
  不撒谎），只是执行层放行；进程内 `guard_read/guard_write` 不受影响。

## CLI

```sh
miyu purge status     # 各层开关、注入正文来源、文件路径
miyu purge edit       # 首次创建注入文件（不覆盖已有）；--print 输出正文
miyu purge selfcheck  # 配置 × 注入正文 × daemon 运行时旗标逐项核对（IPC 查真值）
miyu purge probe      # 出 nonce，新会话里让 Miyu 原样回显 = 注入到达模型
```

## 已验

- **测试**：workspace lib 3136 通过；purge 相关 15 条新测试全绿（配置 serde、
  字节冻结、关=上游逐字节一致、rm/子串闸放行、net 闸放行但形状层保留、
  Landlock 直通行为级验证——旗标关着沙盒外写失败、开着成功）。
- **门禁**：`arch_dep_check`（层序）、`refactor_size_report`（文件规模）、
  `check-model-english`（模型面英文）全过；fmt 归零检查——本次改动没有让任何
  文件变差（本机 rustfmt 1.10 与 CI 1.96.1 对既有文件判定不同，`fmt_no_regress`
  需要 rustup 跑钉版，本机没有 rustup）。
- **E2E 真机**：隔离 MIYU_HOME + 真 daemon + 真 LLM（本机 8326 中转）——
  `miyu purge probe` 出的 nonce 经 WebUI `/api/turns` 发进新会话，模型**原样
  回显** `miyu-purge-05fb82e5`，回合 completed。探针契约（回显=注入到达模型）
  与 dsh-purge 相同口径。selfcheck 对着运行中的 daemon 全绿（含 IPC 查旗标）。

## 已知边界

- 树上另有在途的 voice 半成品（GPT-SoVITS fixed_ref / openai_compat TTS +
  sanitize 断句改版），`voice_tts` 3 条失败、`goals_overview_json` 1 条 warning
  均属那半成品（stash 验证：HEAD 上全绿，pop 后红），与 purge 无关。
- 新装配置的默认模板尚未写进 `AppConfig::default()` 的序列化输出（serde
  `#[serde(default)]` 字段不落盘），首次保存配置后会出现空缺段——手写或等
  下次 save 都行，读写两侧都按默认值处理。
- `<purge-mode>` 段是常量级重指令，token 成本约 21k（默认模板全文）；在意
  上下文预算的话可以精简 `purge-inject.md`——运行时以文件为准，模板只是
  首次种子。
