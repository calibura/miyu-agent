//! 数据类命令：知识库、记忆、技能。
//!
//! 三者的共性是「Miyu 记住的东西」的增删查改入口——知识库是人喂的资料，记忆
//! 是它自己攒的，技能是可复用的操作手册。放在一起是因为它们的子命令结构与
//! 输出格式高度相似，改一个通常要顺手对齐另外两个。

use crate::cli::*;

#[derive(Debug, Args)]
pub struct KbArgs {
    #[command(subcommand)]
    pub command: KbCommand,
}

#[derive(Debug, Args)]
pub struct MemoryArgs {
    #[command(subcommand)]
    pub command: MemoryCommand,
}

#[derive(Debug, Subcommand)]
pub enum MemoryCommand {
    Stats,
    Reset(MemoryResetArgs),
    Search(MemorySearchArgs),
    Remember(MemoryRememberArgs),
}

#[derive(Debug, Args)]
pub struct MemoryResetArgs {
    #[arg(long)]
    pub include_skills: bool,
    /// 只清这个会话记下的记忆(默认清当前人格的全部)
    #[arg(long, value_name = "ID", conflicts_with = "include_skills")]
    pub session: Option<String>,
}

#[derive(Debug, Args)]
pub struct MemorySearchArgs {
    pub query: Vec<String>,
    #[arg(short, long)]
    pub limit: Option<usize>,
    #[arg(long)]
    pub forgotten: bool,
}

#[derive(Debug, Args)]
pub struct MemoryRememberArgs {
    pub content: Vec<String>,
    #[arg(short, long, default_value = "manual")]
    pub source: String,
}

#[derive(Debug, Args)]
pub struct PurgeArgs {
    #[command(subcommand)]
    pub command: PurgeCommand,
}

#[derive(Debug, Subcommand)]
pub enum PurgeCommand {
    /// 状态总览:各层开关、注入正文来源、注入文件路径
    Status,
    /// 创建/查看注入文件路径(不覆盖已有正文);`--print` 输出正文
    Edit {
        #[arg(long)]
        print: bool,
    },
    /// 生效性自检:配置 × 注入正文 × 运行时旗标逐项核对
    Selfcheck,
    /// 探针:输出一个 nonce,会话里让 Miyu 原样回显即可确认注入到达模型
    Probe,
}

#[derive(Debug, Args)]
pub struct SkillsArgs {
    #[command(subcommand)]
    pub command: SkillsCommand,
}

#[derive(Debug, Subcommand)]
pub enum SkillsCommand {
    List,
    Show(SkillNameArgs),
    Enable(SkillNameArgs),
    Disable(SkillNameArgs),
    Remove(SkillNameArgs),
    Stats,
    Prune,
}

#[derive(Debug, Args)]
pub struct SkillNameArgs {
    pub name: String,
}

#[derive(Debug, Subcommand)]
pub enum KbCommand {
    Add(KbAddArgs),
    List,
    Search(KbSearchArgs),
    Find(KbFindArgs),
    Read(KbReadArgs),
    Remove(KbRemoveArgs),
    Reindex,
    Stats,
    Embed(KbEmbedArgs),
}

#[derive(Debug, Args)]
pub struct KbAddArgs {
    pub path: PathBuf,
    #[arg(
        short,
        long,
        help = "Compatibility flag; directories are recursive by default"
    )]
    pub recursive: bool,
}

#[derive(Debug, Args)]
pub struct KbSearchArgs {
    pub query: Vec<String>,
    #[arg(short, long)]
    pub limit: Option<usize>,
}

#[derive(Debug, Args)]
pub struct KbFindArgs {
    pub query: Vec<String>,
    #[arg(short, long)]
    pub limit: Option<usize>,
}

#[derive(Debug, Args)]
pub struct KbReadArgs {
    pub file: String,
    #[arg(long, default_value_t = 1)]
    pub start: usize,
    #[arg(long)]
    pub lines: Option<usize>,
}

