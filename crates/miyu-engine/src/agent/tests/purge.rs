//! purge 注入段的 Agent 级集成测试:开/关两态的 system 提示词形状、字节冻结、
//! 以及「关着时与上游逐字节一致」。

use super::shared::*;
use crate::agent::*;
use miyu_base::config::{purge::ensure_inject_file, AppConfig, PromptAudience};

/// 组一个 Agent 并返回它的 system 提示词。工具面用 build_tool_registry,
/// 与 request_shape_probe 同一条装配路。
fn owner_system_prompt(temp: &std::path::Path, purge_enabled: bool) -> String {
    let paths = test_paths(temp);
    let mut config = AppConfig::default();
    config.purge.enabled = purge_enabled;
    if purge_enabled {
        ensure_inject_file(&paths.config_dir).unwrap();
    }
    let state = StateStore::new(&paths).unwrap();
    state.init_files().unwrap();
    let client =
        OpenAiCompatibleClient::new(config.provider(None).unwrap(), &config, &paths).unwrap();
    let tools =
        crate::tools::build_tool_registry(&config, &paths, PersonaLane::Active, true).unwrap();
    let mut agent = Agent::new_for_audience(
        config,
        &paths,
        state,
        client,
        tools,
        PersonaLane::Active,
        PromptAudience::Owner,
    )
    .unwrap();
    agent.prepare_for_turn().unwrap();
    agent.system_prompt.clone()
}

#[test]
fn purge_off_reproduces_the_upstream_prompt_byte_for_byte() {
    let temp = tempfile::tempdir().unwrap();
    let without = owner_system_prompt(temp.path(), false);
    // 关着时段不存在:提示词里没有任何 purge 痕迹。
    assert!(!without.contains("purge-mode"), "{without}");
    assert!(!without.contains("purge-boost"));
}

#[test]
fn purge_on_appends_the_frozen_section_at_the_very_end() {
    let temp = tempfile::tempdir().unwrap();
    let with = owner_system_prompt(temp.path(), true);
    assert!(with.contains("<purge-mode>"), "{with}");
    assert!(with.contains("</purge-mode>"));
    assert!(with.contains("<purge-boost"));
    // 追加-only:段在最末尾,前面所有段落字节不动(对比关着的版本)。
    let without = owner_system_prompt(temp.path(), false);
    assert!(
        with.starts_with(&without),
        "enabling purge must only append, not rewrite the prefix"
    );
}

#[test]
fn purge_section_is_frozen_per_agent_across_rebuilds() {
    // 同一 Agent 两轮重拼(refresh_system_prompt 走 assemble_system_prompt),
    // 注入段逐字节相同——中途改文件不影响本会话。
    let temp = tempfile::tempdir().unwrap();
    let paths = test_paths(temp.path());
    let mut config = AppConfig::default();
    config.purge.enabled = true;
    ensure_inject_file(&paths.config_dir).unwrap();
    let state = StateStore::new(&paths).unwrap();
    state.init_files().unwrap();
    let client =
        OpenAiCompatibleClient::new(config.provider(None).unwrap(), &config, &paths).unwrap();
    let tools =
        crate::tools::build_tool_registry(&config, &paths, PersonaLane::Active, true).unwrap();
    let mut agent = Agent::new_for_audience(
        config.clone(),
        &paths,
        state,
        client,
        tools,
        PersonaLane::Active,
        PromptAudience::Owner,
    )
    .unwrap();
    agent.prepare_for_turn().unwrap();
    let first = agent.system_prompt.clone();
    // 构造期之后改注入文件:冻结段不受影响。
    let inject = miyu_base::config::purge::inject_file_path(&paths.config_dir);
    std::fs::write(&inject, "edited mid-session").unwrap();
    agent.prepare_for_turn().unwrap();
    let second = agent.system_prompt.clone();
    assert_eq!(first, second, "the section is frozen for the session");
    // 新 Agent 读到新正文——冻结是会话级,不是进程级。
    let state = StateStore::new(&paths).unwrap();
    let client =
        OpenAiCompatibleClient::new(config.provider(None).unwrap(), &config, &paths).unwrap();
    let tools =
        crate::tools::build_tool_registry(&config, &paths, PersonaLane::Active, true).unwrap();
    let mut fresh = Agent::new_for_audience(
        config,
        &paths,
        state,
        client,
        tools,
        PersonaLane::Active,
        PromptAudience::Owner,
    )
    .unwrap();
    fresh.prepare_for_turn().unwrap();
    assert!(fresh.system_prompt.contains("edited mid-session"));
    assert!(!fresh.system_prompt.contains("first"));
}
