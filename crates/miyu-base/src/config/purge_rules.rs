//! purge 的规则页：把工作区里的 `AGENTS.md` / `CLAUDE.md` 之类指令文件
//! 按需拼进注入正文。
//!
//! 移植自 dsh-purge 的 `lib/rules.js`（"CWD 规则"那一块）。那边的形态是
//! 扫描 + 勾选 + 拼装三段；这边保持同一套语义，落到 Rust：
//!
//! - **扫描**：从 cwd 向上走到项目根（以 `.git`/`.hg`/`.svn` 为界），最多 32 层，
//!   每层找四个候选文件名。
//! - **勾选**：只记住路径。**只有仍在当前 cwd 扫描结果里的勾选才生效**——
//!   换个目录后旧勾选不会跨目录偷偷生效，这是 dsh 那边刻意的防穿越设计。
//! - **拼装**：勾选的文件内容按扫描顺序拼接，总长封顶 512KB，每块带一行
//!   `<!-- miyu-purge:cwd-rule <文件名> -->` 头，块间用 `---` 分隔。
//!
//! 与 dsh 的差异：dsh 的规则还能**写回** `AGENTS.md`/`CLAUDE.md`（`RULE_TARGETS`
//! 那套带备份的写入）；miyu 这边第一步只做「读并注入」——写回工作区文件是
//! 一个破坏性动作，等用户明确要了再做，且必须走备份。

use std::path::{Path, PathBuf};

/// 每层查找的候选文件名（与 dsh 的 `CWD_CANDIDATE_FILES` 一致）。
pub const CWD_CANDIDATE_FILES: [&str; 4] = [
    "AGENTS.md",
    "CLAUDE.md",
    "AGENTS.local.md",
    "CLAUDE.local.md",
];

/// 项目根标记。碰到其中之一就不再往上走。
pub const PROJECT_ROOT_MARKERS: [&str; 3] = [".git", ".hg", ".svn"];

/// 向上最多走多少层。防止 cwd 在 `/` 附近时把整条路径都扫一遍。
pub const MAX_CWD_WALK_DEPTH: usize = 32;

/// 拼装后的总长上限。超了就按扫描顺序截断，不报错——注入段太长会挤占上下文，
/// 截断比失败温和。
pub const MAX_LOCAL_INJECT_BYTES: usize = 512 * 1024;

/// 一条扫到的指令文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CwdInstruction {
    /// 绝对路径（规范化后）。
    pub path: PathBuf,
    /// 文件名本身（`AGENTS.md` 等）。
    pub name: String,
    /// 所在目录。
    pub dir: PathBuf,
    /// 这一条相对 cwd 的位置：`cwd` 是本目录，`root` 是项目根，其余 `ancestor`。
    pub scope: CwdScope,
    /// 字节数。
    pub size: u64,
    /// 正文摘要（压空白后取前 160 字符），给 UI 列表用。
    pub preview: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CwdScope {
    /// 就在当前工作目录。
    Cwd,
    /// 项目根（向上走到的最远那层）。
    Root,
    /// 中间的祖先目录。
    Ancestor,
}

/// 从 `cwd` 向上走到项目根，返回由外到内的目录链。
///
/// 找不到项目根标记时，链只有 cwd 自己（与 dsh 的 `ancestorChainForCwd` 同）。
pub fn ancestor_chain(cwd: &Path) -> Vec<PathBuf> {
    let start = normalize(cwd);
    // 先找根：从 cwd 往上，第一个带标记的目录。
    let mut root: Option<PathBuf> = None;
    let mut cursor = Some(start.clone());
    let mut depth = 0;
    while let Some(dir) = cursor {
        if depth >= MAX_CWD_WALK_DEPTH {
            break;
        }
        if PROJECT_ROOT_MARKERS
            .iter()
            .any(|marker| dir.join(marker).exists())
        {
            root = Some(dir.clone());
            break;
        }
        cursor = dir.parent().map(Path::to_path_buf);
        depth += 1;
    }

    // 再从 cwd 往上收集，直到根（没有根就一直收到文件系统顶）。
    let stop = root.unwrap_or_else(|| start.clone());
    let mut chain = Vec::new();
    let mut walker = start;
    for _ in 0..MAX_CWD_WALK_DEPTH {
        let is_stop = walker == stop;
        chain.push(walker.clone());
        if is_stop {
            break;
        }
        match walker.parent() {
            Some(parent) if parent != walker => walker = parent.to_path_buf(),
            _ => break,
        }
    }
    chain.reverse();
    chain
}

/// 扫描 `cwd` 范围内的指令文件，由外到内。
pub fn scan_cwd_instructions(cwd: &Path) -> Vec<CwdInstruction> {
    let chain = ancestor_chain(cwd);
    if chain.is_empty() {
        return Vec::new();
    }
    let leaf = normalize(cwd);
    let root = chain.first().cloned();
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for dir in &chain {
        for name in CWD_CANDIDATE_FILES {
            let path = dir.join(name);
            if !seen.insert(path.clone()) {
                continue;
            }
            let Ok(metadata) = std::fs::metadata(&path) else {
                continue;
            };
            if !metadata.is_file() {
                continue;
            }
            let preview = std::fs::read_to_string(&path)
                .map(|raw| preview_of(&raw))
                .unwrap_or_default();
            let scope = if *dir == leaf {
                CwdScope::Cwd
            } else if Some(dir) == root.as_ref() {
                CwdScope::Root
            } else {
                CwdScope::Ancestor
            };
            out.push(CwdInstruction {
                path,
                name: name.to_string(),
                dir: dir.clone(),
                scope,
                size: metadata.len(),
                preview,
            });
        }
    }
    out
}

