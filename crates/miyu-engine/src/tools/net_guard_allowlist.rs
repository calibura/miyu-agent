//! 出站放行白名单的进程级槽。
//!
//! 移植自 dsh-purge 的「自己的服务器」页：那边把白名单存成每行一条的文本，
//! 放行时只认名单里的主机；这边走 `config.jsonc` 的 `purge.net_allowlist`，
//! 折进这个静态槽供 `net_guard` 热路径读。
//!
//! 为什么是 `RwLock` 而不是 `OnceLock`：`static` 上要能 const 构造，而且
//! `reload` / WebUI 保存要走 `ApplyConfig` 整体替换一次。读多写极少的形态，
//! `RwLock` 足够，不必上无锁结构。

use miyu_base::config::NetAllowEntry;
use std::sync::RwLock;

pub(crate) struct AllowlistSlot {
    entries: RwLock<Vec<NetAllowEntry>>,
}

impl AllowlistSlot {
    pub(crate) const fn new() -> Self {
        Self {
            entries: RwLock::new(Vec::new()),
        }
    }

    /// 整体替换。锁中毒时照常取回：白名单是运维数据，不值得为一次 panic 的
    /// 残留把整条出站路径堵死。
    pub(crate) fn replace(&self, entries: Vec<NetAllowEntry>) {
        *self.entries.write().unwrap_or_else(|e| e.into_inner()) = entries;
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.read().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 名单为空时恒 false：调用方据此区分「没配名单」（→ 保持全放行）与
    /// 「配了但这条没命中」（→ 退回公网闸）。
    pub(crate) fn matches(&self, host: &str) -> bool {
        let entries = self.entries.read().unwrap_or_else(|e| e.into_inner());
        entries.iter().any(|entry| entry.matches(host))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miyu_base::config::NetAllowEntry;

    fn entry(raw: &str) -> NetAllowEntry {
        NetAllowEntry::parse(raw).expect("test entry parses")
    }

    #[test]
    fn empty_slot_matches_nothing() {
        let slot = AllowlistSlot::new();
        assert_eq!(slot.len(), 0);
        assert!(!slot.matches("example.com"));
        assert!(!slot.matches("203.0.113.10"));
    }

    #[test]
    fn replace_swaps_the_whole_list() {
        let slot = AllowlistSlot::new();
        slot.replace(vec![entry("example.com"), entry("203.0.113.10")]);
        assert_eq!(slot.len(), 2);
        assert!(slot.matches("example.com"));
        assert!(slot.matches("203.0.113.10"));
        assert!(!slot.matches("other.example.com"));

        // 再次替换是整体覆盖，不是追加。
        slot.replace(vec![entry("another.test")]);
        assert_eq!(slot.len(), 1);
        assert!(!slot.matches("example.com"));
        assert!(slot.matches("another.test"));
    }

    #[test]
    fn matching_is_case_insensitive_and_strips_trailing_dot() {
        let slot = AllowlistSlot::new();
        slot.replace(vec![entry("Example.COM")]);
        assert!(slot.matches("example.com"));
        assert!(slot.matches("example.com."));
        assert!(slot.matches("EXAMPLE.COM"));
    }
}
