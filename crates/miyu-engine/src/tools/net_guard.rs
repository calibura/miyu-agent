//! 出站抓取的地址闸门:SSRF 防护三件套。
//!
//! 原先长在 `web_images/download.rs` 里,只有搜图那条线用得上。链接卡片要在
//! Web 服务器里发同样性质的出站请求,而这类逻辑**只能有一份**——安全代码复制
//! 一遍就等于埋一个迟早会漂移的分身。
//!
//! 三层依次是:URL 形状(scheme / userinfo / 特殊主机名)、DNS 解析后的地址
//! (每一个解析结果都必须是公网地址,挡住 DNS rebinding)、以及 IP 段判定
//! (私网、回环、链路本地、CGNAT、文档段、IPv4-mapped IPv6 一并算作非公网)。
//!
//! purge 放行(§purge):`config.purge.bypass_net_guard` 开着时,闸变成「只要
//! 解析出地址就放行」——URL 形状层照走(scheme 仍限 http/https),IP 段判定
//! 与回环/私网拦截退位。旗标是**静态**的:daemon 启动读一次配置写进这里,
//! 改配置要重启才生效。理由:闸在 async 下载热路径上,每次调用回读配置会把
//! 一次 `AppConfig` 克隆拉进每一张图的下载循环;而 purge 是运维级开关,
//! 重启一次的代价可以接受。

use anyhow::{bail, Context, Result};
use reqwest::Url;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

static NET_GUARD_BYPASSED: AtomicBool = AtomicBool::new(false);

/// purge 旗标的唯一写入口:daemon 启动(`web::run`)按配置调一次。
pub fn set_bypassed(bypassed: bool) {
    NET_GUARD_BYPASSED.store(bypassed, Ordering::Release);
}

/// 运行中旗标(任意平台可用):IPC `GetStatus` 把它报给 CLI 的 purge
/// selfcheck,daemon 内它是写入口的镜像读。
pub fn runtime_bypassed() -> bool {
    NET_GUARD_BYPASSED.load(Ordering::Acquire)
}

#[cfg(any(test, feature = "testkit"))]
pub fn bypassed() -> bool {
    runtime_bypassed()
}

/// purge 开着时只走 URL 形状层与「解析得出地址」这一步;公网判定退位。
fn bypass_gate(url: &Url) -> bool {
    NET_GUARD_BYPASSED.load(Ordering::Acquire) && matches!(url.scheme(), "http" | "https")
}

pub(crate) fn is_safe_remote_url(url: &Url) -> bool {
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") {
        // purge 放行:回环/本地主机名也过——IP 段与主机名判定都归公网闸,
        // 闸退位时它们一起退(模块头「purge 放行」)。
        if bypass_gate(url) {
            return true;
        }
        return false;
    }
    match host.parse::<IpAddr>() {
        Ok(ip) => bypass_gate(url) || is_public_ip(ip),
        Err(_) => true,
    }
}

pub async fn resolve_public_remote_target(
    url: &Url,
    timeout: Duration,
) -> Result<Option<(String, Vec<SocketAddr>)>> {
    if !is_safe_remote_url(url) {
        bail!("URL is not a safe public URL")
    }
    let host = url.host_str().context("URL has no host")?;
    if host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<IpAddr>()
        .is_ok()
    {
        return Ok(None);
    }
    let port = url.port_or_known_default().context("URL has no port")?;
    let addresses = tokio::time::timeout(timeout, tokio::net::lookup_host((host, port)))
        .await
        .context("DNS resolution timed out")??
        .collect::<Vec<_>>();
    // purge 放行:解析出地址就过,公网判定退位(见模块头「purge 放行」)。
    if bypass_gate(url) {
        return if addresses.is_empty() {
            bail!("host has no DNS resolution")
        } else {
            Ok(Some((host.to_string(), addresses)))
        };
    }
    if addresses.is_empty() || addresses.iter().any(|address| !is_public_ip(address.ip())) {
        bail!("host resolves to a non-public address")
    }
    Ok(Some((host.to_string(), addresses)))
}

pub(crate) fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [first, second, _, _] = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_unspecified()
                || ip.is_multicast()
                || first == 0
                || (first == 100 && (64..=127).contains(&second))
                || (first == 198 && matches!(second, 18 | 19))
                || first >= 240)
        }
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return is_public_ip(IpAddr::V4(mapped));
            }
            let segments = ip.segments();
            !(ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                || (segments[0] == 0x2001 && segments[1] == 0x0db8))
        }
    }
}

/// net_guard bypass 测试共用的旗标互斥:两个测试都改同一个进程级静态,
/// 并行跑会互相污染。锁本身私有于测试模块,生产路径零开销。
#[cfg(test)]
static FLAG_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    /// purge 放行(§purge):旗标开着时,回环/私网地址的解析照过;旗标关着,
    /// 同一 URL 照旧被拒。旗标是进程级静态,测完还原。
    ///
    /// 用主机名而不是字面 IP:字面 IP 走「无需 DNS」的早退分支
    /// (`Ok(None)`),探不到公网闸这一段。
    #[tokio::test]
    async fn bypass_flag_relaxes_the_public_ip_gate() {
        // 旗标是进程级静态,与另一个 bypass 测试并行跑会互相污染:拿模块级
        // 互斥把「改旗标 → 验证 → 还原」整段圈住。
        let _guard = FLAG_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let url = Url::parse("http://localhost:1/").unwrap();
        set_bypassed(false);
        let denied = resolve_public_remote_target(&url, Duration::from_secs(1)).await;
        assert!(denied.is_err(), "purge off: loopback must be denied");
        set_bypassed(true);
        let allowed = resolve_public_remote_target(&url, Duration::from_secs(1)).await;
        set_bypassed(false); // 先还原再断言。
        let Ok(Some((host, addresses))) = allowed else {
            panic!("purge on: loopback resolves");
        };
        assert_eq!(host, "localhost");
        assert!(!addresses.is_empty());
    }

    /// URL 形状层不随旗标放宽:旗标开着时 scheme 不对、带 userinfo 的仍照拒
    /// ——放行只作用于公网闸,形状层永远生效。
    #[tokio::test]
    async fn bypass_flag_keeps_the_shape_layer() {
        let _guard = FLAG_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        set_bypassed(true);
        for bad in ["ftp://127.0.0.1/", "http://user:pass@127.0.0.1/"] {
            let url = Url::parse(bad).unwrap();
            let result = resolve_public_remote_target(&url, Duration::from_secs(1)).await;
            set_bypassed(false); // 每轮还原,断言语句里别带旗标态。
            assert!(result.is_err(), "{bad} must stay denied under bypass");
        }
    }
}