/// 压空白 + 截 160 字符。按字符边界截，别把多字节字符劈开。
fn preview_of(raw: &str) -> String {
    let squeezed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = String::new();
    for ch in squeezed.chars() {
        if out.chars().count() >= 160 {
            break;
        }
        out.push(ch);
    }
    out
}

/// 把勾选的文件拼成注入块。
///
/// `selections` 是用户勾过的路径；**只有仍在 `cwd` 扫描结果里的才用**，
/// 避免换目录后旧勾选跨目录生效。返回空串表示没有可注入的内容。
pub fn assemble_selected(cwd: &Path, selections: &[PathBuf]) -> String {
    if selections.is_empty() {
        return String::new();
    }
    let candidates = scan_cwd_instructions(cwd);
    let candidate_set = candidates
        .iter()
        .map(|c| normalize(&c.path))
        .collect::<std::collections::HashSet<_>>();
    let chosen = selections
        .iter()
        .map(|p| normalize(p))
        .filter(|p| candidate_set.contains(p))
        .collect::<Vec<_>>();
    if chosen.is_empty() {
        return String::new();
    }
    let mut parts = Vec::new();
    let mut total = 0usize;
    for path in chosen {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let block = format!("<!-- miyu-purge:cwd-rule {name} -->\n{text}");
        total += block.len();
        if total > MAX_LOCAL_INJECT_BYTES {
            break;
        }
        parts.push(block);
    }
    parts.join("\n\n---\n\n")
}

/// 规范化路径：能 `canonicalize` 就用它，否则原样返回。
///
/// 勾选比对必须两边同一形态，否则 `./AGENTS.md` 和 `/x/AGENTS.md` 对不上。
fn normalize(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn scanning_stops_at_the_project_root_marker() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("proj");
        let nested = root.join("a/b");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        touch(&root, "AGENTS.md", "root rules");
        touch(&nested, "AGENTS.md", "leaf rules");

        let found = scan_cwd_instructions(&nested);
        let names = found
            .iter()
            .map(|c| c.path.file_name().unwrap().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["AGENTS.md", "AGENTS.md"], "根与叶子各一份");
        // 由外到内：第一条是项目根，最后一条是当前目录。
        assert_eq!(found.first().unwrap().scope, CwdScope::Root);
        assert_eq!(found.last().unwrap().scope, CwdScope::Cwd);
    }

    #[test]
    fn without_a_root_marker_only_the_cwd_is_scanned() {
        let temp = tempfile::tempdir().unwrap();
        touch(temp.path(), "AGENTS.md", "only here");
        let found = scan_cwd_instructions(temp.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].scope, CwdScope::Cwd);
    }

    #[test]
    fn all_four_candidate_names_are_recognized() {
        let temp = tempfile::tempdir().unwrap();
        for name in CWD_CANDIDATE_FILES {
            touch(temp.path(), name, "x");
        }
        let found = scan_cwd_instructions(temp.path());
        assert_eq!(found.len(), CWD_CANDIDATE_FILES.len());
    }

    #[test]
    fn preview_is_squeezed_and_truncated() {
        let temp = tempfile::tempdir().unwrap();
        let body = format!("a\n\n\nb   c {}", "d".repeat(400));
        touch(temp.path(), "AGENTS.md", &body);
        let found = scan_cwd_instructions(temp.path());
        let preview = &found[0].preview;
        assert!(preview.starts_with("a b c "), "{preview}");
        assert!(preview.chars().count() <= 160);
    }

    #[test]
    fn selections_outside_the_current_scan_are_ignored() {
        let temp = tempfile::tempdir().unwrap();
        let inside = touch(temp.path(), "AGENTS.md", "inside text");
        let outside_dir = tempfile::tempdir().unwrap();
        let outside = touch(outside_dir.path(), "AGENTS.md", "outside text");

        // 只勾外面那个：不属于当前 cwd 扫描结果，必须被丢掉。
        let only_outside = assemble_selected(temp.path(), &[outside]);
        assert!(only_outside.is_empty(), "跨目录勾选不该生效");

        // 勾里面那个：正常注入，且带文件头。
        let only_inside = assemble_selected(temp.path(), &[inside]);
        assert!(only_inside.contains("inside text"));
        assert!(only_inside.contains("miyu-purge:cwd-rule AGENTS.md"));
    }

    #[test]
    fn multiple_selections_join_with_a_rule() {
        let temp = tempfile::tempdir().unwrap();
        let a = touch(temp.path(), "AGENTS.md", "first");
        let b = touch(temp.path(), "CLAUDE.md", "second");
        let joined = assemble_selected(temp.path(), &[a, b]);
        assert!(joined.contains("first"));
        assert!(joined.contains("second"));
        assert!(joined.contains("\n\n---\n\n"), "块间用 --- 分隔");
    }

    #[test]
    fn empty_or_blank_files_contribute_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let blank = touch(temp.path(), "AGENTS.md", "   \n\n ");
        assert!(assemble_selected(temp.path(), &[blank]).is_empty());
    }
}