#[derive(Debug, Args)]
pub struct KbRemoveArgs {
    pub file: String,
}

#[derive(Debug, Args)]
pub struct KbEmbedArgs {
    #[command(subcommand)]
    pub command: KbEmbedCommand,
}

#[derive(Debug, Subcommand)]
pub enum KbEmbedCommand {
    Reindex(KbEmbedReindexArgs),
}

#[derive(Debug, Args)]
pub struct KbEmbedReindexArgs {
    #[arg(long)]
    pub quiet: bool,
}

pub(in crate::cli) async fn run_kb(paths: &MiyuPaths, args: KbArgs) -> Result<()> {
    let config = AppConfig::load(paths)?;
    let kb = tools::knowledge_base::KnowledgeBase::new(config, paths.clone())?;
    match args.command {
        KbCommand::Add(args) => {
            let added = kb.add_path(&args.path).await?;
            for path in added {
                println!("{} {path}", t("added", "已添加"));
            }
        }
        KbCommand::List => {
            for file in kb.list()? {
                println!("{}\t{} {}", file.name, file.size_bytes, t("bytes", "字节"));
            }
        }
        KbCommand::Search(args) => {
            let query = args.query.join(" ");
            println!("{}", kb.search(&query, args.limit).await?);
        }
        KbCommand::Find(args) => {
            let query = args.query.join(" ");
            println!("{}", kb.find_by_name(&query, args.limit)?);
        }
        KbCommand::Read(args) => {
            println!("{}", kb.read_file(&args.file, args.start, args.lines)?);
        }
        KbCommand::Remove(args) => {
            kb.remove(&args.file)?;
            println!("{} {}", t("removed", "已移除"), args.file);
        }
        KbCommand::Reindex => {
            let files = kb.list()?;
            println!(
                "{}: {}",
                t(
                    "keyword index is rebuilt on demand; files tracked",
                    "关键词索引会按需重建；已跟踪文件数",
                ),
                files.len()
            );
        }
        KbCommand::Stats => {
            let mut stats = kb.stats()?;
            if let Some(object) = stats.as_object_mut() {
                if let Ok(status) = miyu_engine::default_kb::status(paths) {
                    object.insert(
                        "default_kb_update_available".to_string(),
                        serde_json::json!(status.has_update_notice),
                    );
                }
            }
            println!("{}", stats);
        }
        KbCommand::Embed(args) => match args.command {
            KbEmbedCommand::Reindex(args) => {
                kb.reindex_embeddings(args.quiet).await?;
            }
        },
    }
    Ok(())
}

pub(in crate::cli) async fn run_update_default_kb(paths: &MiyuPaths) -> Result<()> {
    let config = AppConfig::load_or_default(paths)?;
    let state = miyu_engine::default_kb::update(paths, &config, |stage| {
        let mut stderr = io::stderr().lock();
        let _ = write_default_kb_update_progress(&mut stderr, stage);
    })?;
    println!(
        "{}: {}",
        t("updated default knowledge base", "已更新默认知识库"),
        state.shorin_wiki_commit
    );
    Ok(())
}

pub(in crate::cli) fn write_default_kb_update_progress(
    output: &mut impl Write,
    stage: miyu_engine::default_kb::UpdateStage,
) -> io::Result<()> {
    writeln!(output, "[default-kb] {}", stage.message())?;
    output.flush()
}

pub(in crate::cli) fn run_memory(paths: &MiyuPaths, args: MemoryArgs) -> Result<()> {
    let config = AppConfig::load_or_default(paths)?;
    let store = MemoryStore::new(&config, paths);
    match args.command {
        MemoryCommand::Stats => println!("{}", store.stats()?),
        MemoryCommand::Reset(args) => match args.session.as_deref() {
            Some(session_id) => println!("{}", store.reset_session(session_id)?.describe()),
            None => {
                store.reset_all()?;
                if args.include_skills {
                    miyu_core::skills::purge_generated_skills(
                        &config.active_persona_skills_dir(paths),
                    )?;
                }
                println!("{}", t("cleared assistant memory", "已清空助手记忆"));
            }
        },
        MemoryCommand::Search(args) => {
            let query = join_message(args.query);
            let limit = args.limit.unwrap_or(10);
            println!("{}", store.recall_memories(&query, limit, args.forgotten)?);
        }
        MemoryCommand::Remember(args) => {
            let content = join_message(args.content);
            let id = store.remember_fact(&content, &args.source)?;
            println!("{}: {id}", t("remembered fact", "已记住事实"));
        }
    }
    Ok(())
}

