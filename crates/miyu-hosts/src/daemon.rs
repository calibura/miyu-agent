use anyhow::Result;
use miyu_base::paths::MiyuPaths;
use miyu_core::args::WebArgs;

/// Unified background host for IPC, WebUI and configured platform transports.
/// Transport-specific HTTP handlers remain in `web`; lifecycle ownership lives
/// here so future entrypoints do not acquire a second process model.
pub async fn run(paths: MiyuPaths, web: WebArgs) -> Result<()> {
    // 一个家目录只认一个 daemon。闸放在这儿是因为再往下就要开库、抢端口、
    // 拉起语音前端了——让位要赶在占资源之前。
    //
    // 这把锁刻意不在 `runtime_dir()` 里：那个目录名取决于 `MIYU_HOME` 设没
    // 设，同一份数据会算出两把互不可见的锁（09-21 本机就这样跑着两个
    // daemon，一个占 8300、一个回退到临时端口，连带两个 miyu-voice 抢同一个
    // 麦克风）。
    let _singleton = match miyu_core::ipc::acquire_home_singleton(&paths) {
        Ok(lease) => lease,
        Err(busy) => {
            let detail = match &busy.record {
                Some(record) => format!(
                    " pid={} runtime={}",
                    record.pid,
                    record.runtime_dir.display()
                ),
                None => String::new(),
            };
            // 走 println! 是因为 daemon 的 stdout 就是 daemon.log，而拉起我们
            // 的那个 CLI 在启动失败时正是去 tail 它——让用户一眼看见为什么
            // 没起来，而不是对着一个空日志猜。
            println!(
                "{}{detail}",
                miyu_base::i18n::text(
                    "This home directory already has a running Miyu daemon; this process is standing down.",
                    "这个家目录已经有 Miyu daemon 在跑了；本进程让位退出。"
                )
            );
            tracing::warn!(
                home = %paths.root_dir.display(),
                holder_pid = busy.record.as_ref().map(|record| record.pid),
                "another daemon already owns this home directory; standing down"
            );
            // 让位不能是退出码 0：systemd 的 `Restart=on-failure` 只认非零码。
            // 一旦让位按 0 退出，unit 就静默变 inactive，只能靠外部轮询（guard
            // timer）每 30 秒把监管捡回来——占位方一死中间全是没人管的窗口，
            // 而且若占位方本身就是野进程，就永远轮不回来了。EX_TEMPFAIL(75)：
            // 占位方在跑不是本次进程的错，但确实没起来，监督者应当稍后重试。
            return Err(miyu_core::ipc::daemon_stand_down_error(busy));
        }
    };
    crate::web::run(paths, web).await
}
