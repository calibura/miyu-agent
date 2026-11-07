>更新内容记录在此处，每次更新release时作为releasenote发布，发布后清理已发布内容。

## 重要更新

- WebUI 新增「目标」面板：跨会话列出所有长任务目标，可直接暂停/继续/完成（带审计日志）。
- 「目标」面板附带 daemon 级轻量待办清单（`GET/POST/PATCH/DELETE /api/todos`），存管理员库的 `daemon_todos` 表。
- 新配置 `platforms.qq.goal_progress_notify`（默认关闭）：goal 自动续轮每轮结束后向 QQ 管理员推送一句简短进度。
- **purge 破甲子系统**（移植 dsh-purge）：
  - 配置驱动五层开关：`purge.enabled` 总开关，`bypass_command_guards`（rm 闸与 command_deny）、`bypass_net_guard`（SSRF 三件套）、`sandbox_passthrough`（Landlock 直通）、`boost`（尾部强化段）。没有 `[purge]` 段时每个挂接点与上游逐字节一致。
  - 注入正文住 `~/.miyu/config/purge-inject.md`，会话期字节冻结；用户改的正文永不覆盖。
  - **出站白名单** `purge.net_allowlist`：每行一个 IP 或主机名（也接受 `账号@主机`）。留空 = 全放行；填了只放行名单内主机，精确匹配不做通配。
  - **规则页** `purge.rule_selections` + `miyu purge rules`：扫描 cwd 到项目根（`.git`/`.hg`/`.svn` 为界，上限 32 层）的 `AGENTS.md`/`CLAUDE.md`/`AGENTS.local.md`/`CLAUDE.local.md`，勾选后拼进注入段（`<purge-mode>` 内、`<purge-boost>` 前）。勾选只在当前目录扫描结果内生效，防跨目录穿越。
  - **审计日志** `<state_dir>/purge-audit.jsonl`：purge 段实质变化（开关/名单/规则）逐条留痕，带旗标快照；改无关配置不产出事件。
  - CLI：`miyu purge status|edit|selfcheck|probe|rules`；`edit --reset` 恢复出厂模板并把旧正文备份成 `purge-inject.md.bak-<时间戳>`。
  - **WebUI**：设置页新增「Purge 模式」段（5 个开关 + 白名单列表），保存即刷新运行时旗标。

## 修复

- daemon 让位退出码从 0 改为 `EX_TEMPFAIL(75)`：原来退出码 0 不算失败，systemd 的 `Restart=on-failure` 不重启，unit 静默变 inactive，只能靠 guard 每 30 秒轮询捡回来；占位方一死中间全是没人管的窗口。
- **受 systemd 监管的 daemon 不再被客户端私杀私拉**：旧 build_id 的客户端（比如升级前就开着的 TUI）连上时，原来走「IPC 关停 + 自己 spawn 裸进程」，与 guard 形成自持循环（2026-11-06 本机实测转了一小时）。现在先认 cgroup，daemon 在 `miyu.service` 里就委托 `systemctl --user restart`；本进程若就在同一 unit 里则拒绝动手，避免把执行自己的父进程连根拔掉。
- WebUI 保存走 `ApplyConfig` 时不再漏刷 purge 的两个静态旗标（出站闸、沙盒直通）。此前面板点了开关、jsonc 写了，实际不生效，必须重启 daemon。
- daemon 监管体系（`miyu.service` + `miyu-guard.timer` + guard 脚本）入版本库，与已部署版本逐字节一致。