pub(in crate::cli) fn run_skills(paths: &MiyuPaths, args: SkillsArgs) -> Result<()> {
    std::fs::create_dir_all(&paths.skills_dir)?;
    match args.command {
        SkillsCommand::List => {
            for name in skill_names(paths)? {
                let disabled = paths.skills_dir.join(&name).join(".disabled").exists();
                println!(
                    "{}{}",
                    name,
                    if disabled {
                        t(" [disabled]", " [已禁用]")
                    } else {
                        ""
                    }
                );
            }
        }
        SkillsCommand::Show(args) => {
            let path = skill_dir(paths, &args.name)?.join("SKILL.md");
            println!("{}", std::fs::read_to_string(path)?);
        }
        SkillsCommand::Enable(args) => {
            let marker = skill_dir(paths, &args.name)?.join(".disabled");
            if marker.exists() {
                std::fs::remove_file(marker)?;
            }
            println!("{}: {}", t("enabled skill", "已启用 skill"), args.name);
        }
        SkillsCommand::Disable(args) => {
            let marker = skill_dir(paths, &args.name)?.join(".disabled");
            std::fs::write(marker, "disabled\n")?;
            println!("{}: {}", t("disabled skill", "已禁用 skill"), args.name);
        }
        SkillsCommand::Remove(args) => {
            let dir = skill_dir(paths, &args.name)?;
            std::fs::remove_dir_all(dir)?;
            println!("{}: {}", t("removed skill", "已移除 skill"), args.name);
        }
        SkillsCommand::Stats => {
            let names = skill_names(paths)?;
            let disabled = names
                .iter()
                .filter(|name| paths.skills_dir.join(name).join(".disabled").exists())
                .count();
            println!(
                "{}",
                serde_json::json!({
                    "ok": true,
                    "skills_dir": paths.skills_dir.display().to_string(),
                    "skills": names.len(),
                    "disabled": disabled,
                    "enabled": names.len().saturating_sub(disabled),
                })
            );
        }
        SkillsCommand::Prune => {
            let mut removed = 0usize;
            for name in skill_names(paths)? {
                let dir = paths.skills_dir.join(&name);
                let raw = std::fs::read_to_string(dir.join("SKILL.md")).unwrap_or_default();
                if miyu_core::skills::is_generated_skill(&raw) && dir.join(".disabled").exists() {
                    std::fs::remove_dir_all(dir)?;
                    removed += 1;
                }
            }
            println!("{}: {removed}", t("pruned skills", "已清理 skills"));
        }
    }
    Ok(())
}

pub(in crate::cli) fn skill_names(paths: &MiyuPaths) -> Result<Vec<String>> {
    let mut names = Vec::new();
    if !paths.skills_dir.exists() {
        return Ok(names);
    }
    for entry in std::fs::read_dir(&paths.skills_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() && entry.path().join("SKILL.md").is_file() {
            names.push(entry.file_name().to_string_lossy().to_string());
        }
    }
    names.sort();
    Ok(names)
}

pub(in crate::cli) fn skill_dir(paths: &MiyuPaths, name: &str) -> Result<PathBuf> {
    let clean = name.trim();
    if clean.is_empty()
        || clean.contains('/')
        || clean.contains('\\')
        || clean == "."
        || clean == ".."
    {
        bail!("{}: {name}", t("invalid skill name", "无效 skill 名称"));
    }
    let dir = paths.skills_dir.join(clean);
    if !dir.join("SKILL.md").is_file() {
        bail!("{}: {name}", t("skill not found", "未找到 skill"));
    }
    Ok(dir)
}

