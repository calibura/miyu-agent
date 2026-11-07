//! `AppConfig::save` 自动为 purge 段变更记审计的集成测试。
//!
//! 审计是旁路能力，但它挂在保存热路径上，所以两条硬要求：
//! 1. **不该记的不记**——改个 voice 参数不能产出 purge 事件；
//! 2. **该记的一定记**——开关/名单/规则勾选变化要留痕，且失败不能推翻保存。

use super::super::*;

fn paths(root: &std::path::Path) -> MiyuPaths {
    let config_dir = root.join("config");
    std::fs::create_dir_all(&config_dir).unwrap();
    MiyuPaths {
        root_dir: root.to_path_buf(),
        config_dir: config_dir.clone(),
        config_file: config_dir.join("config.jsonc"),
        skills_dir: config_dir.join("skills"),
        data_dir: root.join("data"),
        cache_dir: root.join("cache"),
        state_dir: root.join("state"),
        pictures_dir: root.join("pictures"),
        fish_hook_file: root.join("miyu.fish"),
        bash_hook_file: root.join("miyu.bash"),
        zsh_hook_file: root.join("miyu.zsh"),
        scripts_dir: config_dir.join("scripts"),
        system_scripts_dir: std::path::PathBuf::new(),
    }
}

fn audit_lines(paths: &MiyuPaths) -> Vec<crate::config::purge::PurgeAuditEvent> {
    let path = crate::config::purge::audit_file_path(&paths.state_dir);
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    raw.lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

#[test]
fn turning_purge_on_and_off_leaves_two_audit_lines() {
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(temp.path());
    std::fs::create_dir_all(&paths.state_dir).unwrap();

    // 第一次保存,全默认(purge 关):不该记。
    let mut config = AppConfig::default();
    config.save(&paths).unwrap();
    assert!(audit_lines(&paths).is_empty(), "默认配置落盘不算事件");

    // 打开 purge:记一条。
    config.purge.enabled = true;
    config.save(&paths).unwrap();
    let lines = audit_lines(&paths);
    assert_eq!(lines.len(), 1);
    assert!(lines[0].enabled);
    assert_eq!(lines[0].detail, "purge enabled");

    // 关掉:再记一条。
    config.purge.enabled = false;
    config.save(&paths).unwrap();
    let lines = audit_lines(&paths);
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[1].detail, "purge disabled");
}

#[test]
fn unrelated_field_changes_produce_no_purge_audit() {
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(temp.path());
    std::fs::create_dir_all(&paths.state_dir).unwrap();

    let mut config = AppConfig::default();
    config.purge.enabled = true;
    config.save(&paths).unwrap();
    let baseline = audit_lines(&paths).len();

    // 改一个跟 purge 无关的字段。
    config.display.expand_reasoning = !config.display.expand_reasoning;
    config.save(&paths).unwrap();

    assert_eq!(
        audit_lines(&paths).len(),
        baseline,
        "改 voice/display 不该产出 purge 事件"
    );
}

#[test]
fn allowlist_and_rule_changes_are_summarized() {
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(temp.path());
    std::fs::create_dir_all(&paths.state_dir).unwrap();

    let mut config = AppConfig::default();
    config.purge.enabled = true;
    config.save(&paths).unwrap();

    config.purge.net_allowlist = vec!["example.com".to_string(), "203.0.113.10".to_string()];
    config.save(&paths).unwrap();
    let lines = audit_lines(&paths);
    let last = lines.last().unwrap();
    assert!(
        last.detail.contains("allowlist=2 entries"),
        "{}",
        last.detail
    );

    config.purge.rule_selections = vec!["/tmp/AGENTS.md".to_string()];
    config.save(&paths).unwrap();
    let lines = audit_lines(&paths);
    let last = lines.last().unwrap();
    assert!(
        last.detail.contains("rules=1 selections"),
        "{}",
        last.detail
    );
}
