//! purge 注入段的拼装(对照 dsh-purge 的 prompt-inject 注入层)。
//!
//! 位置:system 提示词的最末尾(MCP 指令段之后)。每请求新组装、不化石
//! (AGENTS §1.4);段体在同一会话里**字节冻结**——由 [`PurgeInjection::freeze`]
//! 在 Agent 构造期读一次,之后 `refresh_system_prompt` 拼的都是同一份,中途改
//! `purge-inject.md` 只影响新会话。外壳是 XML 标签,与 `<style-lock>` /
//! `<artifact-policy>` 同一族;标签名 [`miyu_base::config::purge::PURGE_SECTION_TAG`]。
//!
//! 顺序即缓存前缀:追加-only,前缀里已有的段一个字节不动。purge 关着时
//! [`PurgeInjection::section`] 返回 None,拼装结果与上游逐字节一致。

use miyu_base::config::purge::{BOOST_SECTION, PURGE_SECTION_TAG};

/// 一个会话的 purge 注入段:构造期折出来,之后只读。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::agent) struct PurgeInjection {
    /// 冻结的段体(含外壳)。None = purge 关着或注入正文为空,系统提示词不拼。
    section: Option<String>,
}

impl PurgeInjection {
    /// 构造期折一次:读配置、读注入正文,冻结段体。
    pub(in crate::agent) fn resolve(
        config: &miyu_base::config::AppConfig,
        paths: &miyu_base::paths::MiyuPaths,
    ) -> Self {
        let text = config.purge.inject_text(paths).unwrap_or_default();
        let flags = miyu_base::config::PurgeFlags::resolve(&config.purge, &text);
        Self {
            section: flags.inject.then(|| Self::assemble_from(&text, &flags)),
        }
    }

    /// 测试与显式正文路径:拿给定的注入正文折段体(不读盘)。
    #[cfg(any(test, feature = "testkit"))]
    pub(in crate::agent) fn from_text(config: &miyu_base::config::AppConfig, text: &str) -> Self {
        let flags = miyu_base::config::PurgeFlags::resolve(&config.purge, text);
        Self {
            section: flags.inject.then(|| Self::assemble_from(text, &flags)),
        }
    }

    /// 冻结的段体。拼装层只在 `assemble_system_prompt` 最末尾追加它。
    pub(in crate::agent) fn section(&self) -> Option<&str> {
        self.section.as_deref()
    }

    fn assemble_from(text: &str, flags: &miyu_base::config::PurgeFlags) -> String {
        let body = text.trim();
        let mut section = format!("<{PURGE_SECTION_TAG}>\n{body}\n</{PURGE_SECTION_TAG}>");
        if flags.boost {
            section.push_str(BOOST_SECTION);
        }
        section
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miyu_base::config::PurgeConfig;

    fn config(enabled: bool) -> miyu_base::config::AppConfig {
        let mut config = miyu_base::config::AppConfig::default();
        config.purge = PurgeConfig {
            enabled,
            ..PurgeConfig::default()
        };
        config
    }

    #[test]
    fn disabled_config_yields_no_section() {
        assert!(PurgeInjection::from_text(&config(false), "text")
            .section()
            .is_none());
    }

    #[test]
    fn section_wraps_body_and_appends_boost() {
        let injection = PurgeInjection::from_text(&config(true), "be direct");
        let section = injection.section().expect("enabled + non-empty text");
        assert!(section.starts_with("<purge-mode>\nbe direct\n</purge-mode>"));
        assert!(section.contains("<purge-boost"));
        // 字节冻结:同一输入再折一次,逐字节相同。
        assert_eq!(
            injection,
            PurgeInjection::from_text(&config(true), " be direct \n")
        );
    }

    #[test]
    fn boost_off_keeps_only_the_body() {
        let mut config = config(true);
        config.purge.boost = false;
        let injection = PurgeInjection::from_text(&config, "body");
        let section = injection.section().unwrap();
        assert!(section.starts_with("<purge-mode>\nbody\n</purge-mode>"));
        assert!(!section.contains("purge-boost"), "{section}");
        assert_eq!(section, "<purge-mode>\nbody\n</purge-mode>");
    }

    #[test]
    fn empty_text_yields_no_section_even_when_enabled() {
        assert!(PurgeInjection::from_text(&config(true), "  \n ")
            .section()
            .is_none());
    }
}