// ── purge 子系统(§purge)────────────────────────────────────────────

/// purge 探针 nonce 的落盘路径(state 目录,30 分钟过期)。
fn purge_probe_path(paths: &MiyuPaths) -> PathBuf {
    paths.state_dir.join("purge-probe.json")
}

/// 向运行中的 daemon 查 purge 运行时旗标(GetStatus 的 `purge` 块)。
/// 返回 (net_guard_bypassed, sandbox_passthrough);daemon 没在跑或旧版没带
/// 这个块时按 Err 处理,由调用方退回配置期望值。
pub(in crate::cli) async fn purge_runtime_flags(
    paths: &MiyuPaths,
) -> Result<(bool, bool)> {
    let (_, data) = send_ipc_admin(paths, IpcCommand::GetStatus).await?;
    let purge = data
        .get("purge")
        .ok_or_else(|| anyhow::anyhow!("daemon did not report purge flags (older build?)"))?;
    let net = purge
        .get("net_guard_bypassed")
        .and_then(serde_json::Value::as_bool)
        .context("missing net_guard_bypassed in daemon status")?;
    let sandbox = purge
        .get("sandbox_passthrough")
        .and_then(serde_json::Value::as_bool)
        .context("missing sandbox_passthrough in daemon status")?;
    Ok((net, sandbox))
}

