# 上游信息与分叉说明

本仓库是 [SHORiN-KiWATA/miyu-agent](https://github.com/SHORiN-KiWATA/miyu-agent)
的分叉，由 `calibura` 维护。**本仓库不再主动跟随上游**：上游的提交不会
自动合并进来，需要时由维护者手动决定是否引入。

## 上游快照

| 项目 | 值 |
| --- | --- |
| 上游仓库 | `https://github.com/SHORiN-KiWATA/miyu-agent` |
| 分叉基点 | `5df19e2` 修复readme链接异常；优化miyu提示词 |
| 分叉日期 | 2026-11-02 |

上面的 commit 是本仓库与上游的最后一个共同祖先（`git merge-base`）。

## git remote 布局

| remote | 地址 | 用途 |
| --- | --- | --- |
| `origin` | `https://github.com/calibura/miyu-agent` | 本仓库的推送目标 |
| `upstream` | `https://github.com/SHORiN-KiWATA/miyu-agent` | 只读参考，push 已禁用 |

`upstream` 的 push URL 被设为 `no_push`，误推会被 git 拒绝。想手动看一眼
上游新提交（**不会**改动工作区）：

```bash
git fetch upstream main
git log --oneline HEAD..upstream/main      # 上游比我们多的
git log --oneline upstream/main..HEAD      # 我们比上游多的
```

## 分支约定

| 分支 | 说明 |
| --- | --- |
| `main` | 与上游同源，不含本地定制。留作对照基线。 |
| `calibura/local` | 实际使用的分支。所有本地定制都在这里。 |

## 本地定制内容

相对上游增加的部分，按主题归类：

### QQ 平台增强

- `feat(platform): support mention_all in response targets` — @全体成员
- `feat(onebot): bot status and group increase notices` — 机器人上下线、入群通知
- `refactor(real_context): reaction cleanup via applied table` — 表情回应去重

### 语音

- `feat(voice): stt_only 纯转写模式` — QQ 语音转文字不必开唤醒和麦克风
- `feat(qq): 语音转写优先用协议端 fetch_ptt_text` — 用 QQ 自己的识别结果，
  本地 SenseVoice 作兜底
- `feat(voice): 接入 GPT-SoVITS 播报供应商` — 本地部署的 GPT-SoVITS 作为
  TTS 后端，支持自定义音色与情绪参考音频

  GPT-SoVITS 与另两家供应商的差别，改代码时要记得：

  - **没有 API key**。自托管，「配好了」的判据是 `base_url` 非空 **且**
    `ref_audio_path` 非空（`GptSovitsTtsConfig::has_key`）。缺参考音频时
    `is_active()` 为假，`send_voice_message` 不会被注册。
  - **协议不同**。`POST {base}/tts`（`api_v2.py`），参数是
    `text / text_lang / ref_audio_path / prompt_text / prompt_lang /
    text_split_method / media_type`，成功时返回**裸 wav 字节**而不是 JSON。
    所以 `gptsovits_extract_audio` 不按 JSON 解析，先看状态码再验
    `RIFF…WAVE` 头，失败时才回退去解析 `{"message": ...}`。
  - **地址不补 `/v1`**。MiniMax / MiMo 会补，GPT-SoVITS 不会，反而会把
    用户多填的 `/v1` 剥掉（`gptsovits_base`）。
  - **超时可配**。`gptsovits.timeout_seconds` 默认 180 秒，因为本地首次
    推理要加载 GPT + SoVITS 权重。另两家的超时是硬编码在
    `voice_tts.rs` 里的（MiMo 90s / MiniMax 60s）。
  - **音色不是 id**。音色由参考音频决定，服务端没有音色注册表，
    `list_gptsovits_voices` 只回当前配的那一条给设置页显示。

### WebUI / TUI

- `feat(web): goal management panel with cross-session overview` — 跨会话目标面板
- `feat(web): daemon-level lightweight todo list` — 轻量待办
- `feat(cli): /goals cross-session goal overview in TUI` — TUI 里的目标总览
- `feat(qq): optional goal progress push after each round` — 每轮后推送目标进度

## 构建

本分叉的构建方式见 `~/.miyu/qq-module/rebuild.sh`：从本仓库切分支、`cargo build
--release`、把产物装到 `/usr/bin/miyu`。注意 `yay -S miyu` 之类的包管理器
升级会用上游版本覆盖 `/usr/bin/miyu`，升级后需要重新用本仓库构建。

**改 `web/` 下任何文件都必须重新编译。** WebUI 的静态资源是通过
`include_str!` 内嵌进二进制的（见 `crates/miyu-hosts/src/web/mod.rs` 的
`INDEX_HTML` / `STYLES_CSS` / `APP_JS` 等常量），不是运行时从磁盘读。
改完 JS 只重启 daemon 不会生效。

前端文案的英文翻译在 `web/i18n-en.js`，CI 会跑
`python3 test_scripts/check-webui-i18n.py` 校验引用与词典的一致性，
提交前本地跑一遍。

## 与上游同步的建议

需要引入上游改动时，用 merge 而不是 rebase，避免重写已推送的历史：

```bash
git fetch upstream main
git switch calibura/local
git merge upstream/main          # 解决冲突后提交
cargo build --release            # 务必编译验证
```
