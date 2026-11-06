# daemon 监管：systemd unit + guard

这套东西保证「`miyu.service` 一直在跑，且只有一个 daemon 占着家目录」。
2026-11-03 本机发生过一次自持重启循环（48 分钟 16 次启动），根因是两个坑，
两处修法都在这里留档。

## 装法

```sh
sudo install -m 755 miyu-guard /usr/local/bin/miyu-guard
install -Dm 644 miyu.service      ~/.config/systemd/user/miyu.service
install -Dm 644 miyu-guard.service ~/.config/systemd/user/miyu-guard.service
install -Dm 644 miyu-guard.timer   ~/.config/systemd/user/miyu-guard.timer
systemctl --user daemon-reload
systemctl --user enable --now miyu.service miyu-guard.timer
```

## 坑一：让位用退出码 0，监管静默蒸发

daemon 撞到同家目录已有 daemon 时会主动让位。原来那段 `return Ok(())`
——退出码 0。而 unit 是 `Restart=on-failure`，0 不算失败，systemd 就不重启；
unit 静默变 `inactive`，只剩 guard 的 30 秒轮询能把监管捡回来。占位方一死，
中间那一整段是没人管的窗口。

现在让位走 `EX_TEMPFAIL`（75）：占位方在跑不是本次进程的错，但确实没起来，
该让监督者稍后重试。改动在 `crates/miyu-hosts/src/daemon.rs`，
错误类型与码见 `crates/miyu-core/src/ipc/lifecycle.rs`
（`daemon_stand_down_error` / `HomeDaemonStandDown`）与
`src/cli/exit_code.rs`（`EXIT_DAEMON_STAND_DOWN`）。

`miyu daemon stop` 走的是 IPC 关停，退出码仍是 0，不受影响。

## 坑二：guard 把「过渡态」当敌情，还连自己人一起杀

旧版 guard 的判断是「`is-active` 不是 `active` 就 reclaim」，于是
`deactivating` / `activating` 这些 systemd 自己的过渡态也被当成「有野
daemon 占端口」；而 reclaim 用 `pgrep -f "miyu __daemon"` **无差别**杀，
连 systemd 正监管的那个一起 `SIGKILL`。

两条一起，就成了闭环：guard 杀掉监管实例 → 野 daemon 占端口 → systemd
拉起的实例让位（原来是退出码 0，索性连重启都没有）→ 30 秒后 guard 再
捞一次 → 无限转。

现在两条硬规则：

1. `active` 直接退出；`activating` / `deactivating` / `reloading` /
   `refreshing` 先等它自己走完（上限 40 秒），不介入。
2. 只杀 `/proc/<pid>/cgroup` **不含** `miyu.service` 的进程。cgroup 里有
   `miyu.service` 的是 systemd 的合法子民，永远不碰。

顺带：想让服务真的停着不被守卫拉回来，建 `~/.miyu/supervision-paused`。