pub(in crate::cli) async fn run_purge(paths: &MiyuPaths, args: PurgeArgs) -> Result<()> {
    match args.command {
        PurgeCommand::Status => {
            let config = AppConfig::load_or_default(paths)?;
            let purge = &config.purge;
            let inject_path = purge.inject_file(paths);
            let text = purge.inject_text(paths);
            let flags =
                miyu_base::config::PurgeFlags::resolve(purge, text.as_deref().unwrap_or(""));
            println!(
                "{}: {}",
                t("purge mode", "purge 模式"),
                if purge.enabled {
                    t("on", "开")
                } else {
                    t("off", "关")
                }
            );
            let row = |label: &str, on: bool| {
                println!("  {label}: {}", if on { "✓" } else { "✗" });
            };
            row(
                &t("injection into system prompt", "system 注入"),
                flags.inject,
            );
            row(&t("boost section", "强化段"), flags.boost);
            row(
                &t("command guards bypass", "命令闸放行"),
                flags.bypass_command_guards,
            );
            row(&t("net guard bypass", "出站闸放行"), flags.bypass_net_guard);
            row(
                &t("sandbox passthrough", "沙盒直通"),
                flags.sandbox_bypassed,
            );
            println!(
                "{}: {}",
                t("inject file", "注入文件"),
                inject_path.display()
            );
            let source = match &text {
                Some(_) => t("custom (purge-inject.md)", "自定义 (purge-inject.md)"),
                None if purge.enabled => t(
                    "missing — run `miyu purge edit` first",
                    "缺失——先跑 `miyu purge edit`",
                ),
                None => t("off", "关"),
            };
            println!("{}: {source}", t("inject source", "注入正文"));
        }
        PurgeCommand::Edit { print } => {
            let created = miyu_base::config::purge::ensure_inject_file(&paths.config_dir)?;
            let path = miyu_base::config::purge::inject_file_path(&paths.config_dir);
            if created {
                println!(
                    "{}: {}",
                    t("created inject file", "已创建注入文件"),
                    path.display()
                );
            } else {
                println!("{}", path.display());
            }
            if print {
                println!("---");
                println!("{}", std::fs::read_to_string(&path).unwrap_or_default());
            }
        }
        PurgeCommand::Selfcheck => {
            let config = AppConfig::load_or_default(paths)?;
            let purge = &config.purge;
            let text = purge.inject_text(paths);
            let flags =
                miyu_base::config::PurgeFlags::resolve(purge, text.as_deref().unwrap_or(""));
            let mut failures: Vec<String> = Vec::new();
            let check = |failures: &mut Vec<String>, name: &str, ok: bool| {
                println!("{} {name}", if ok { "✓" } else { "✗" });
                if !ok {
                    failures.push(name.to_string());
                }
            };
            check(
                &mut failures,
                &t("purge.enabled is on", "purge.enabled 开着"),
                purge.enabled,
            );
            check(
                &mut failures,
                &t(
                    "inject text readable (enabled ⇒ file exists)",
                    "注入正文可读(开着 ⇒ 文件在)",
                ),
                !purge.enabled || flags.inject,
            );
            check(
                &mut failures,
                &t("boost section configured", "强化段配置"),
                !purge.enabled || !purge.boost || flags.boost,
            );
            // 运行时旗标(net 闸/沙盒直通)住在 daemon 进程的内存里,CLI 进程
            // 读不到。daemon 在跑就走 IPC 拿真实值;没在跑,静态旗标天然还没
            // 生效,按配置期望报——启动时它们会被写成同一个值。
            let daemon_live = ipc::daemon_info(paths).await.is_some();
            let (net_flag, sandbox_flag) = if daemon_live {
                match purge_runtime_flags(paths).await {
                    Ok((net, sandbox)) => (net, sandbox),
                    Err(error) => {
                        println!(
                            "⚠ {}: {error}",
                            t(
                                "failed to query the running daemon; checking config expectation only",
                                "查询运行中的 daemon 失败;只按配置期望核对"
                            )
                        );
                        (purge.net_guard_bypassed(), purge.sandbox_bypassed())
                    }
                }
            } else {
                (purge.net_guard_bypassed(), purge.sandbox_bypassed())
            };
            check(
                &mut failures,
                &t("net guard flag matches config", "出站闸旗标与配置一致"),
                net_flag == purge.net_guard_bypassed(),
            );
            check(
                &mut failures,
                &t("sandbox flag matches config", "沙盒旗标与配置一致"),
                sandbox_flag == purge.sandbox_bypassed(),
            );
            if failures.is_empty() {
                println!("{}", t("purge selfcheck: all green", "purge 自检:全绿"));
            } else {
                println!(
                    "{}",
                    t(
                        "hint: edit config.jsonc, then restart the daemon (or `miyu reload`) to apply.",
                        "提示:改完 config.jsonc 后重启 daemon(或 `miyu reload`)生效。"
                    )
                );
                bail!(
                    "{}",
                    t(
                        "purge selfcheck failed; see the ✗ rows above",
                        "purge 自检未过;看上面的 ✗ 行"
                    )
                );
            }
        }
        PurgeCommand::Probe => {
            let config = AppConfig::load_or_default(paths)?;
            if !config.purge.enabled {
                bail!(
                    "{}",
                    t(
                        "purge mode is off; enable it in config.jsonc first",
                        "purge 模式关着;先在 config.jsonc 里打开"
                    )
                );
            }
            let nonce = format!(
                "miyu-purge-{:08x}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .subsec_nanos() as u64
                    ^ std::process::id() as u64
            );
            let expires = chrono::Utc::now() + chrono::Duration::minutes(30);
            let payload = serde_json::json!({ "nonce": nonce, "expires": expires.to_rfc3339() });
            std::fs::create_dir_all(&paths.state_dir)?;
            std::fs::write(purge_probe_path(paths), payload.to_string())?;
            println!(
                "{}",
                t(
                    "send this line to Miyu in a new session:",
                    "把下面这行发给新会话里的 Miyu:"
                )
            );
            println!("  请原样输出下面这行,不要加任何别的内容:\n  [{nonce}]");
            println!(
                "{}",
                t(
                    "echo confirmed = the injection reached the model (probe expires in 30 min).",
                    "回显确认 = 注入已到达模型(探针 30 分钟后过期)。"
                )
            );
        }
    }
    Ok(())
}
